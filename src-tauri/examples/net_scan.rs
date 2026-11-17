//! Diagnostic program for the network scan and the status check. It sends
//! real pings, so run it on the network you want to look at.
//!
//!   net_scan                    scan the best network, then check a few of the devices found
//!   net_scan 192.168.68.0/22    scan that range instead
//!   net_scan --subnets          only list the networks this PC is on
//!   net_scan --check HOST...    only run a status round on these addresses or names
//!
//! A HOST can be given the hardware address the inventory would hold for it,
//! as in 192.168.68.20=8c:90:2d:a0:0e:02. Two hosts at one address with
//! different hardware addresses show which of them the round takes to be there.
//!
//! Extra switches:
//!   --full            also look up makers and names, as the app's own scan does
//!   --no-check        skip the status round after a scan
//!   --twice           run the status round a second time straight after
//!   --cancel-after N  stop the scan after N seconds, to see what a cancelled scan returns

use home_cc_lib::history::Check;
use home_cc_lib::net::{self, Located, RawFound, ScanPhase, ScanProgress, Target};
use netdev::ipnet::Ipv4Net;
use std::net::{IpAddr, Ipv4Addr};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Options {
    range: String,
    subnets_only: bool,
    check_only: Vec<String>,
    full: bool,
    no_check: bool,
    twice: bool,
    cancel_after: Option<u64>,
}

fn options() -> Result<Options, String> {
    let mut o = Options::default();
    let mut checking = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--subnets" => o.subnets_only = true,
            "--full" => o.full = true,
            "--no-check" => o.no_check = true,
            "--twice" => o.twice = true,
            "--cancel-after" => {
                let seconds = args.next().and_then(|n| n.parse().ok());
                o.cancel_after = Some(seconds.ok_or("--cancel-after needs a number of seconds")?);
            }
            "--check" => checking = true,
            other if other.starts_with('-') => return Err(format!("unknown switch {other}")),
            host if checking => o.check_only.push(host.to_string()),
            range => o.range = range.to_string(),
        }
    }
    if checking && o.check_only.is_empty() {
        return Err("--check needs at least one address or name".into());
    }
    Ok(o)
}

fn main() -> ExitCode {
    let options = match options() {
        Ok(options) => options,
        Err(problem) => {
            eprintln!("{problem}\nSee the top of examples/net_scan.rs for how to use this.");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("could not start: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run(options))
}

async fn run(options: Options) -> ExitCode {
    if !options.check_only.is_empty() {
        let rounds = if options.twice { 2 } else { 1 };
        for _ in 0..rounds {
            check_round(&options.check_only).await;
        }
        return ExitCode::SUCCESS;
    }

    println!("Networks this PC is on, best first:");
    let subnets = tokio::task::spawn_blocking(net::subnets).await.unwrap_or_default();
    for s in &subnets {
        println!(
            "  {:<18} on {:<12} this PC is {:<15} {}",
            s.cidr,
            s.interface,
            s.own_ip,
            if s.best { "(best)" } else { "" }
        );
    }
    if subnets.is_empty() {
        println!("  none");
    }
    if options.subnets_only {
        return ExitCode::SUCCESS;
    }

    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(seconds) = options.cancel_after {
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(seconds));
            cancel.store(true, Ordering::Relaxed);
        });
    }
    let started = Instant::now();
    let progress = progress_printer(started);
    println!(
        "\nScanning {} ...",
        if options.range.is_empty() { "the best network" } else { options.range.as_str() }
    );

    let (range, found) = if options.full {
        match net::scan(&options.range, progress, cancel).await {
            Ok(outcome) => {
                for f in &outcome.found {
                    println!(
                        "  {:<16} {:<18} {:<26} {:<24} {}{}",
                        f.ip,
                        f.mac.as_deref().unwrap_or("(no hardware address)"),
                        f.vendor.as_deref().unwrap_or("-"),
                        f.hostname.as_deref().unwrap_or("-"),
                        reply_text(f.rtt_ms, f.is_self),
                        if f.looks_like_mesh { "  [mesh]" } else { "" },
                    );
                }
                let found: Vec<RawFound> = outcome
                    .found
                    .iter()
                    .filter_map(|f| {
                        Some(RawFound {
                            ip: f.ip.parse().ok()?,
                            mac: f.mac.clone(),
                            rtt_ms: f.rtt_ms,
                            is_self: f.is_self,
                        })
                    })
                    .collect();
                (outcome.range, found)
            }
            Err(e) => {
                println!("The scan failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        match net::scan_raw(&options.range, &progress, &cancel).await {
            Ok(raw) => {
                for f in &raw.found {
                    let mac = f.mac.as_deref().unwrap_or("(no hardware address)");
                    println!("  {:<16} {:<18} {}", f.ip, mac, reply_text(f.rtt_ms, f.is_self));
                }
                println!("Notes:");
                for note in &raw.notes {
                    println!("  {note}");
                }
                (raw.range, raw.found)
            }
            Err(e) => {
                println!("The scan failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    };
    let answered = found.iter().filter(|f| f.rtt_ms.is_some()).count();
    let own = found.iter().filter(|f| f.is_self).count();
    println!(
        "Scan of {range} took {:.1} s: {} devices ({answered} answered ping, {} gave no ping reply, {own} this PC)",
        started.elapsed().as_secs_f64(),
        found.len(),
        found.len() - answered - own,
    );

    if !options.no_check {
        let hosts = hosts_to_check(&range, &found);
        let rounds = if options.twice { 2 } else { 1 };
        for _ in 0..rounds {
            check_round(&hosts).await;
        }
    }
    ExitCode::SUCCESS
}

/// Prints a line when the phase changes and every two seconds in between.
fn progress_printer(started: Instant) -> impl Fn(ScanProgress) + Send + Sync + 'static {
    let last: Mutex<Option<(ScanPhase, Instant)>> = Mutex::new(None);
    move |p: ScanProgress| {
        let mut last = last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let due = match *last {
            Some((phase, at)) => {
                phase != p.phase || at.elapsed() >= Duration::from_secs(2) || p.done == p.total
            }
            None => true,
        };
        if due {
            let seconds = started.elapsed().as_secs_f64();
            println!("  [{seconds:5.1} s] {:?} {}/{}, {} found so far", p.phase, p.done, p.total, p.found);
            *last = Some((p.phase, Instant::now()));
        }
    }
}

fn reply_text(rtt_ms: Option<f64>, is_self: bool) -> String {
    match (rtt_ms, is_self) {
        (_, true) => "this PC".into(),
        (Some(ms), _) => format!("{ms:.2} ms"),
        (None, _) => "no ping reply".into(),
    }
}

/// A handful of what the scan found (the quiet ones first, they are the
/// interesting case), this PC, an address nobody has, and a name that
/// doesn't exist.
fn hosts_to_check(range: &str, found: &[RawFound]) -> Vec<String> {
    let mut picked: Vec<Ipv4Addr> =
        found.iter().filter(|f| f.rtt_ms.is_none() && !f.is_self).map(|f| f.ip).take(3).collect();
    picked.extend(found.iter().filter(|f| f.rtt_ms.is_some()).map(|f| f.ip).take(5));
    picked.extend(found.iter().filter(|f| f.is_self).map(|f| f.ip).take(1));
    if let Ok(range) = range.parse::<Ipv4Net>() {
        let unused = range.hosts().rev().find(|ip| !found.iter().any(|f| f.ip == *ip));
        picked.extend(unused);
    }
    let mut hosts: Vec<String> = picked.iter().map(|ip| ip.to_string()).collect();
    hosts.push("no-such-device.invalid".into());
    hosts
}

/// Each host is "HOST" or "HOST=HARDWARE-ADDRESS".
async fn check_round(hosts: &[String]) {
    let target = |(n, host): (usize, &String)| {
        let (host, mac) = match host.split_once('=') {
            Some((host, mac)) => (host, Some(mac.to_string())),
            None => (host.as_str(), None),
        };
        Target { device_id: n as i64, host: host.to_string(), mac }
    };
    let targets: Vec<Target> = hosts.iter().enumerate().map(target).collect();
    println!("\nStatus round for {} hosts ...", targets.len());
    let started = Instant::now();
    let checks = match net::check(targets.clone()).await {
        Ok(checks) => checks,
        Err(e) => {
            println!("The status round could not run: {e}");
            return;
        }
    };
    let took = started.elapsed();
    let own = net::own_devices();
    for (n, host) in hosts.iter().enumerate() {
        let id = n as i64;
        let line = match checks.iter().find(|c| c.device_id == id) {
            Some(Check { up: true, .. }) if own.contains(&id) => "up, this PC".into(),
            Some(Check { up: true, rtt_ms: Some(ms), .. }) => format!("up, {ms:.2} ms"),
            Some(Check { up: true, rtt_ms: None, .. }) => "up, no ping reply".into(),
            Some(Check { up: false, .. }) => "down".into(),
            None => "NO RESULT (a bug: every host must get one)".into(),
        };
        println!("  {host:<40} {line}");
    }
    let up = checks.iter().filter(|c| c.up).count();
    println!(
        "Round took {:.2} s: {} results, {up} up, {} down, {} this PC",
        took.as_secs_f64(),
        checks.len(),
        checks.len() - up,
        own.len()
    );

    // What a scan would be told about the hosts that are names.
    let started = Instant::now();
    let located = net::locate(targets.clone()).await;
    let names = targets.iter().filter(|t| t.host.parse::<IpAddr>().is_err()).count();
    let took = started.elapsed().as_secs_f64();
    println!("Looking up the {names} names among them took {took:.2} s; {} lead somewhere", located.len());
    for (id, place) in located {
        let host = targets.iter().find(|t| t.device_id == id).map_or("?", |t| t.host.as_str());
        match place {
            Located::At(ip) => println!("  {host:<40} is at {ip}"),
            Located::Own => println!("  {host:<40} is this PC"),
        }
    }
}
