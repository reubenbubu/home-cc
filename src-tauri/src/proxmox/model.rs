//! What Proxmox's API answers, turned into what the screen shows.
//!
//! Everything is read leniently: Proxmox leaves out fields that don't apply
//! (a stopped VM has no uptime, an offline node no CPU), some versions send
//! numbers as text, and anything the token may not see is simply missing.
//! An entry that can't be used is skipped rather than failing the whole list.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::client::clean;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuestKind {
    /// A virtual machine.
    Qemu,
    /// A container.
    Lxc,
}

impl GuestKind {
    /// As it appears in API paths.
    pub fn as_str(self) -> &'static str {
        match self {
            GuestKind::Qemu => "qemu",
            GuestKind::Lxc => "lxc",
        }
    }
}

/// One Proxmox server (a node) with its storage.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub name: String,
    /// "online", "offline" or "unknown".
    pub status: String,
    /// 0 to 100.
    pub cpu_pct: Option<f64>,
    pub cpu_count: Option<u32>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    /// The node's own system disk.
    pub disk_used: Option<u64>,
    pub disk_total: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub storages: Vec<Storage>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Storage {
    pub name: String,
    /// Proxmox's name for the type, such as "dir", "lvmthin", "zfspool" or "cifs".
    pub kind: Option<String>,
    /// What it holds, such as "images", "rootdir", "iso", "backup".
    pub content: Vec<String>,
    pub shared: bool,
    /// False when Proxmox can't reach it or it is switched off.
    pub active: bool,
    pub used: Option<u64>,
    pub total: Option<u64>,
}

/// A VM or a container.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Guest {
    pub vmid: u32,
    pub kind: GuestKind,
    /// Empty if it has none.
    pub name: String,
    pub node: String,
    /// "running", "stopped" or "paused".
    pub status: String,
    /// A template can't be started, only copied.
    pub template: bool,
    /// Set while Proxmox is busy with it, such as "backup", "snapshot" or "migrate".
    pub lock: Option<String>,
    /// 0 to 100, of the processors given to it.
    pub cpu_pct: Option<f64>,
    pub cpu_count: Option<u32>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    /// Containers report what their disk holds; VMs only report its size.
    pub disk_used: Option<u64>,
    pub disk_total: Option<u64>,
    pub uptime_secs: Option<u64>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub nodes: Vec<Node>,
    pub guests: Vec<Guest>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub name: String,
    pub description: String,
    /// Unix seconds.
    pub taken_at: Option<i64>,
    pub parent: Option<String>,
    /// True if the VM's memory was saved too, so a rollback returns it running.
    pub with_ram: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotList {
    /// Oldest first.
    pub snapshots: Vec<Snapshot>,
    /// The snapshot the guest has moved on from: where a rollback would go
    /// without losing others. None if it has no snapshots.
    pub current_parent: Option<String>,
}

/// Where a task Proxmox runs in the background has got to.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskState {
    Running,
    Done,
    /// With Proxmox's reason.
    Failed(String),
}

fn number(v: &Value, key: &str) -> Option<f64> {
    let n = match v.get(key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }?;
    n.is_finite().then_some(n)
}

fn count(v: &Value, key: &str) -> Option<u64> {
    number(v, key).filter(|n| *n >= 0.0).map(|n| n.round() as u64)
}

fn small(v: &Value, key: &str) -> Option<u32> {
    count(v, key).and_then(|n| u32::try_from(n).ok())
}

fn text(v: &Value, key: &str) -> Option<String> {
    let s = match v.get(key)? {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!s.is_empty()).then_some(s)
}

fn flag(v: &Value, key: &str) -> bool {
    match v.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(_) => number(v, key).is_some_and(|n| n != 0.0),
        None => false,
    }
}

/// 0..1 from Proxmox to 0..100.
fn percent(v: &Value, key: &str) -> Option<f64> {
    number(v, key).map(|f| (f * 100.0).clamp(0.0, 100.0))
}

/// A size of 0 means "not known" for totals.
fn total(v: &Value, key: &str) -> Option<u64> {
    count(v, key).filter(|n| *n > 0)
}

fn list(v: &Value, key: &str, separators: &[char]) -> Vec<String> {
    text(v, key)
        .map(|s| s.split(separators).map(str::trim).filter(|t| !t.is_empty()).map(clean).collect())
        .unwrap_or_default()
}

const NOT_A_LIST: &str = "Proxmox answered with something home-cc doesn't understand (a list was expected). Is this a Proxmox VE host?";

/// Reads GET /cluster/resources: every node, VM, container and storage the
/// token may see, in one answer.
pub fn parse_resources(data: &Value) -> Result<Overview, String> {
    let items = data.as_array().ok_or(NOT_A_LIST)?;
    let mut nodes = Vec::new();
    let mut guests = Vec::new();
    let mut storages: Vec<(String, Storage)> = Vec::new();
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("node") => {
                let Some(name) = text(item, "node") else { continue };
                let status = text(item, "status").unwrap_or_else(|| "unknown".into());
                let online = status == "online";
                nodes.push(Node {
                    name: clean(&name),
                    status: clean(&status),
                    // An offline node still lists its sizes, but its use is old news.
                    cpu_pct: online.then(|| percent(item, "cpu")).flatten(),
                    cpu_count: small(item, "maxcpu"),
                    mem_used: online.then(|| count(item, "mem")).flatten(),
                    mem_total: total(item, "maxmem"),
                    disk_used: online.then(|| count(item, "disk")).flatten(),
                    disk_total: total(item, "maxdisk"),
                    uptime_secs: online.then(|| count(item, "uptime")).flatten().filter(|n| *n > 0),
                    storages: Vec::new(),
                });
            }
            Some(kind @ ("qemu" | "lxc")) => {
                let Some(vmid) = small(item, "vmid") else { continue };
                let Some(node) = text(item, "node") else { continue };
                let status = text(item, "status").unwrap_or_else(|| "unknown".into());
                let running = status == "running";
                guests.push(Guest {
                    vmid,
                    kind: if kind == "qemu" { GuestKind::Qemu } else { GuestKind::Lxc },
                    name: text(item, "name").map(|n| clean(&n)).unwrap_or_default(),
                    node: clean(&node),
                    status: clean(&status),
                    template: flag(item, "template"),
                    lock: text(item, "lock").map(|l| clean(&l)),
                    cpu_pct: running.then(|| percent(item, "cpu")).flatten(),
                    cpu_count: small(item, "maxcpu"),
                    mem_used: running.then(|| count(item, "mem")).flatten(),
                    mem_total: total(item, "maxmem"),
                    // A VM's disk use isn't known to Proxmox (it reports 0); a container's is.
                    disk_used: if kind == "lxc" { count(item, "disk").filter(|_| running) } else { None },
                    disk_total: total(item, "maxdisk"),
                    uptime_secs: running.then(|| count(item, "uptime")).flatten().filter(|n| *n > 0),
                    tags: list(item, "tags", &[';', ',', ' ']),
                });
            }
            Some("storage") => {
                let (Some(name), Some(node)) = (text(item, "storage"), text(item, "node")) else { continue };
                let active = text(item, "status").as_deref() == Some("available");
                storages.push((
                    node,
                    Storage {
                        name: clean(&name),
                        kind: text(item, "plugintype").map(|k| clean(&k)),
                        content: list(item, "content", &[',']),
                        shared: flag(item, "shared"),
                        active,
                        used: active.then(|| count(item, "disk")).flatten(),
                        total: active.then(|| total(item, "maxdisk")).flatten(),
                    },
                ));
            }
            // Pools, network zones and anything newer aren't shown.
            _ => {}
        }
    }
    nodes.sort_by(|a, b| a.name.cmp(&b.name));
    for (node, storage) in storages {
        // Storage of a node the token can't see has nowhere to go.
        if let Some(n) = nodes.iter_mut().find(|n| n.name == clean(&node)) {
            n.storages.push(storage);
        }
    }
    for n in &mut nodes {
        n.storages.sort_by(|a, b| a.name.cmp(&b.name));
    }
    guests.sort_by_key(|g| g.vmid);
    Ok(Overview { nodes, guests })
}

/// Reads GET /nodes/{node}/{qemu|lxc}/{vmid}/snapshot. Proxmox lists the
/// guest's present state as a snapshot called "current"; it is left out.
pub fn parse_snapshots(data: &Value) -> Result<SnapshotList, String> {
    let items = data.as_array().ok_or(NOT_A_LIST)?;
    let mut snapshots = Vec::new();
    let mut current_parent = None;
    for item in items {
        let Some(name) = text(item, "name") else { continue };
        if name == "current" {
            current_parent = text(item, "parent").map(|p| clean(&p));
            continue;
        }
        snapshots.push(Snapshot {
            name: clean(&name),
            description: text(item, "description").map(|d| clean(&d)).unwrap_or_default(),
            taken_at: number(item, "snaptime").map(|n| n as i64),
            parent: text(item, "parent").map(|p| clean(&p)),
            with_ram: flag(item, "vmstate"),
        });
    }
    snapshots.sort_by(|a, b| a.taken_at.cmp(&b.taken_at).then_with(|| a.name.cmp(&b.name)));
    Ok(SnapshotList { snapshots, current_parent })
}

/// Reads GET /nodes/{node}/tasks/{upid}/status.
pub fn parse_task(data: &Value) -> TaskState {
    match text(data, "status").as_deref() {
        Some("stopped") => match text(data, "exitstatus") {
            Some(exit) if exit == "OK" || exit.starts_with("WARNINGS") => TaskState::Done,
            Some(exit) => TaskState::Failed(clean(&exit)),
            None => TaskState::Failed("The task ended without saying how.".into()),
        },
        _ => TaskState::Running,
    }
}

/// The task ID a POST that starts a background task answers with, such as
/// "UPID:pve:0001F2A4:…". None if the change was made straight away.
pub fn task_id(data: &Value) -> Option<String> {
    data.as_str().map(str::trim).filter(|s| s.starts_with("UPID:")).map(str::to_string)
}

/// Checks a snapshot name the way Proxmox does: a letter, then letters,
/// digits, "-" or "_", 2 to 40 characters in all.
pub fn check_snapshot_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !(2..=40).contains(&name.len()) || !first_ok || !rest_ok {
        return Err(
            "A snapshot name must start with a letter and hold only letters, digits, - and _ (no spaces), 2 to 40 characters. For example: before_update".into(),
        );
    }
    if name.eq_ignore_ascii_case("current") {
        return Err("\"current\" is the name Proxmox uses for the present state. Pick another name.".into());
    }
    Ok(())
}

/// A node name as Proxmox allows it: letters, digits, "-" and ".".
pub fn check_node_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty() && name.len() <= 63 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    if ok { Ok(()) } else { Err(format!("\"{}\" isn't a Proxmox node name.", clean(name))) }
}

/// A guest's network addresses as Proxmox knows them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestAddresses {
    /// IPv4 addresses, in the order reported, without the guest's own loopback and
    /// link-local ones.
    pub addresses: Vec<String>,
    /// Why Proxmox can't tell, when that is normal (a VM without the QEMU guest agent).
    pub unknown_why: Option<String>,
}

/// One address as a guest reports it, if it is worth showing: IPv4, not 127.x or
/// 169.254.x. A container reports it with its prefix ("192.168.68.31/24").
fn usable_ipv4(text: &str) -> Option<String> {
    let ip: std::net::Ipv4Addr = text.split('/').next()?.trim().parse().ok()?;
    (!ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified()).then(|| ip.to_string())
}

fn push_new(list: &mut Vec<String>, ip: Option<String>) {
    if let Some(ip) = ip.filter(|ip| !list.contains(ip)) {
        list.push(ip);
    }
}

/// A VM's addresses from its QEMU guest agent (agent/network-get-interfaces):
/// {"result": [{"name": "eth0", "ip-addresses": [{"ip-address-type": "ipv4", "ip-address": "…"}]}]}.
pub fn parse_agent_addresses(data: &Value) -> Vec<String> {
    let mut found = Vec::new();
    let interfaces = data.get("result").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    for interface in interfaces {
        let addresses = interface.get("ip-addresses").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        for a in addresses {
            if a.get("ip-address-type").and_then(Value::as_str).is_some_and(|t| t.eq_ignore_ascii_case("ipv4")) {
                push_new(&mut found, a.get("ip-address").and_then(Value::as_str).and_then(usable_ipv4));
            }
        }
    }
    found
}

/// A container's addresses (lxc/<vmid>/interfaces): [{"name": "eth0", "inet": "192.168.68.31/24"}].
pub fn parse_lxc_addresses(data: &Value) -> Vec<String> {
    let mut found = Vec::new();
    for interface in data.as_array().map(Vec::as_slice).unwrap_or_default() {
        push_new(&mut found, interface.get("inet").and_then(Value::as_str).and_then(usable_ipv4));
    }
    found
}

/// True when Proxmox's refusal only says the VM has no guest agent running, which
/// is normal: then it simply can't tell the VM's address.
pub fn no_guest_agent(message: &str) -> bool {
    let m = message.to_lowercase();
    m.contains("guest agent") && (m.contains("not running") || m.contains("not configured") || m.contains("no qemu"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_guest_addresses() {
        let agent = serde_json::json!({"result": [
            {"name": "lo", "ip-addresses": [
                {"ip-address-type": "ipv4", "ip-address": "127.0.0.1", "prefix": 8},
                {"ip-address-type": "ipv6", "ip-address": "::1", "prefix": 128}]},
            {"name": "ens18", "ip-addresses": [
                {"ip-address-type": "ipv4", "ip-address": "192.168.68.35", "prefix": 24},
                {"ip-address-type": "ipv6", "ip-address": "fe80::be24:11ff:fe3a:5510", "prefix": 64}]},
            {"name": "docker0", "ip-addresses": [{"ip-address-type": "ipv4", "ip-address": "172.17.0.1", "prefix": 16}]},
            {"name": "odd", "ip-addresses": [{"ip-address-type": "ipv4", "ip-address": "169.254.3.4"}, {"ip-address": "10.0.0.9"}]},
            {"name": "again", "ip-addresses": [{"ip-address-type": "ipv4", "ip-address": "192.168.68.35"}]}
        ]});
        assert_eq!(parse_agent_addresses(&agent), ["192.168.68.35", "172.17.0.1"]);
        assert!(parse_agent_addresses(&Value::Null).is_empty());

        let lxc = serde_json::json!([
            {"name": "lo", "inet": "127.0.0.1/8", "inet6": "::1/128", "hwaddr": "00:00:00:00:00:00"},
            {"name": "eth0", "inet": "192.168.68.31/24", "inet6": "fe80::be24:11ff:fe3a:5501/64", "hwaddr": "bc:24:11:3a:55:01"},
            {"name": "eth1", "hwaddr": "bc:24:11:3a:55:09"},
            {"name": "bad", "inet": "not an address"}
        ]);
        assert_eq!(parse_lxc_addresses(&lxc), ["192.168.68.31"]);
        assert!(parse_lxc_addresses(&serde_json::json!({"oops": 1})).is_empty());

        assert!(no_guest_agent("QEMU guest agent is not running (HTTP 500)"));
        assert!(no_guest_agent("No QEMU guest agent configured (HTTP 500)"));
        assert!(!no_guest_agent("Permission check failed (/vms/110, VM.Monitor) (HTTP 403)"));
    }

    fn fixture(text: &str) -> Value {
        serde_json::from_str::<Value>(text).unwrap()["data"].clone()
    }

    #[test]
    fn reads_nodes_guests_and_storage() {
        let o = parse_resources(&fixture(include_str!("testdata/cluster_resources.json"))).unwrap();

        assert_eq!(o.nodes.len(), 2);
        let pve = &o.nodes[0];
        assert_eq!(pve.name, "pve");
        assert_eq!(pve.status, "online");
        assert!((pve.cpu_pct.unwrap() - 7.31).abs() < 1e-9);
        assert_eq!(pve.cpu_count, Some(8));
        assert_eq!((pve.mem_used, pve.mem_total), (Some(9468637184), Some(33498034176)));
        assert_eq!((pve.disk_used, pve.disk_total), (Some(12817555456), Some(100861726720)));
        assert_eq!(pve.uptime_secs, Some(1209631));
        // Sorted by name; one that can't be reached keeps its sizes but shows no use; a 0 size is "not known".
        let names: Vec<&str> = pve.storages.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["local", "local-lvm", "nas-backup"]);
        assert_eq!(pve.storages[1].kind.as_deref(), Some("lvmthin"));
        assert_eq!(pve.storages[1].content, ["rootdir", "images"]);
        assert_eq!((pve.storages[1].used, pve.storages[1].total), (Some(53687091200), Some(374538764288)));
        let nas = &pve.storages[2];
        assert!(!nas.active && nas.shared);
        assert_eq!((nas.used, nas.total), (None, None));

        let off = &o.nodes[1];
        assert_eq!(off.status, "offline");
        assert_eq!((off.cpu_pct, off.mem_used, off.uptime_secs), (None, None, None));
        assert_eq!((off.mem_total, off.disk_total), (Some(16717615104), None));
        // Storage of a node that isn't listed ("ghost") is dropped.
        assert!(off.storages.is_empty());

        // By number; the entry without a number is skipped; pools and network zones aren't guests.
        let ids: Vec<u32> = o.guests.iter().map(|g| g.vmid).collect();
        assert_eq!(ids, [100, 101, 102, 200, 9000]);

        let ha = &o.guests[0];
        assert_eq!((ha.kind, ha.name.as_str(), ha.node.as_str(), ha.status.as_str()), (GuestKind::Qemu, "homeassistant", "pve", "running"));
        // Numbers sent as text are read too.
        assert_eq!((ha.mem_used, ha.uptime_secs), (Some(2147483648), Some(86400)));
        assert_eq!(ha.lock.as_deref(), Some("backup"));
        // Proxmox doesn't know what a VM's disk holds.
        assert_eq!((ha.disk_used, ha.disk_total), (None, Some(34359738368)));

        let pihole = &o.guests[1];
        assert_eq!(pihole.kind, GuestKind::Lxc);
        assert_eq!(pihole.tags, ["dns", "infra"]);
        assert_eq!(pihole.disk_used, Some(1306071040));
        assert!((pihole.cpu_pct.unwrap() - 0.49).abs() < 1e-9);

        // A stopped container with no name.
        let unnamed = &o.guests[2];
        assert_eq!((unnamed.name.as_str(), unnamed.status.as_str()), ("", "stopped"));
        assert_eq!((unnamed.cpu_pct, unnamed.mem_used, unnamed.uptime_secs), (None, None, None));

        let stopped = &o.guests[3];
        assert_eq!((stopped.cpu_pct, stopped.mem_used, stopped.uptime_secs), (None, None, None));
        assert!(!stopped.template);
        assert!(o.guests[4].template);
    }

    #[test]
    fn an_empty_answer_is_an_empty_overview() {
        let o = parse_resources(&Value::Array(vec![])).unwrap();
        assert!(o.nodes.is_empty() && o.guests.is_empty());
        assert!(parse_resources(&Value::Null).is_err());
        assert!(parse_resources(&serde_json::json!({"nodes": []})).is_err());
    }

    #[test]
    fn reads_snapshots_without_the_present_state() {
        let list = parse_snapshots(&fixture(include_str!("testdata/snapshots.json"))).unwrap();
        let names: Vec<&str> = list.snapshots.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["after_setup", "before_update"]);
        assert_eq!(list.current_parent.as_deref(), Some("before_update"));
        let first = &list.snapshots[0];
        assert_eq!(first.description, "Fresh install\nwith add-ons");
        assert_eq!(first.taken_at, Some(1731000000));
        assert!(!first.with_ram);
        assert_eq!(list.snapshots[1].parent.as_deref(), Some("after_setup"));
        assert!(list.snapshots[1].with_ram);

        // A guest with no snapshots lists only "current", with no parent.
        let none = parse_snapshots(&serde_json::json!([{"name": "current", "description": "You are here!", "running": 0}])).unwrap();
        assert!(none.snapshots.is_empty());
        assert_eq!(none.current_parent, None);
    }

    #[test]
    fn reads_task_states() {
        assert_eq!(parse_task(&serde_json::json!({"status": "running", "type": "qmstart"})), TaskState::Running);
        assert_eq!(parse_task(&serde_json::json!({"status": "stopped", "exitstatus": "OK"})), TaskState::Done);
        assert_eq!(parse_task(&serde_json::json!({"status": "stopped", "exitstatus": "WARNINGS: 1"})), TaskState::Done);
        assert_eq!(
            parse_task(&serde_json::json!({"status": "stopped", "exitstatus": "VM 100 qmp command 'quit' failed - got timeout"})),
            TaskState::Failed("VM 100 qmp command 'quit' failed - got timeout".into())
        );
        assert!(matches!(parse_task(&serde_json::json!({"status": "stopped"})), TaskState::Failed(_)));
        assert_eq!(task_id(&serde_json::json!("UPID:pve:0001F2A4:0A1B2C3D:6738B2C1:qmstart:100:root@pam!homecc:")).as_deref(), Some("UPID:pve:0001F2A4:0A1B2C3D:6738B2C1:qmstart:100:root@pam!homecc:"));
        assert_eq!(task_id(&Value::Null), None);
    }

    #[test]
    fn snapshot_and_node_names_are_checked() {
        let (longest, too_long) = ("a".repeat(40), "a".repeat(41));
        for good in ["before_update", "b4", "Snap-2026-11-17", longest.as_str()] {
            assert!(check_snapshot_name(good).is_ok(), "{good}");
        }
        for bad in ["", "a", "1st", "with space", "dots.no", "current", "Current", too_long.as_str(), "ünicode"] {
            assert!(check_snapshot_name(bad).is_err(), "{bad}");
        }
        assert!(check_node_name("pve").is_ok());
        assert!(check_node_name("pve-2.lan").is_ok());
        assert!(check_node_name("../pve").is_err());
        assert!(check_node_name("").is_err());
    }
}
