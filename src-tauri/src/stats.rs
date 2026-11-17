//! Machine stats read over SSH: operating system, uptime, CPU, RAM and disks.
//!
//! A small script runs on the machine (stats_linux.sh, or one of the two
//! stats_windows scripts) and prints the same simple format from every system:
//!
//! ```text
//! @@hcc-stats 1
//! key=value            one per line, unknown keys are ignored
//! disk=<total_kb>\t<used_kb>\t<avail_kb>\t<fstype>\t<source>\t<mount>     repeated
//! @@end
//! ```
//! Anything before the first marker (login banners, .bashrc chatter) is skipped.
//! No `@@end` means the script died half-way and the result is thrown away.

use crate::inventory::Os;
use crate::ssh::{self, ExecError, ExecOutput, HostKey, Login, Session, SshError};
use data_encoding::BASE64;
use serde::Serialize;
use std::time::Duration;
use tokio::time::Instant;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    /// Where it is mounted: "/" or "/home" on Linux, "C:" on Windows.
    pub mount: String,
    /// Device name on Linux, volume label on Windows. May be empty.
    pub label: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineStats {
    /// For example "Ubuntu 24.04.1 LTS" or "Microsoft Windows 11 Pro".
    pub os_name: String,
    pub uptime_secs: Option<u64>,
    /// CPU in use, 0 to 100. On Linux this needs two readings, so the first
    /// one has none.
    pub cpu_pct: Option<f64>,
    /// One-minute load average (Linux only).
    pub load1: Option<f64>,
    pub cpu_count: Option<u32>,
    pub mem_total_kb: Option<u64>,
    pub mem_used_kb: Option<u64>,
    /// Real, local disks only.
    pub disks: Vec<Disk>,
    /// True when a Windows account wasn't allowed the full query and some
    /// values may be missing.
    pub reduced: bool,
}

/// CPU counters from one reading, kept so the next one can work out CPU use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuTicks {
    pub total: u64,
    pub idle: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Collected {
    pub stats: MachineStats,
    /// The identity the server presented.
    pub host_key: HostKey,
    /// Pass back in as `prev` next time.
    pub cpu_ticks: Option<CpuTicks>,
}

// The scripts are sent with the same line endings whatever the checkout used. The repo's
// .gitattributes keeps Unix line endings for *.sh only, so `unix_newlines` and
// `compact_powershell` tidy them here in code before anything is sent.

/// Sent on stdin to [`LINUX_COMMAND`].
const LINUX_SCRIPT: &str = include_str!("stats_linux.sh");
/// Works whatever the user's login shell is (bash, zsh, fish, csh): it only has to start `sh`.
const LINUX_COMMAND: &str = "sh -s";
const WINDOWS_SCRIPT: &str = include_str!("stats_windows.ps1");
/// Second try for Windows accounts that are not administrators: inside an SSH session
/// Windows refuses them the full query, so this one uses only .NET and the registry.
const WINDOWS_SCRIPT_NO_WMI: &str = include_str!("stats_windows_nowmi.ps1");

/// One reading, from connecting to logging out, never takes much longer than this.
const TOTAL_LIMIT: Duration = Duration::from_secs(30);
/// The full Windows script gets this long, so that the reduced one still has time to
/// run if the full one hangs.
const WINDOWS_FIRST_LIMIT: Duration = Duration::from_secs(12);
/// More disks than this is not a home machine; it is a script gone wrong.
const MAX_DISKS: usize = 64;
/// Names longer than this are cut, so odd output can't flood the screens.
const MAX_TEXT: usize = 200;

/// Log in to one machine, read its stats and log out. `os` picks the Linux
/// (also Proxmox) or Windows command; other systems are tried as Linux,
/// unless the server introduces itself as Windows' own SSH server.
/// `prev` is the `cpu_ticks` from the previous reading of the same machine.
/// `first_use` is handed the machine's identity on a first connection (no
/// `login.known_key`), once and before anything is offered to the machine, so
/// it can be stored even if the login or the reading then fails; see
/// [`ssh::connect`].
/// Bounded by timeouts: never takes more than about 30 seconds.
pub async fn collect(
    login: &Login,
    os: Os,
    prev: Option<CpuTicks>,
    first_use: &(dyn Fn(&HostKey) + Send + Sync),
) -> Result<Collected, SshError> {
    let deadline = Instant::now() + TOTAL_LIMIT;
    let session = ssh::connect(login, first_use).await?;
    // A machine of unknown type that greets as Windows' own SSH server gets the Windows
    // command straight away: the Linux one could only fail there.
    let windows = match os {
        Os::Windows => true,
        Os::Linux | Os::Proxmox => false,
        Os::Other => session.server_version().contains("OpenSSH_for_Windows"),
    };
    let host = login.host.trim();
    let reading = if windows {
        read_windows(&session, host, deadline).await
    } else {
        read_linux(&session, host, deadline).await.map(|reading| (reading, false))
    };
    let host_key = session.host_key().clone();
    session.close().await;
    let (reading, reduced) = reading?;
    let (stats, cpu_ticks) = summarise(reading, reduced, prev);
    Ok(Collected { stats, host_key, cpu_ticks })
}

/// Time left before the deadline, less a moment to log out. Never under a second, so a
/// slow login still leaves the command a chance.
fn time_left(deadline: Instant) -> Duration {
    let left = deadline.saturating_duration_since(Instant::now());
    left.saturating_sub(Duration::from_secs(1)).max(Duration::from_secs(1))
}

async fn read_linux(session: &Session, host: &str, deadline: Instant) -> Result<Reading, SshError> {
    let script = unix_newlines(LINUX_SCRIPT);
    let output = session.exec(LINUX_COMMAND, Some(script.as_bytes()), time_left(deadline)).await?;
    parse_stats(&output.stdout).map_err(|e| SshError::Other(explain(host, "Linux", &e, &output)))
}

/// The full script first. If it ran but could not finish (which is what a refused query
/// looks like) or hung, the reduced script once. The bool is true when the reduced one
/// was needed.
async fn read_windows(session: &Session, host: &str, deadline: Instant) -> Result<(Reading, bool), SshError> {
    let limit = time_left(deadline).min(WINDOWS_FIRST_LIMIT);
    match session.try_exec(&powershell_command(WINDOWS_SCRIPT), None, limit).await {
        Ok(output) => {
            if let Ok(reading) = parse_stats(&output.stdout) {
                return Ok((reading, false));
            }
        }
        Err(ExecError::TimedOut) => {}
        // The connection itself is in trouble: the other script would fare no better.
        Err(ExecError::Ssh(e)) => return Err(e),
    }
    let output = session.exec(&powershell_command(WINDOWS_SCRIPT_NO_WMI), None, time_left(deadline)).await?;
    match parse_stats(&output.stdout) {
        Ok(reading) => Ok((reading, true)),
        Err(e) => Err(SshError::Other(explain(host, "Windows", &e, &output))),
    }
}

/// A plain sentence for a reading that could not be used.
fn explain(host: &str, system: &str, problem: &StatsError, output: &ExecOutput) -> String {
    let mut text = match problem {
        StatsError::NoStats => format!(
            "{host} did not answer the stats command. If it is not a {system} machine, change its type in \
             the device list."
        ),
        StatsError::Truncated => format!("{host} stopped part-way through reading its stats. Try again."),
        StatsError::UnknownVersion => {
            format!("{host} sent its stats in a form this version of home-cc does not know.")
        }
    };
    // The machine's own complaint often names the cause ("sh: awk: not found").
    let said =
        clean(String::from_utf8_lossy(&output.stderr).lines().find(|l| !l.trim().is_empty()).unwrap_or(""));
    if !said.is_empty() {
        let said: String = said.chars().take(120).collect();
        text += &format!(" It said: \"{said}\".");
    }
    text
}

/// `sh` reads a stray carriage return as part of the command, so make sure none is sent
/// even if this file was checked out with Windows line endings.
fn unix_newlines(script: &str) -> String {
    script.replace("\r\n", "\n")
}

/// The script as it is sent: the same bytes whatever line endings the checkout used,
/// with comment lines and blank lines left out to keep the command short.
fn compact_powershell(script: &str) -> String {
    let lines: Vec<&str> = script
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty() && !line.trim_start().starts_with('#'))
        .collect();
    lines.join("\n")
}

/// Wrap a PowerShell script so it survives cmd.exe (the default shell of Windows' SSH
/// server) or any other shell: base64 of the UTF-16 text has no character that needs
/// quoting. cmd.exe refuses command lines over 8191 characters, so scripts must stay small.
fn powershell_command(script: &str) -> String {
    let utf16le: Vec<u8> = compact_powershell(script).encode_utf16().flat_map(u16::to_le_bytes).collect();
    format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -InputFormat None -OutputFormat Text \
         -EncodedCommand {}",
        BASE64.encode(&utf16le)
    )
}

/// Everything one run of a script reported, before it is boiled down for the screens.
#[derive(Debug, Clone, PartialEq, Default)]
struct Reading {
    os: Option<String>,
    /// First line of `pveversion` when the machine is a Proxmox host.
    pve: Option<String>,
    uptime_s: Option<u64>,
    /// 1, 5 and 15 minute load averages (Linux only).
    load: Option<[f64; 3]>,
    ncpu: Option<u32>,
    /// Running totals since boot (Linux). CPU use comes from two readings.
    cpu_ticks: Option<CpuTicks>,
    /// A ready-made CPU percentage (Windows).
    cpu_pct: Option<f64>,
    mem: Option<Mem>,
    disks: Vec<RawDisk>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mem {
    total_kb: u64,
    /// Linux: MemAvailable. Windows: free physical memory.
    avail_kb: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawDisk {
    total_kb: u64,
    used_kb: u64,
    avail_kb: u64,
    /// Linux: the device ("/dev/sda2", "rpool/ROOT/pve-1"). Windows: the volume label.
    source: String,
    /// Linux: the mount point. Windows: the drive ("C:").
    mount: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StatsError {
    /// No start marker: the script never ran.
    NoStats,
    /// A start marker for a format newer than this code.
    UnknownVersion,
    /// No end marker: the script died half-way.
    Truncated,
}

const BEGIN: &str = "@@hcc-stats ";
const END: &str = "@@end";

/// Parse what a stats script printed. The text comes from another machine, so nothing
/// in it is trusted: a line that makes no sense is skipped, never an error or a crash.
/// Takes bytes because Windows may send text that is not valid UTF-8.
fn parse_stats(stdout: &[u8]) -> Result<Reading, StatsError> {
    let text = String::from_utf8_lossy(stdout);
    // Windows adds carriage returns if anything on the way does, and may put a
    // byte-order mark first.
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r').trim_start_matches('\u{feff}'));

    let version = lines
        .by_ref()
        .find_map(|l| l.trim().strip_prefix(BEGIN).map(str::trim))
        .ok_or(StatsError::NoStats)?;
    if version != "1" {
        return Err(StatsError::UnknownVersion);
    }

    let mut reading = Reading::default();
    let mut complete = false;
    for line in lines {
        if line.trim() == END {
            complete = true;
            break;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim_end();
        match key {
            "os" => reading.os = Some(clean(value)).filter(|v| !v.is_empty()),
            "pve" => reading.pve = Some(clean(value)).filter(|v| !v.is_empty()),
            "uptime_s" => reading.uptime_s = value.trim().parse().ok(),
            "ncpu" => reading.ncpu = value.trim().parse().ok().filter(|n| *n > 0),
            "cpu_pct" => reading.cpu_pct = number(value.trim()).map(|p| p.min(100.0)),
            "load" => {
                let parts: Vec<Option<f64>> = value.split_whitespace().map(number).collect();
                if let [Some(a), Some(b), Some(c)] = parts.as_slice() {
                    reading.load = Some([*a, *b, *c]);
                }
            }
            "cpu_ticks" => {
                reading.cpu_ticks = two_numbers(value).map(|(total, idle)| CpuTicks { total, idle })
            }
            "mem_kb" => {
                reading.mem = two_numbers(value)
                    .filter(|(total, _)| *total > 0)
                    .map(|(total_kb, avail_kb)| Mem { total_kb, avail_kb })
            }
            "disk" => {
                if reading.disks.len() < MAX_DISKS {
                    reading.disks.extend(disk(value));
                }
            }
            // The scripts send more than the screens show yet (kernel, swap, ...), and a
            // newer script may send more still.
            _ => {}
        }
    }
    if !complete {
        return Err(StatsError::Truncated);
    }
    Ok(reading)
}

/// One `disk=` line: six tab-separated fields. The mount point is last so that it may
/// contain anything but a tab.
fn disk(value: &str) -> Option<RawDisk> {
    let fields: Vec<&str> = value.splitn(6, '\t').collect();
    let [total, used, avail, _fstype, source, mount] = fields.as_slice() else { return None };
    let mount = clean(mount);
    if mount.is_empty() {
        return None;
    }
    Some(RawDisk {
        total_kb: total.parse().ok().filter(|t| *t > 0)?,
        used_kb: used.parse().ok()?,
        avail_kb: avail.parse().ok()?,
        source: clean(source),
        mount,
    })
}

/// A plain number, zero or more. Text like "nan" and "inf" also parses as a number in
/// Rust, and must not reach the screens.
fn number(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.0)
}

fn two_numbers(value: &str) -> Option<(u64, u64)> {
    let mut it = value.split_whitespace().map(str::parse::<u64>);
    match (it.next(), it.next(), it.next()) {
        (Some(Ok(a)), Some(Ok(b)), None) => Some((a, b)),
        _ => None,
    }
}

/// Text from the machine, made safe to show: no control characters, not too long.
fn clean(text: &str) -> String {
    text.trim().chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect()
}

/// CPU in use between two readings of the same Linux machine, 0 to 100. None if no time
/// passed or the counters went backwards (the machine restarted in between).
fn cpu_busy_pct(earlier: CpuTicks, later: CpuTicks) -> Option<f64> {
    let total = later.total.checked_sub(earlier.total).filter(|t| *t > 0)?;
    let idle = later.idle.checked_sub(earlier.idle)?;
    let pct = 100.0 * total.saturating_sub(idle) as f64 / total as f64;
    // One decimal is as much as the counters are good for.
    Some(((pct * 10.0).round() / 10.0).clamp(0.0, 100.0))
}

/// Boil a reading down to what the screens show, and hand back the CPU counters for
/// the next reading.
fn summarise(reading: Reading, reduced: bool, prev: Option<CpuTicks>) -> (MachineStats, Option<CpuTicks>) {
    let cpu_pct = reading.cpu_pct.or_else(|| cpu_busy_pct(prev?, reading.cpu_ticks?));
    let disks = reading
        .disks
        .into_iter()
        .map(|d| {
            // "Used" is everything that is not free to use, so that used / total matches
            // the percentage `df` shows. Linux keeps some space back for the system; a
            // disk with none left for the owner should read as full.
            let used_kb =
                if d.avail_kb <= d.total_kb { d.total_kb - d.avail_kb } else { d.used_kb.min(d.total_kb) };
            Disk {
                mount: d.mount,
                label: d.source,
                total_bytes: d.total_kb.saturating_mul(1024),
                used_bytes: used_kb.saturating_mul(1024),
            }
        })
        .collect();
    let stats = MachineStats {
        os_name: os_name(reading.os, reading.pve.as_deref()),
        uptime_secs: reading.uptime_s,
        cpu_pct,
        load1: reading.load.map(|[one, _, _]| one),
        cpu_count: reading.ncpu,
        mem_total_kb: reading.mem.map(|m| m.total_kb),
        mem_used_kb: reading.mem.map(|m| m.total_kb.saturating_sub(m.avail_kb)),
        disks,
        reduced,
    };
    (stats, reading.cpu_ticks)
}

/// A Proxmox host calls itself Debian; show it as Proxmox, with its version.
fn os_name(os: Option<String>, pve: Option<&str>) -> String {
    // The line looks like "pve-manager/8.3.0/c1689ccb1065a83b (running kernel: 6.8.12-4-pve)".
    let version = pve
        .and_then(|line| line.strip_prefix("pve-manager/"))
        .and_then(|rest| rest.split('/').next())
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 20
                && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-~+".contains(c))
        });
    match (version, os) {
        (Some(version), _) => format!("Proxmox VE {version}"),
        (None, Some(os)) => os,
        (None, None) => "Unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINUX: &str = "Welcome to Debian! (noise from a login script)\n\
@@hcc-stats 1\n\
os=Debian GNU/Linux 12 (bookworm)\n\
os_id=debian\n\
os_version=12\n\
kernel=6.8.12-4-pve\n\
arch=x86_64\n\
host=pve\n\
pve=pve-manager/8.3.0/c1689ccb1065a83b (running kernel: 6.8.12-4-pve)\n\
uptime_s=811292\n\
load=2.73 2.50 2.49\n\
cpu_ticks=1611993611 1339701423\n\
ncpu=20\n\
mem_kb=51335112 38035504\n\
swap_kb=20777976 9721348\n\
zfs_arc_kb=4194304\n\
some_future_key=whatever = with equals\n\
disk=202943772\t179882632\t14296928\text4\t/dev/mapper/ubuntu--vg-ubuntu--lv\t/\n\
disk=1098628\t6452\t1092176\tvfat\t/dev/sda1\t/boot/efi\n\
disk=1000\t400\t600\text4\t/dev/sdb1\t/media/jim/My Disk = big\n\
disk=broken\tline\n\
@@end\n\
trailing noise\n";

    const WINDOWS: &str = "\u{feff}@@hcc-stats 1\r\n\
os=Microsoft Windows 11 Pro\r\n\
os_id=windows\r\n\
os_version=10.0.26100\r\n\
kernel=26100\r\n\
arch=AMD64\r\n\
host=MAIN-PC\r\n\
uptime_s=93784\r\n\
ncpu=16\r\n\
cpu_pct=7\r\n\
mem_kb=33472512 21048576\r\n\
swap_kb=4980736 4900000\r\n\
disk=975823868\t512000000\t463823868\tNTFS\tWindows\tC:\r\n\
disk=1953512444\t100\t1953512344\tNTFS\t\tD:\r\n\
@@end\r\n";

    #[test]
    fn parses_linux_output() {
        let r = parse_stats(LINUX.as_bytes()).unwrap();
        assert_eq!(r.os.as_deref(), Some("Debian GNU/Linux 12 (bookworm)"));
        assert!(r.pve.as_deref().unwrap().starts_with("pve-manager/8.3.0"));
        assert_eq!(r.uptime_s, Some(811292));
        assert_eq!(r.load, Some([2.73, 2.50, 2.49]));
        assert_eq!(r.ncpu, Some(20));
        assert_eq!(r.cpu_ticks, Some(CpuTicks { total: 1611993611, idle: 1339701423 }));
        assert_eq!(r.cpu_pct, None);
        assert_eq!(r.mem, Some(Mem { total_kb: 51335112, avail_kb: 38035504 }));
        assert_eq!(r.disks.len(), 3, "the broken disk line is dropped, the rest kept");
        assert_eq!(r.disks[0].mount, "/");
        assert_eq!(r.disks[0].source, "/dev/mapper/ubuntu--vg-ubuntu--lv");
        assert_eq!(r.disks[2].mount, "/media/jim/My Disk = big");
    }

    #[test]
    fn parses_windows_output_with_crlf_and_bom() {
        let r = parse_stats(WINDOWS.as_bytes()).unwrap();
        assert_eq!(r.os.as_deref(), Some("Microsoft Windows 11 Pro"));
        assert_eq!(r.cpu_pct, Some(7.0));
        assert_eq!(r.cpu_ticks, None);
        assert_eq!(r.load, None);
        assert_eq!(r.ncpu, Some(16));
        assert_eq!(r.disks.len(), 2);
        assert_eq!(r.disks[0].mount, "C:");
        assert_eq!(r.disks[0].source, "Windows");
        assert_eq!(r.disks[1].source, "", "a drive with no label");
        assert_eq!(r.disks[1].mount, "D:");
    }

    #[test]
    fn rejects_empty_truncated_and_future_output() {
        assert_eq!(parse_stats(b""), Err(StatsError::NoStats));
        assert_eq!(parse_stats(b"sh: awk: not found\n"), Err(StatsError::NoStats));
        assert_eq!(parse_stats(b"@@hcc-stats 1\nos=Debian\nuptime_s=5\n"), Err(StatsError::Truncated));
        assert_eq!(parse_stats(b"@@hcc-stats 2\n@@end\n"), Err(StatsError::UnknownVersion));
        assert_eq!(
            parse_stats(b"@@end\n@@hcc-stats 1\n"),
            Err(StatsError::Truncated),
            "an end before the start"
        );
    }

    #[test]
    fn bad_values_become_none_not_errors() {
        let out = "@@hcc-stats 1\nuptime_s=soon\nload=1.0 2.0\ncpu_ticks=5\nmem_kb=0 0\nncpu=0\n\
         cpu_pct=250\nos=\n@@end\n";
        let r = parse_stats(out.as_bytes()).unwrap();
        assert_eq!(r.uptime_s, None);
        assert_eq!(r.load, None);
        assert_eq!(r.cpu_ticks, None);
        assert_eq!(r.mem, None);
        assert_eq!(r.ncpu, None);
        assert_eq!(r.cpu_pct, Some(100.0));
        assert_eq!(r.os, None);
    }

    #[test]
    fn survives_bytes_that_are_not_utf8() {
        let mut out = b"@@hcc-stats 1\nos=Windows \xff\xfe\nuptime_s=7\n@@end\n".to_vec();
        out.extend_from_slice(b"\xff");
        let r = parse_stats(&out).unwrap();
        assert_eq!(r.uptime_s, Some(7));
        assert!(r.os.unwrap().starts_with("Windows"));
    }

    #[test]
    fn hostile_numbers_are_dropped() {
        let out = "@@hcc-stats 1\n\
uptime_s=-5\n\
ncpu=99999999999999999999\n\
cpu_pct=NaN\n\
load=inf 1 1\n\
cpu_ticks=18446744073709551616 1\n\
mem_kb=1e9 5\n\
disk=-1\t0\t0\text4\t/dev/a\t/a\n\
disk=1.5\t0\t0\text4\t/dev/b\t/b\n\
disk=0\t0\t0\text4\t/dev/c\t/c\n\
disk=10\t0\t5\text4\t/dev/d\t\n\
disk=10\t0\t5\text4\t/dev/e\n\
disk=\n\
=\n\
===\n\
@@end\n";
        let r = parse_stats(out.as_bytes()).unwrap();
        assert_eq!(r, Reading::default(), "every line is nonsense, so nothing is kept");

        let negative = parse_stats(b"@@hcc-stats 1\ncpu_pct=-3\nload=-1 0 0\n@@end\n").unwrap();
        assert_eq!((negative.cpu_pct, negative.load), (None, None));
    }

    #[test]
    fn hostile_text_is_cleaned_and_capped() {
        let long = "x".repeat(5000);
        let mut out = format!("@@hcc-stats 1\nos=Evil\x1b[31m OS\x07 {long}\n");
        for i in 0..500 {
            out += &format!("disk=100\t50\t50\text4\t/dev/sd{i}\t/mnt/\x1b[2J{i}\n");
        }
        out += "@@end\n";
        let r = parse_stats(out.as_bytes()).unwrap();
        let os = r.os.unwrap();
        assert!(os.starts_with("Evil[31m OS "), "control characters are removed: {os:?}");
        assert_eq!(os.chars().count(), MAX_TEXT);
        assert_eq!(r.disks.len(), MAX_DISKS);
        assert_eq!(r.disks[3].mount, "/mnt/[2J3");
    }

    #[test]
    fn arbitrary_bytes_never_panic() {
        // A small fixed-seed generator: the same "random" inputs on every run.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let pieces: [&[u8]; 14] = [
            b"@@hcc-stats 1\n",
            b"@@end\n",
            b"disk=",
            b"\t",
            b"\n",
            b"\r\n",
            b"=",
            b"load=",
            b"cpu_ticks=",
            b"mem_kb=",
            b"18446744073709551615",
            b" ",
            b"\xff\xfe\x00",
            b"os=",
        ];
        for _ in 0..2000 {
            let mut input = Vec::new();
            for _ in 0..(next() % 40) {
                match next() % 3 {
                    0 => input.extend_from_slice(pieces[(next() % 14) as usize]),
                    1 => input.push((next() % 256) as u8),
                    _ => input.extend_from_slice(format!("{}", next() % 1000).as_bytes()),
                }
            }
            if let Ok(reading) = parse_stats(&input) {
                // Whatever came out must also survive the arithmetic.
                let _ = summarise(reading, false, Some(CpuTicks { total: u64::MAX, idle: 0 }));
            }
        }
    }

    #[test]
    fn cpu_busy_between_two_readings() {
        let a = CpuTicks { total: 1000, idle: 900 };
        let b = CpuTicks { total: 2000, idle: 1650 };
        assert_eq!(cpu_busy_pct(a, b), Some(25.0));
        assert_eq!(cpu_busy_pct(a, a), None, "no time passed");
        assert_eq!(cpu_busy_pct(b, a), None, "counters went backwards: restart");
        let idle_back = CpuTicks { total: 3000, idle: 100 };
        assert_eq!(cpu_busy_pct(b, idle_back), None, "idle went backwards on its own");
        // Idle can tick a little ahead of the total between two reads; never over 100 or under 0.
        let odd = CpuTicks { total: 2010, idle: 1700 };
        assert_eq!(cpu_busy_pct(b, odd), Some(0.0));
        let busy = CpuTicks { total: 3000, idle: 1650 };
        assert_eq!(cpu_busy_pct(b, busy), Some(100.0));
        let huge = CpuTicks { total: u64::MAX, idle: u64::MAX / 2 };
        assert!(
            cpu_busy_pct(CpuTicks { total: 0, idle: 0 }, huge).is_some_and(|p| (49.0..=51.0).contains(&p))
        );
    }

    #[test]
    fn linux_cpu_needs_two_readings() {
        let reading = parse_stats(LINUX.as_bytes()).unwrap();
        let (first, ticks) = summarise(reading.clone(), false, None);
        assert_eq!(first.cpu_pct, None, "the first reading has nothing to compare with");
        assert_eq!(ticks, Some(CpuTicks { total: 1611993611, idle: 1339701423 }));

        let prev = CpuTicks { total: 1611993611 - 2000, idle: 1339701423 - 1500 };
        let (second, _) = summarise(reading.clone(), false, Some(prev));
        assert_eq!(second.cpu_pct, Some(25.0));
        let prev = CpuTicks { total: 1611993611 - 3000, idle: 1339701423 - 2000 };
        let (rounded, _) = summarise(reading.clone(), false, Some(prev));
        assert_eq!(rounded.cpu_pct, Some(33.3), "one decimal place");

        let after_restart = CpuTicks { total: 1611993611 + 1, idle: 5 };
        let (third, ticks) = summarise(reading, false, Some(after_restart));
        assert_eq!(third.cpu_pct, None, "counters lower than last time: the machine restarted");
        assert!(ticks.is_some(), "the new counters are still handed back for next time");
    }

    #[test]
    fn linux_reading_becomes_stats() {
        let (stats, _) = summarise(parse_stats(LINUX.as_bytes()).unwrap(), false, None);
        assert_eq!(stats.os_name, "Proxmox VE 8.3.0");
        assert_eq!(stats.uptime_secs, Some(811292));
        assert_eq!(stats.load1, Some(2.73));
        assert_eq!(stats.cpu_count, Some(20));
        assert_eq!(stats.mem_total_kb, Some(51335112));
        assert_eq!(stats.mem_used_kb, Some(51335112 - 38035504));
        assert!(!stats.reduced);
        assert_eq!(stats.disks.len(), 3);
        let root = &stats.disks[0];
        assert_eq!((root.mount.as_str(), root.label.as_str()), ("/", "/dev/mapper/ubuntu--vg-ubuntu--lv"));
        assert_eq!(root.total_bytes, 202943772 * 1024);
        // df shows 93% for this disk; used / total must say the same.
        assert_eq!(root.used_bytes, (202943772 - 14296928) * 1024);
        assert_eq!((100.0 * root.used_bytes as f64 / root.total_bytes as f64).round(), 93.0);
    }

    #[test]
    fn windows_reading_becomes_stats() {
        let prev = Some(CpuTicks { total: 1, idle: 1 });
        let (stats, ticks) = summarise(parse_stats(WINDOWS.as_bytes()).unwrap(), true, prev);
        assert_eq!(stats.os_name, "Microsoft Windows 11 Pro");
        assert_eq!(stats.cpu_pct, Some(7.0), "Windows gives the percentage itself");
        assert_eq!(ticks, None);
        assert_eq!(stats.load1, None);
        assert_eq!(stats.mem_used_kb, Some(33472512 - 21048576));
        assert!(stats.reduced);
        let c = &stats.disks[0];
        assert_eq!((c.mount.as_str(), c.label.as_str()), ("C:", "Windows"));
        assert_eq!(c.used_bytes, 512000000 * 1024);
        assert_eq!(stats.disks[1].label, "");
    }

    #[test]
    fn odd_sizes_do_not_overflow() {
        let out = format!(
            "@@hcc-stats 1\nmem_kb=100 500\ndisk={max}\t{max}\t0\text4\t/dev/a\t/a\n\
             disk=100\t30\t900\text4\t/dev/b\t/b\n@@end\n",
            max = u64::MAX
        );
        let (stats, _) = summarise(parse_stats(out.as_bytes()).unwrap(), false, None);
        assert_eq!(stats.mem_used_kb, Some(0), "more free than there is: shown as none used");
        assert_eq!(stats.disks[0].total_bytes, u64::MAX);
        assert_eq!(stats.disks[0].used_bytes, u64::MAX);
        assert_eq!(
            stats.disks[1].used_bytes,
            30 * 1024,
            "free space larger than the disk: fall back to used"
        );
        assert_eq!(stats.os_name, "Unknown");
    }

    #[test]
    fn proxmox_name_only_from_a_well_formed_line() {
        let debian = || Some("Debian GNU/Linux 12 (bookworm)".to_string());
        assert_eq!(
            os_name(debian(), Some("pve-manager/8.3.0/c1689ccb (running kernel: 6.8)")),
            "Proxmox VE 8.3.0"
        );
        assert_eq!(os_name(debian(), Some("pve-manager/9.0.0~beta1/abc")), "Proxmox VE 9.0.0~beta1");
        assert_eq!(os_name(debian(), Some("something else")), "Debian GNU/Linux 12 (bookworm)");
        assert_eq!(os_name(debian(), Some("pve-manager/<b>x</b>/abc")), "Debian GNU/Linux 12 (bookworm)");
        assert_eq!(os_name(debian(), Some("pve-manager//abc")), "Debian GNU/Linux 12 (bookworm)");
        assert_eq!(os_name(debian(), None), "Debian GNU/Linux 12 (bookworm)");
    }

    fn decode_powershell(command: &str) -> String {
        let b64 = command.rsplit(' ').next().unwrap();
        let bytes = BASE64.decode(b64.as_bytes()).unwrap();
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn powershell_command_is_utf16le_base64() {
        let command = powershell_command("echo 'hé'");
        assert!(command.starts_with("powershell.exe "));
        assert!(command.contains(" -NonInteractive "));
        assert!(command.contains(" -EncodedCommand "));
        assert_eq!(decode_powershell(&command), "echo 'hé'");
        let b64 = command.rsplit(' ').next().unwrap();
        assert!(
            b64.chars().all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c)),
            "nothing a shell would touch"
        );
    }

    #[test]
    fn powershell_scripts_fit_cmd_exe() {
        // cmd.exe refuses command lines over 8191 characters; leave room for what the
        // SSH server puts in front.
        for script in [WINDOWS_SCRIPT, WINDOWS_SCRIPT_NO_WMI] {
            let command = powershell_command(script);
            assert!(command.len() < 8000, "{} characters", command.len());
            let sent = decode_powershell(&command);
            assert!(sent.starts_with("$ErrorActionPreference = 'Stop'\n"), "comments are not sent");
            assert!(sent.ends_with("$s.Flush()"));
            assert!(!sent.contains('\r') && !sent.contains("\n\n") && !sent.contains("\n#"));
            assert!(sent.contains("@@hcc-stats 1") && sent.contains("@@end"));
        }
    }

    #[test]
    fn scripts_are_sent_the_same_whatever_the_checkout_line_endings() {
        for script in [WINDOWS_SCRIPT, WINDOWS_SCRIPT_NO_WMI] {
            let unix = script.replace("\r\n", "\n");
            let windows = unix.replace('\n', "\r\n");
            assert_ne!(unix, windows);
            assert_eq!(powershell_command(&unix), powershell_command(&windows));
        }
        let unix = LINUX_SCRIPT.replace("\r\n", "\n");
        let windows = unix.replace('\n', "\r\n");
        assert_eq!(unix_newlines(&windows), unix);
        assert!(!unix_newlines(LINUX_SCRIPT).contains('\r'));
        assert!(unix.starts_with("# home-cc machine stats, Linux."));
    }

    #[test]
    fn compacting_keeps_code_and_drops_comments() {
        let script = "# header\r\n\r\n$a = 1   \r\n  # indented comment\r\n  $b = '#not a comment'\r\n";
        assert_eq!(compact_powershell(script), "$a = 1\n  $b = '#not a comment'");
    }

    #[test]
    fn failures_are_explained_in_plain_words() {
        let quiet = ExecOutput::default();
        let none = explain("nas", "Linux", &StatsError::NoStats, &quiet);
        assert_eq!(
            none,
            "nas did not answer the stats command. If it is not a Linux machine, change its type in the \
             device list."
        );
        let noisy = ExecOutput {
            stderr:
                b"\n'sh' is not recognized as an internal or external command,\r\noperable program\x1b[0m"
                    .to_vec(),
            ..ExecOutput::default()
        };
        let said = explain("pc", "Linux", &StatsError::NoStats, &noisy);
        assert!(
            said.ends_with("It said: \"'sh' is not recognized as an internal or external command,\"."),
            "{said}"
        );
        let cut = explain("pc", "Windows", &StatsError::Truncated, &quiet);
        assert_eq!(cut, "pc stopped part-way through reading its stats. Try again.");
    }

    /// Tauri commands run on several threads. Checked by the compiler; nothing runs.
    #[test]
    fn collect_can_cross_threads() {
        fn sendable<T: Send>(_: &T) {}
        let login = Login {
            host: String::new(),
            port: 22,
            user: String::new(),
            known_key: None,
            key_path: None,
            key_passphrase: None,
            password: None,
        };
        sendable(&collect(&login, Os::Linux, None, &|_| {}));
    }

    /// The identity reaches the caller on a first connection even when the login then
    /// fails, so it can be stored whatever happens next.
    #[tokio::test]
    async fn first_connection_identity_is_handed_over_before_the_login() {
        use crate::ssh::test_server;
        let server = test_server::start(&[russh::MethodKind::Password]).await;
        let seen = std::sync::Mutex::new(Vec::new());
        let first_use = |key: &HostKey| seen.lock().unwrap().push((key.clone(), server.attempts().len()));
        let result = collect(&server.login(), Os::Linux, None, &first_use).await;
        assert!(matches!(result, Err(SshError::AuthFailed(_))), "{result:?}");
        assert_eq!(*seen.lock().unwrap(), [(server.host_key.clone(), 0)]);

        // A machine whose identity is stored is not a first connection.
        let mut known = server.login();
        known.known_key = Some(server.host_key.clone());
        let result = collect(&known, Os::Linux, None, &first_use).await;
        assert!(matches!(result, Err(SshError::AuthFailed(_))), "{result:?}");
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn stats_serialise_with_the_names_the_screens_expect() {
        let (stats, _) = summarise(parse_stats(WINDOWS.as_bytes()).unwrap(), false, None);
        let json = serde_json::to_value(&stats).unwrap();
        for key in [
            "osName",
            "uptimeSecs",
            "cpuPct",
            "load1",
            "cpuCount",
            "memTotalKb",
            "memUsedKb",
            "disks",
            "reduced",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
        let disk = &json["disks"][0];
        for key in ["mount", "label", "totalBytes", "usedBytes"] {
            assert!(disk.get(key).is_some(), "missing {key}");
        }
        assert!(json["load1"].is_null());
    }
}
