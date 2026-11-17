//! Diagnostic: shows the maker and name the app would find for devices this
//! PC already knows about. It asks each device once and scans nothing.
//!
//!   ip -4 neigh | net_ident                      (reads the neighbour list)
//!   net_ident 192.168.68.1=8c:90:2d:a0:0e:02 192.168.68.20

use home_cc_lib::net::{names, oui};
use std::io::Read;
use std::net::Ipv4Addr;
use std::time::Instant;

/// One line of `ip neigh` ("192.168.68.1 dev eth0 lladdr 8c:90:... REACHABLE")
/// or one argument ("192.168.68.1=8c:90:..." or just the address).
fn parse(line: &str) -> Option<(Ipv4Addr, Option<String>)> {
    let mut words = line.split_whitespace();
    let first = words.next()?;
    if let Some((ip, mac)) = first.split_once('=') {
        return Some((ip.parse().ok()?, Some(mac.to_string())));
    }
    let ip = first.parse().ok()?;
    let rest: Vec<&str> = words.collect();
    let mac = rest.iter().position(|word| *word == "lladdr").and_then(|at| rest.get(at + 1)).map(|mac| mac.to_string());
    // Neighbour lines without a hardware address are addresses nobody answered for.
    (rest.is_empty() || mac.is_some()).then_some((ip, mac))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut devices: Vec<(Ipv4Addr, Option<String>)> = if args.is_empty() {
        let mut text = String::new();
        let _ = std::io::stdin().read_to_string(&mut text);
        text.lines().filter_map(parse).collect()
    } else {
        args.iter().filter_map(|arg| parse(arg)).collect()
    };
    devices.sort();
    devices.dedup_by_key(|(ip, _)| *ip);
    if devices.is_empty() {
        eprintln!("No addresses given. Try: ip -4 neigh | net_ident");
        return;
    }

    let ips: Vec<Ipv4Addr> = devices.iter().map(|(ip, _)| *ip).collect();
    let started = Instant::now();
    let found = names::discover(&ips).await;
    let took = started.elapsed();

    let (mut with_maker, mut with_name, mut mesh) = (0, 0, 0);
    println!("{:<16} {:<18} {:<42} name", "address", "hardware address", "maker");
    for (ip, mac) in &devices {
        let maker = mac.as_deref().and_then(oui::vendor);
        let maker_text = match (&maker, mac.as_deref()) {
            (Some(maker), _) if oui::looks_like_mesh(maker) => format!("{maker} [mesh?]"),
            (Some(maker), _) => maker.clone(),
            (None, Some(mac)) if oui::is_randomised(mac) => "(private address)".to_string(),
            _ => "-".to_string(),
        };
        with_maker += usize::from(maker.is_some());
        mesh += usize::from(maker.as_deref().is_some_and(oui::looks_like_mesh));
        with_name += usize::from(found.contains_key(ip));
        let name = found.get(ip).map(String::as_str).unwrap_or("-");
        println!("{:<16} {:<18} {:<42} {name}", ip.to_string(), mac.as_deref().unwrap_or("-"), maker_text);
    }
    println!(
        "\n{} devices: {with_maker} with a maker ({mesh} look like mesh nodes), {with_name} with a name. Names took {took:.2?}.",
        devices.len()
    );
}
