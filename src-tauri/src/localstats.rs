//! This PC's own stats, read without SSH.
//!
//! home-cc runs on one of the machines it watches. That one needs no SSH
//! user, login or identity check: the same script that stats.rs sends over
//! SSH runs here as an ordinary program (`sh` on Linux, PowerShell on
//! Windows, with no window), and what it prints goes through the same
//! parser, so the numbers are the ones an SSH reading would give. The same
//! limits apply too: 30 seconds for the whole reading, 12 for the full
//! Windows script before the one without WMI is tried.

use crate::error::Result;
use crate::inventory::{Device, DeviceKind, Inventory};
use crate::net;
use crate::stats::{self, CpuTicks, MachineStats, Reading, StatsError};
use std::collections::{BTreeSet, HashSet};
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// More output than this from a stats script is a script gone wrong. The rest is
/// read and thrown away, so the script isn't left stuck writing.
const MAX_OUTPUT: usize = 1 << 20;
/// How often a running script is looked at to see whether it has finished.
const POLL: Duration = Duration::from_millis(20);

// --- Which device is this PC ---

/// The devices in `devices` that are this PC (never one set to Ignore). Worked
/// out afresh on each call, so a new address from the router (DHCP) counts as
/// soon as this PC has it. Usually one device, sometimes none, and more than
/// one if the owner added this PC twice (say once by address and once as
/// "localhost"). Blocks for a moment on Windows while the network adapters
/// are listed.
pub fn this_pc_ids(devices: &[Device]) -> BTreeSet<i64> {
    let own = net::own_ips_now();
    let hostname = dns_lookup::get_hostname().ok();
    // The same verdict the status checks give the screens ("this PC").
    let by_checks = net::own_devices();
    devices
        .iter()
        .filter(|d| d.kind != DeviceKind::Ignore)
        .filter(|d| is_this_pc(&d.host, &own, hostname.as_deref(), by_checks.contains(&d.id)))
        .map(|d| d.id)
        .collect()
}

/// Whether one device is this PC, off the async threads.
pub async fn is_this_pc_now(device: &Device) -> bool {
    let (device, id) = (device.clone(), device.id);
    tauri::async_runtime::spawn_blocking(move || this_pc_ids(std::slice::from_ref(&device)))
        .await
        .is_ok_and(|ids| ids.contains(&id))
}

/// `this_pc_ids` for a whole list, off the async threads.
pub async fn this_pc_ids_now(devices: Vec<Device>) -> BTreeSet<i64> {
    tauri::async_runtime::spawn_blocking(move || this_pc_ids(&devices)).await.unwrap_or_default()
}

/// A device's host (as the owner typed it, or a scan stored it) is this PC when it is
/// - one of this PC's own addresses (`own`), or a loopback one such as 127.0.0.1 or ::1;
/// - "localhost", or this PC's own name (`hostname`), with or without a network ending
///   such as ".local" or ".lan", in any mix of capitals;
/// - any other name that the latest round of status checks found leads to this PC
///   (`by_checks`; the round looks names up, this function doesn't).
///
/// An address is judged by itself alone, never by `by_checks`: that verdict can be from
/// before the owner pointed the device at another machine.
fn is_this_pc(host: &str, own: &HashSet<Ipv4Addr>, hostname: Option<&str>, by_checks: bool) -> bool {
    let host = host.trim().trim_end_matches('.');
    match host.trim_start_matches('[').trim_end_matches(']').parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || own.contains(&ip),
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback() || ip.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback() || own.contains(&v4))
        }
        Err(_) => {
            let host = host.to_ascii_lowercase();
            let first_part = |name: &str| name.split('.').next().unwrap_or("").to_string();
            let own_name = hostname.map(|n| n.trim().trim_end_matches('.').to_ascii_lowercase()).unwrap_or_default();
            host == "localhost"
                || host.ends_with(".localhost")
                || (!own_name.is_empty() && (host == own_name || first_part(&host) == first_part(&own_name)))
                || by_checks
        }
    }
}

/// The system home-cc runs on, as the device list stores it.
fn own_os_word() -> Option<&'static str> {
    if cfg!(windows) {
        Some("windows")
    } else if cfg!(target_os = "linux") {
        Some("linux")
    } else {
        None
    }
}

/// A device that is this PC and whose system is still "Other" gets the system home-cc
/// runs on (Windows or Linux). One the owner picked is never changed. True if any
/// device changed.
pub fn fill_in_os(inv: &Inventory, this_pc: &BTreeSet<i64>) -> Result<bool> {
    let Some(word) = own_os_word() else { return Ok(false) };
    let conn = inv.conn();
    let mut changed = 0;
    for id in this_pc {
        changed +=
            conn.execute("UPDATE devices SET os = ?1 WHERE id = ?2 AND os = 'other'", rusqlite::params![word, id])?;
    }
    Ok(changed > 0)
}

// --- Reading the stats ---

/// Read this PC's stats, off the async threads. `prev` is the `cpu_ticks` of the
/// previous local reading, as for [`stats::collect`]. The error is a sentence for the
/// owner. Never takes much more than 30 seconds.
pub async fn collect(prev: Option<CpuTicks>) -> std::result::Result<(MachineStats, Option<CpuTicks>), String> {
    tauri::async_runtime::spawn_blocking(move || collect_here(prev))
        .await
        .unwrap_or_else(|e| Err(format!("Reading this PC's stats stopped unexpectedly: {e}")))
}

/// [`collect`], on the calling thread.
pub fn collect_here(prev: Option<CpuTicks>) -> std::result::Result<(MachineStats, Option<CpuTicks>), String> {
    let deadline = Instant::now() + stats::TOTAL_LIMIT;
    let (reading, reduced) = read_here(deadline)?;
    Ok(stats::summarise(reading, reduced, prev))
}

#[cfg(target_os = "linux")]
fn read_here(deadline: Instant) -> std::result::Result<(Reading, bool), String> {
    read_with_sh("/bin/sh", deadline)
}

/// The full script first, then, if it ran but could not finish or hung, the one without
/// WMI, as over SSH. Run as the person using the PC, the full one normally works.
#[cfg(windows)]
fn read_here(deadline: Instant) -> std::result::Result<(Reading, bool), String> {
    let program = powershell();
    let first = time_left(deadline).min(stats::WINDOWS_FIRST_LIMIT);
    match run(&program, &powershell_args(stats::WINDOWS_SCRIPT)[..], None, first) {
        Ok(output) => {
            if let Ok(reading) = stats::parse_stats(&output.stdout) {
                return Ok((reading, false));
            }
        }
        Err(RunError::TimedOut) => {}
        // PowerShell can't be started at all: the other script would fare no better.
        Err(e) => return Err(e.explain("PowerShell")),
    }
    let output = run(&program, &powershell_args(stats::WINDOWS_SCRIPT_NO_WMI)[..], None, time_left(deadline))
        .map_err(|e| e.explain("PowerShell"))?;
    stats::parse_stats(&output.stdout).map(|reading| (reading, true)).map_err(|e| explain(&e, &output))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn read_here(_deadline: Instant) -> std::result::Result<(Reading, bool), String> {
    Err("home-cc can read this PC's own stats on Windows and Linux only.".into())
}

/// The Linux script, given to `shell` on its input as `sh -s` gets it over SSH.
#[cfg(target_os = "linux")]
fn read_with_sh(shell: &str, deadline: Instant) -> std::result::Result<(Reading, bool), String> {
    let script = stats::unix_newlines(stats::LINUX_SCRIPT);
    let output = run(shell, &["-s"], Some(script.as_bytes()), time_left(deadline)).map_err(|e| e.explain("sh"))?;
    stats::parse_stats(&output.stdout).map(|reading| (reading, false)).map_err(|e| explain(&e, &output))
}

/// PowerShell by its full path, so a program of the same name in some folder on the
/// PATH can't stand in for it.
#[cfg(windows)]
fn powershell() -> String {
    std::env::var("SystemRoot")
        .ok()
        .map(|root| format!(r"{root}\System32\WindowsPowerShell\v1.0\powershell.exe"))
        .filter(|path| std::path::Path::new(path).is_file())
        .unwrap_or_else(|| "powershell.exe".into())
}

/// The same switches as over SSH (see stats.rs): no profile, no questions, the script
/// as base64 so nothing in it needs quoting.
#[cfg_attr(not(windows), allow(dead_code))]
fn powershell_args(script: &str) -> Vec<String> {
    let mut args: Vec<String> =
        ["-NoLogo", "-NoProfile", "-NonInteractive", "-InputFormat", "None", "-OutputFormat", "Text", "-EncodedCommand"]
            .into_iter()
            .map(String::from)
            .collect();
    args.push(stats::encoded_powershell(script));
    args
}

/// Time left before the deadline, but never under a second, so a slow first try still
/// leaves the next one a chance.
fn time_left(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now()).max(Duration::from_secs(1))
}

/// A plain sentence for output that could not be used, with the script's own complaint
/// if it made one.
fn explain(problem: &StatsError, output: &Output) -> String {
    let mut text = match problem {
        StatsError::NoStats => "The stats script did not run on this PC.".to_string(),
        StatsError::Truncated => "The stats script stopped part-way through on this PC. Try again.".to_string(),
        StatsError::UnknownVersion => {
            "The stats script on this PC answered in a form this version of home-cc does not know.".to_string()
        }
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    let said = stats::clean(stderr.lines().find(|l| !l.trim().is_empty()).unwrap_or(""));
    if !said.is_empty() {
        let said: String = said.chars().take(120).collect();
        text += &format!(" It said: \"{said}\".");
    }
    text
}

// --- Running a program here ---

/// What a program printed.
#[derive(Debug, Default)]
pub struct Output {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug)]
pub enum RunError {
    /// The program could not be started, or the system lost track of it.
    Failed(std::io::Error),
    /// It was still running when its time was up, and was stopped.
    TimedOut,
}

impl RunError {
    fn explain(&self, program: &str) -> String {
        match self {
            RunError::Failed(e) => format!("Could not run {program} on this PC to read its stats: {e}."),
            RunError::TimedOut => {
                "Reading this PC's stats took too long and was stopped. It is tried again with the next reading."
                    .to_string()
            }
        }
    }
}

/// Run `program` with `args` on this PC, hand it `input` (or nothing) and collect what it
/// prints. Blocks until it ends, and stops it if it is still running after `limit`. On
/// Windows it gets no console window, so nothing flashes up on the screen.
///
/// Stopping it stops that program only. Anything it started itself may run on for a
/// moment; the stats scripts start only short commands that end by themselves.
pub fn run(
    program: &str,
    args: &[impl AsRef<str>],
    input: Option<&[u8]>,
    limit: Duration,
) -> std::result::Result<Output, RunError> {
    let deadline = Instant::now() + limit;
    let mut command = Command::new(program);
    command
        .args(args.iter().map(AsRef::as_ref))
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_window(&mut command);
    let mut child = command.spawn().map_err(RunError::Failed)?;

    if let (Some(mut pipe), Some(input)) = (child.stdin.take(), input) {
        let input = input.to_vec();
        // On its own, so a program that doesn't read its input can't hold this up. The
        // pipe closes when the thread ends, which tells the program there is no more.
        thread::spawn(move || {
            pipe.write_all(&input).ok();
        });
    }
    let stdout = read_in_background(child.stdout.take());
    let stderr = read_in_background(child.stderr.take());

    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
            outcome => {
                child.kill().ok();
                child.wait().ok();
                return Err(match outcome {
                    Err(e) => RunError::Failed(e),
                    _ => RunError::TimedOut,
                });
            }
        }
    }
    // It has ended, so its output is complete as soon as the pipes are read to the end.
    // Something it left running could keep a pipe open: don't wait for that for ever.
    let wait = deadline.saturating_duration_since(Instant::now()).max(Duration::from_secs(1));
    Ok(Output {
        stdout: stdout.recv_timeout(wait).unwrap_or_default(),
        stderr: stderr.recv_timeout(Duration::from_millis(500)).unwrap_or_default(),
    })
}

/// Everything from a pipe until it closes, up to `MAX_OUTPUT`, read on a thread of its own.
fn read_in_background(pipe: Option<impl Read + Send + 'static>) -> mpsc::Receiver<Vec<u8>> {
    let (send, receive) = mpsc::channel();
    match pipe {
        Some(mut pipe) => {
            thread::spawn(move || {
                let mut kept = Vec::new();
                let mut buf = [0u8; 8192];
                loop {
                    match pipe.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            let room = MAX_OUTPUT.saturating_sub(kept.len());
                            kept.extend_from_slice(&buf[..n.min(room)]);
                        }
                        Err(e) if e.kind() == ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                send.send(kept).ok();
            });
        }
        None => {
            send.send(Vec::new()).ok();
        }
    }
    receive
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    /// Win32's CREATE_NO_WINDOW. PowerShell is a console program: without this a
    /// console window would flash up on every reading.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{DeviceInput, Os};

    const HOME: [u8; 4] = [192, 168, 68, 58];

    fn own() -> HashSet<Ipv4Addr> {
        HashSet::from([Ipv4Addr::from(HOME), Ipv4Addr::new(172, 17, 0, 1)])
    }

    #[test]
    fn this_pc_by_its_own_address() {
        let own = own();
        assert!(is_this_pc("192.168.68.58", &own, None, false));
        assert!(is_this_pc(" 192.168.68.58 ", &own, None, false), "spaces around it");
        assert!(is_this_pc("172.17.0.1", &own, None, false), "any of its adapters");
        assert!(!is_this_pc("192.168.68.54", &own, None, false), "another machine");
        assert!(!is_this_pc("192.168.68.54", &own, None, true), "an address is judged by itself");
        // After the router hands this PC a new address, the old one is someone else's.
        let moved = HashSet::from([Ipv4Addr::new(192, 168, 68, 77)]);
        assert!(!is_this_pc("192.168.68.58", &moved, None, false));
        assert!(is_this_pc("192.168.68.77", &moved, None, false));
        // No network at all.
        assert!(!is_this_pc("192.168.68.58", &HashSet::new(), None, false));
    }

    #[test]
    fn this_pc_by_loopback_and_localhost() {
        let none = HashSet::new();
        let hosts = ["127.0.0.1", "127.0.1.1", "::1", "[::1]", "::ffff:127.0.0.1", "localhost", "LOCALHOST", "localhost."];
        for host in hosts.into_iter().chain(["app.localhost"]) {
            assert!(is_this_pc(host, &none, None, false), "{host}");
        }
        assert!(is_this_pc("::ffff:192.168.68.58", &own(), None, false));
        assert!(!is_this_pc("::2", &none, None, false));
        assert!(!is_this_pc("notlocalhost", &none, None, false));
    }

    #[test]
    fn this_pc_by_its_own_name() {
        let none = HashSet::new();
        let name = Some("reuben-pc");
        for host in ["reuben-pc", "REUBEN-PC", "reuben-pc.local", "Reuben-PC.lan", "reuben-pc."] {
            assert!(is_this_pc(host, &none, name, false), "{host}");
        }
        assert!(is_this_pc("reuben-pc", &none, Some("REUBEN-PC.home.arpa"), false), "a full name from the system");
        assert!(!is_this_pc("nas", &none, name, false));
        assert!(!is_this_pc("reuben-pc2", &none, name, false));
        assert!(!is_this_pc("nas", &none, Some(""), false), "no name known");
        assert!(!is_this_pc("nas", &none, None, false));
        // A name that the status round found leads here.
        assert!(is_this_pc("desktop.example", &none, name, true));
    }

    fn device(id: i64, host: &str, kind: DeviceKind) -> Device {
        Device {
            id,
            name: format!("device {id}"),
            host: host.into(),
            kind,
            os: Os::Other,
            ssh_user: None,
            ssh_port: 22,
            notes: String::new(),
            mac: None,
            vendor: None,
            hostname: None,
            source: crate::inventory::Source::Manual,
            reviewed: true,
            first_seen: 0,
            last_seen: None,
        }
    }

    #[test]
    fn this_pc_among_the_devices_on_this_very_pc() {
        let hostname = dns_lookup::get_hostname().unwrap();
        let devices = [
            device(1, "127.0.0.1", DeviceKind::Pc),
            device(2, "192.0.2.55", DeviceKind::Pc),
            device(3, "localhost", DeviceKind::Ignore),
            device(4, &hostname, DeviceKind::Other),
        ];
        assert_eq!(this_pc_ids(&devices), BTreeSet::from([1, 4]), "an ignored device is never read");
        assert!(this_pc_ids(&devices[1..2]).is_empty(), "none at all");
        assert!(this_pc_ids(&[]).is_empty());
        // Every address this PC has right now counts.
        if let Some(ip) = net::own_ips_now().into_iter().next() {
            assert_eq!(this_pc_ids(&[device(9, &ip.to_string(), DeviceKind::Pc)]), BTreeSet::from([9]));
        }
    }

    #[test]
    fn os_is_filled_in_only_where_still_other() {
        let inv = Inventory::in_memory().unwrap();
        let add = |name: &str, os: Os| {
            inv.add(DeviceInput {
                name: name.into(),
                host: format!("{name}.lan"),
                kind: DeviceKind::Pc,
                os,
                ssh_user: None,
                ssh_port: None,
                notes: String::new(),
                mac: None,
            })
            .unwrap()
            .id
        };
        let (unknown, chosen, other_pc) = (add("this", Os::Other), add("set", Os::Proxmox), add("else", Os::Other));
        let this_pc = BTreeSet::from([unknown, chosen]);
        assert!(fill_in_os(&inv, &this_pc).unwrap());
        let expected = if cfg!(windows) { Os::Windows } else { Os::Linux };
        assert_eq!(inv.get(unknown).unwrap().os, expected);
        assert_eq!(inv.get(chosen).unwrap().os, Os::Proxmox, "the owner's choice stays");
        assert_eq!(inv.get(other_pc).unwrap().os, Os::Other, "not this PC");
        assert!(!fill_in_os(&inv, &this_pc).unwrap(), "nothing left to fill in");
        assert!(!fill_in_os(&inv, &BTreeSet::new()).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn runs_a_program_with_input_and_collects_its_output() {
        let out = run("/bin/sh", &["-c", "cat; echo oops >&2"], Some(b"hello\n"), Duration::from_secs(10)).unwrap();
        assert_eq!(out.stdout, b"hello\n");
        assert_eq!(out.stderr, b"oops\n");
        let out = run("/bin/sh", &["-c", "echo no input"], None, Duration::from_secs(10)).unwrap();
        assert_eq!(out.stdout, b"no input\n");
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_runs_too_long_is_stopped() {
        let started = Instant::now();
        let outcome = run("/bin/sh", &["-c", "exec sleep 30"], None, Duration::from_millis(300));
        assert!(matches!(outcome, Err(RunError::TimedOut)), "{outcome:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        assert!(RunError::TimedOut.explain("sh").contains("took too long"));
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_ignores_its_input_does_not_hold_things_up() {
        let big = vec![b'x'; 4 * MAX_OUTPUT];
        let out = run("/bin/sh", &["-c", "echo done"], Some(&big), Duration::from_secs(10)).unwrap();
        assert_eq!(out.stdout, b"done\n");
    }

    #[cfg(unix)]
    #[test]
    fn too_much_output_is_cut() {
        let out = run("/bin/sh", &["-c", "head -c 3000000 /dev/zero"], None, Duration::from_secs(20)).unwrap();
        assert_eq!(out.stdout.len(), MAX_OUTPUT);
    }

    #[test]
    fn a_missing_program_is_explained() {
        match run("/no/such/program-home-cc", &[] as &[&str], None, Duration::from_secs(5)) {
            Err(e @ RunError::Failed(_)) => {
                assert!(e.explain("PowerShell").starts_with("Could not run PowerShell on this PC to read its stats: "))
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unusable_output_is_explained_in_plain_words() {
        let output = Output { stdout: b"@@hcc-stats 1\nos=x\n".to_vec(), stderr: b"\n sh: awk: not found\n".to_vec() };
        let e = stats::parse_stats(&output.stdout).unwrap_err();
        assert_eq!(
            explain(&e, &output),
            "The stats script stopped part-way through on this PC. Try again. It said: \"sh: awk: not found\"."
        );
        let quiet = Output::default();
        assert_eq!(explain(&StatsError::NoStats, &quiet), "The stats script did not run on this PC.");
    }

    #[test]
    fn powershell_gets_the_same_switches_as_over_ssh() {
        let args = powershell_args(stats::WINDOWS_SCRIPT);
        let switches =
            ["-NoLogo", "-NoProfile", "-NonInteractive", "-InputFormat", "None", "-OutputFormat", "Text", "-EncodedCommand"];
        assert_eq!(args[..8], switches);
        assert_eq!(args.len(), 9);
        assert!(args[8].chars().all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c)));
    }

    /// The real Linux script, run here, read by the real parser.
    #[cfg(target_os = "linux")]
    #[test]
    fn reads_this_linux_pc() {
        let (first, ticks) = collect_here(None).unwrap();
        assert!(!first.os_name.is_empty() && first.os_name != "Unknown", "{first:?}");
        assert!(first.mem_total_kb.is_some_and(|kb| kb > 0), "{first:?}");
        assert!(first.mem_used_kb.is_some());
        assert!(first.cpu_count.is_some_and(|n| n > 0), "{first:?}");
        assert!(first.uptime_secs.is_some() && first.load1.is_some());
        assert_eq!(first.cpu_pct, None, "CPU use needs two readings on Linux");
        assert!(!first.reduced);
        let ticks = ticks.expect("CPU counters");
        // Some time has to pass for the counters to move.
        thread::sleep(Duration::from_millis(300));
        let (second, _) = collect_here(Some(ticks)).unwrap();
        let cpu = second.cpu_pct.expect("CPU use from the second reading");
        assert!((0.0..=100.0).contains(&cpu));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_shell_that_is_not_there_is_explained() {
        let e = read_with_sh("/no/such/sh", Instant::now() + Duration::from_secs(5)).unwrap_err();
        assert!(e.starts_with("Could not run sh on this PC to read its stats: "), "{e}");
    }
}
