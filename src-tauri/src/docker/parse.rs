//! Reading what the docker command line prints. Everything here is plain text in,
//! plain values out, so it is tested with samples of real output and never needs a
//! Docker host.
//!
//! What the machine sends is treated as untrusted: odd lines are skipped, long text
//! is cut, and nothing in it is ever run.

use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

/// Text from the machine is cut to this many characters, so odd output can't flood
/// the screens.
const MAX_TEXT: usize = 300;
/// More containers than this on one home machine is a listing gone wrong.
const MAX_CONTAINERS: usize = 2000;

/// One container, as the Docker screen shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    /// The full id (64 hex characters).
    pub id: String,
    /// Without the leading "/" Docker keeps on names.
    pub name: String,
    pub image: String,
    /// "running", "exited", "paused", "restarting", "created", "removing" or "dead".
    pub state: String,
    /// "healthy", "unhealthy" or "starting", for containers that have a health check.
    pub health: Option<String>,
    /// How the last run ended, for a container that is not running.
    pub exit_code: Option<i64>,
    /// Unix seconds of the last start. None if it never started.
    pub started_at: Option<i64>,
    /// Unix seconds of the last stop. None if it never stopped.
    pub finished_at: Option<i64>,
    pub ports: Vec<Port>,
    /// The compose project it belongs to, if any.
    pub project: Option<String>,
    /// The service name in the compose file.
    pub service: Option<String>,
    /// Folder the stack was started from (compose's project folder).
    #[serde(skip)]
    pub working_dir: Option<String>,
    /// Compose files the stack was started with.
    #[serde(skip)]
    pub config_files: Vec<String>,
}

/// A port the container listens on, published on the host or not.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Port {
    /// The port number on the host, if it is published.
    pub host_port: Option<u16>,
    /// Only set when published on one address rather than all of them.
    pub host_ip: Option<String>,
    pub container_port: u16,
    /// "tcp", "udp" or "sctp".
    pub protocol: String,
}

/// CPU and memory of one running container, from `docker stats --no-stream`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerStats {
    pub id: String,
    /// Share of one CPU core, so it can go above 100 on a machine with several.
    pub cpu_pct: Option<f64>,
    pub mem_used_bytes: Option<u64>,
    pub mem_limit_bytes: Option<u64>,
}

/// The Go template handed to `docker inspect`: only what the screen needs, as one line
/// of JSON per container. `index` is used for Health because a container without a
/// health check has no such key, and Docker's templates stop at a missing key.
pub const INSPECT_FORMAT: &str = concat!(
    r#"{"id":{{json .Id}},"name":{{json .Name}},"image":{{json .Config.Image}},"#,
    r#""state":{{json .State.Status}},"startedAt":{{json .State.StartedAt}},"#,
    r#""finishedAt":{{json .State.FinishedAt}},"exitCode":{{json .State.ExitCode}},"#,
    r#""health":{{with index .State "Health"}}{{json .Status}}{{else}}null{{end}},"#,
    r#""labels":{{json .Config.Labels}},"ports":{{json .NetworkSettings.Ports}}}"#
);

const LABEL_PROJECT: &str = "com.docker.compose.project";
const LABEL_SERVICE: &str = "com.docker.compose.service";
const LABEL_WORKING_DIR: &str = "com.docker.compose.project.working_dir";
const LABEL_CONFIG_FILES: &str = "com.docker.compose.project.config_files";

/// Containers from `docker inspect --format INSPECT_FORMAT`. Lines that aren't one of
/// ours (a warning, a container removed half-way through) are skipped.
pub fn containers(output: &str) -> Vec<Container> {
    let mut list: Vec<Container> = output.lines().filter_map(container).take(MAX_CONTAINERS).collect();
    list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list
}

fn container(line: &str) -> Option<Container> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let id = v.get("id")?.as_str()?;
    if !is_container_id(id) {
        return None;
    }
    let labels = v.get("labels").and_then(Value::as_object);
    let label = |key: &str| {
        labels.and_then(|l| l.get(key)).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
    };
    let name = text(v.get("name").and_then(Value::as_str).unwrap_or(""));
    let name = name.strip_prefix('/').unwrap_or(&name).to_string();
    let health = v.get("health").and_then(Value::as_str).filter(|h| !h.is_empty() && *h != "none").map(text);
    let state = v.get("state").and_then(Value::as_str).map(text).unwrap_or_else(|| "unknown".into());
    let running = state == "running" || state == "restarting" || state == "paused";
    Some(Container {
        id: id.to_ascii_lowercase(),
        name: if name.is_empty() { id[..12].to_string() } else { name },
        image: text(v.get("image").and_then(Value::as_str).unwrap_or("")),
        exit_code: if running { None } else { v.get("exitCode").and_then(Value::as_i64) },
        started_at: v.get("startedAt").and_then(Value::as_str).and_then(unix_time),
        finished_at: v.get("finishedAt").and_then(Value::as_str).and_then(unix_time),
        ports: ports(v.get("ports")),
        project: label(LABEL_PROJECT).map(text),
        service: label(LABEL_SERVICE).map(text),
        working_dir: label(LABEL_WORKING_DIR).map(str::to_string),
        config_files: label(LABEL_CONFIG_FILES).map(split_files).unwrap_or_default(),
        state,
        health,
    })
}

/// Compose writes several files into the label separated by commas.
fn split_files(list: &str) -> Vec<String> {
    list.split(',').map(str::trim).filter(|f| !f.is_empty()).map(str::to_string).collect()
}

/// A full or short container id: hex only, so it is safe to put in a command.
pub fn is_container_id(id: &str) -> bool {
    (12..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `NetworkSettings.Ports`: {"80/tcp": [{"HostIp": "0.0.0.0", "HostPort": "8080"}, ...], "443/tcp": null}.
/// A port published on every address appears once for IPv4 and once for IPv6; it is
/// shown once.
fn ports(value: Option<&Value>) -> Vec<Port> {
    let Some(map) = value.and_then(Value::as_object) else { return Vec::new() };
    let mut out: Vec<Port> = Vec::new();
    for (key, bindings) in map {
        let (number, protocol) = key.split_once('/').unwrap_or((key.as_str(), "tcp"));
        let Ok(container_port) = number.parse::<u16>() else { continue };
        let protocol = text(protocol);
        let published: Vec<&Value> = bindings.as_array().map(|b| b.iter().collect()).unwrap_or_default();
        if published.is_empty() {
            out.push(Port { host_port: None, host_ip: None, container_port, protocol });
            continue;
        }
        for b in published {
            let Some(host_port) = b.get("HostPort").and_then(Value::as_str).and_then(|p| p.parse().ok()) else {
                continue;
            };
            let ip = b.get("HostIp").and_then(Value::as_str).unwrap_or("").trim();
            let host_ip = (!matches!(ip, "" | "0.0.0.0" | "::")).then(|| text(ip));
            let port = Port { host_port: Some(host_port), host_ip, container_port, protocol: protocol.clone() };
            if !out.contains(&port) {
                out.push(port);
            }
        }
    }
    // Published ports first, by host port; the rest by container port.
    out.sort_by_key(|p| (p.host_port.is_none(), p.host_port, p.container_port, p.protocol.clone(), p.host_ip.clone()));
    out
}

/// "2026-11-13T18:39:38.798262602Z" (or with an offset) to Unix seconds. Docker's
/// "0001-01-01T00:00:00Z" means never.
pub fn unix_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b' ') || b[13] != b':' {
        return None;
    }
    let num = |from: usize, to: usize| s.get(from..to)?.parse::<i64>().ok();
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if year < 1971 || !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // Skip the fraction of a second, then read the zone.
    let rest = &s[19..];
    let rest = rest.strip_prefix('.').map(|r| r.trim_start_matches(|c: char| c.is_ascii_digit())).unwrap_or(rest);
    let offset = match rest {
        "Z" | "z" => 0,
        zone if zone.len() == 6 && (zone.starts_with('+') || zone.starts_with('-')) => {
            let h: i64 = zone.get(1..3)?.parse().ok()?;
            let m: i64 = zone.get(4..6)?.parse().ok()?;
            let secs = h * 3600 + m * 60;
            if zone.starts_with('-') { -secs } else { secs }
        }
        _ => return None,
    };
    Some(days_from_civil(year, month, day) * 86400 + hour * 3600 + minute * 60 + second - offset)
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// `docker stats --no-stream --no-trunc --format '{{json .}}'`, one line per running container.
pub fn stats(output: &str) -> Vec<ContainerStats> {
    output
        .lines()
        .filter_map(|line| {
            let v: Value = serde_json::from_str(line.trim()).ok()?;
            let id = v.get("ID").or_else(|| v.get("Container")).and_then(Value::as_str)?;
            if !is_container_id(id) {
                return None;
            }
            let field = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("");
            let (used, limit) = field("MemUsage").split_once('/').unwrap_or(("", ""));
            Some(ContainerStats {
                id: id.to_ascii_lowercase(),
                cpu_pct: percent(field("CPUPerc")),
                mem_used_bytes: size(used),
                mem_limit_bytes: size(limit),
            })
        })
        .take(MAX_CONTAINERS)
        .collect()
}

/// "0.86%" to 0.86. "--" (no reading yet) to None.
fn percent(s: &str) -> Option<f64> {
    let n: f64 = s.trim().strip_suffix('%')?.trim().parse().ok()?;
    n.is_finite().then_some(n.max(0.0))
}

/// Docker's sizes: "2.5GB", "15.73kB" and "0B" count in thousands, "432.3MiB" in 1024s.
/// "2B (virtual 120MB)" reads as its first part. "N/A" or anything else gives None.
pub fn size(s: &str) -> Option<u64> {
    let s = s.trim();
    let s = s.split_once(' ').map_or(s, |(first, _)| first);
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (number, unit) = s.split_at(split);
    let n: f64 = number.parse().ok()?;
    let factor: f64 = match unit {
        "B" => 1.0,
        "kB" | "KB" | "k" | "K" => 1e3,
        "MB" | "M" => 1e6,
        "GB" | "G" => 1e9,
        "TB" | "T" => 1e12,
        "PB" | "P" => 1e15,
        "KiB" => 1024.0,
        "MiB" => 1024f64.powi(2),
        "GiB" => 1024f64.powi(3),
        "TiB" => 1024f64.powi(4),
        "PiB" => 1024f64.powi(5),
        _ => return None,
    };
    let bytes = n * factor;
    (bytes.is_finite() && bytes >= 0.0).then(|| bytes.round() as u64)
}

/// Cut to a sensible length and drop control characters.
pub fn text(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(MAX_TEXT).collect::<String>().trim().to_string()
}

// --- Cleanup preview ---

/// One thing a cleanup would remove.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    /// What is handed to docker to remove it: a container id, an image id or
    /// "name:tag", or a volume name.
    pub key: String,
    /// What the owner sees: a container or volume name, or an image name and tag.
    pub label: String,
    /// More about it: when it stopped, how old it is.
    pub detail: String,
    /// Space it takes that removing it would free, if Docker said.
    pub size_bytes: Option<u64>,
}

/// Everything a cleanup could remove from one host, read from
/// `docker system df -v --format '{{json .}}'`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPreview {
    /// Containers that are not running (exited, created, dead).
    pub containers: Vec<CleanupItem>,
    /// Images no container uses, tagged or not.
    pub images: Vec<CleanupItem>,
    /// Volumes no container uses. Named ones may hold a stack's data.
    pub volumes: Vec<CleanupItem>,
    /// Build cache entries not in use, and the space they take.
    pub build_cache_count: u32,
    pub build_cache_bytes: u64,
}

pub fn cleanup_preview(output: &str) -> Option<CleanupPreview> {
    // Some versions print a warning line first; the JSON is the line that parses.
    let v: Value = output.lines().find_map(|l| serde_json::from_str::<Value>(l.trim()).ok().filter(Value::is_object))?;
    let list = |key: &str| v.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    let field = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).map(text).unwrap_or_default();
    let mut preview = CleanupPreview::default();

    for c in list("Containers") {
        let state = field(&c, "State");
        let id = field(&c, "ID");
        if !matches!(state.as_str(), "exited" | "created" | "dead") || !is_container_id(&id) {
            continue;
        }
        let image = field(&c, "Image");
        let status = field(&c, "Status");
        preview.containers.push(CleanupItem {
            label: first_name(&field(&c, "Names")).unwrap_or_else(|| id[..12].to_string()),
            detail: [image, status].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · "),
            size_bytes: size(&field(&c, "Size")),
            key: id,
        });
    }

    // An image with several tags is listed once per tag. Each tag is removed by name
    // (removing by id would refuse), and its space is counted once.
    let mut counted: Vec<String> = Vec::new();
    for i in list("Images") {
        if field(&i, "Containers").parse::<u64>().ok() != Some(0) {
            continue;
        }
        let id = field(&i, "ID");
        let short = id.strip_prefix("sha256:").unwrap_or(&id).chars().take(12).collect::<String>();
        if short.len() < 12 || !short.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let (repo, tag) = (field(&i, "Repository"), field(&i, "Tag"));
        let tagged = !repo.is_empty() && repo != "<none>" && !tag.is_empty() && tag != "<none>";
        let (key, label) = if tagged { (format!("{repo}:{tag}"), format!("{repo}:{tag}")) } else { (id.clone(), format!("Untagged image {short}")) };
        let unique = size(&field(&i, "UniqueSize")).or_else(|| size(&field(&i, "Size")));
        let size_bytes = if counted.contains(&id) { Some(0) } else { unique };
        counted.push(id);
        let created = field(&i, "CreatedSince");
        preview.images.push(CleanupItem {
            key,
            label,
            detail: if created.is_empty() { String::new() } else { format!("Made {created}") },
            size_bytes,
        });
    }

    for vol in list("Volumes") {
        let name = field(&vol, "Name");
        if field(&vol, "Links").parse::<u64>().ok() != Some(0) || !is_volume_name(&name) {
            continue;
        }
        let labels = field(&vol, "Labels");
        let anonymous = labels.split(',').any(|l| l.trim().starts_with("com.docker.volume.anonymous"))
            || (name.len() == 64 && name.chars().all(|c| c.is_ascii_hexdigit()));
        let stack = labels
            .split(',')
            .find_map(|l| l.trim().strip_prefix("com.docker.compose.project="))
            .map(str::to_string);
        let detail = match (anonymous, stack) {
            (true, _) => "Unnamed volume".to_string(),
            (false, Some(stack)) => format!("Named volume of the {stack} stack"),
            (false, None) => "Named volume".to_string(),
        };
        let label = if anonymous { format!("Unnamed volume {}", &name[..name.len().min(12)]) } else { name.clone() };
        preview.volumes.push(CleanupItem { key: name, label, detail, size_bytes: size(&field(&vol, "Size")) });
    }

    for b in list("BuildCache") {
        if field(&b, "InUse") == "true" {
            continue;
        }
        preview.build_cache_count += 1;
        // Shared entries are counted by every image that uses them; only count each once.
        if field(&b, "Shared") != "true" {
            preview.build_cache_bytes += size(&field(&b, "Size")).unwrap_or(0);
        }
    }
    Some(preview)
}

/// `docker ps` lists a container's names separated by commas; the first is its own.
fn first_name(names: &str) -> Option<String> {
    names.split(',').map(str::trim).find(|n| !n.is_empty()).map(|n| n.trim_start_matches('/').to_string())
}

/// Volume names Docker allows: letters, digits and "_.-", starting with a letter or digit.
pub fn is_volume_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && name.len() <= 255
}

/// An image reference as listed by Docker: "name:tag", "registry:5000/a/b:tag" or
/// "sha256:<hex>". Safe characters only, so nothing odd ends up in a command even
/// though it is quoted there as well.
pub fn is_image_ref(r: &str) -> bool {
    !r.is_empty()
        && r.len() <= 512
        && !r.starts_with('-')
        && r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/' | ':' | '@'))
}

// --- Docker's own answer to "is it there?" ---

/// What a look for Docker on a machine found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Probe {
    /// No docker command on the machine.
    Missing,
    /// The docker command is there. `version` is the engine's version when it answered;
    /// `error` is what stopped it answering otherwise.
    Present { version: Option<String>, compose: Compose, error: Option<String> },
}

/// Which compose command the machine has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Compose {
    /// `docker compose` (the plugin that comes with current Docker).
    Plugin,
    /// The older stand-alone `docker-compose`.
    Standalone,
    None,
}

impl Compose {
    pub fn as_str(self) -> &'static str {
        match self {
            Compose::Plugin => "plugin",
            Compose::Standalone => "standalone",
            Compose::None => "none",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "plugin" => Compose::Plugin,
            "standalone" => Compose::Standalone,
            _ => Compose::None,
        }
    }
}

/// Reads what [`super::scripts::PROBE`] prints:
/// ```text
/// @@hcc-docker none|cli
/// @@version <json string>        (or the error docker printed, on the lines before @@exit)
/// @@exit <code>
/// @@compose plugin|standalone|none
/// ```
pub fn probe(output: &str) -> Option<Probe> {
    let mut lines = output.lines().skip_while(|l| !l.starts_with("@@hcc-docker "));
    match lines.next()?.trim() {
        "@@hcc-docker none" => return Some(Probe::Missing),
        "@@hcc-docker cli" => {}
        _ => return None,
    }
    let mut said: Vec<&str> = Vec::new();
    let mut exit: Option<i32> = None;
    let mut compose = Compose::None;
    for line in lines {
        if let Some(code) = line.strip_prefix("@@exit ") {
            exit = code.trim().parse().ok();
        } else if let Some(kind) = line.strip_prefix("@@compose ") {
            compose = Compose::parse(kind.trim());
        } else if exit.is_none() && !line.trim().is_empty() {
            said.push(line);
        }
    }
    let exit = exit?;
    let version = (exit == 0)
        .then(|| said.iter().find_map(|l| serde_json::from_str::<String>(l.trim()).ok()))
        .flatten()
        .map(|v| text(&v))
        .filter(|v| !v.is_empty());
    let error = match &version {
        Some(_) => None,
        None => Some(text(&said.join(" "))).filter(|s| !s.is_empty()).or(Some(format!("docker ended with {exit}"))),
    };
    Some(Probe::Present { version, compose, error })
}

/// Why a docker command failed, in a form the screens can give advice for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The user may not use Docker (not in the docker group).
    Permission,
    /// Docker is installed but its service isn't running.
    Daemon,
    /// There is no docker command.
    Missing,
}

/// Recognise Docker's own messages for the usual setup problems. Wording differs
/// between versions, so several forms are checked.
pub fn refusal(said: &str, exit: Option<u32>) -> Option<Refusal> {
    let s = said.to_ascii_lowercase();
    let about_docker = s.contains("docker api") || s.contains("docker daemon") || s.contains("docker.sock");
    if s.contains("permission denied") && about_docker {
        Some(Refusal::Permission)
    } else if s.contains("cannot connect to the docker daemon")
        || s.contains("failed to connect to the docker api")
        || s.contains("is the docker daemon running")
    {
        Some(Refusal::Daemon)
    } else if exit == Some(127) && (s.contains("docker: not found") || s.contains("docker: command not found")) {
        Some(Refusal::Missing)
    } else {
        None
    }
}

/// Group containers by compose project: (project, containers) in name order, then the
/// containers that belong to none.
pub fn by_project(containers: Vec<Container>) -> (BTreeMap<String, Vec<Container>>, Vec<Container>) {
    let mut stacks: BTreeMap<String, Vec<Container>> = BTreeMap::new();
    let mut loose = Vec::new();
    for c in containers {
        match c.project.clone() {
            Some(p) => stacks.entry(p).or_default().push(c),
            None => loose.push(c),
        }
    }
    for list in stacks.values_mut() {
        list.sort_by(|a, b| (a.service.as_deref().unwrap_or(&a.name), &a.name).cmp(&(b.service.as_deref().unwrap_or(&b.name), &b.name)));
    }
    (stacks, loose)
}

/// Remove the colour and cursor codes some programs print (compose's progress output),
/// so the text reads cleanly on screen.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    // Parameters, then one final letter.
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    // Up to the bell or ESC \.
                    for c in chars.by_ref() {
                        if c == '\u{7}' || c == '\\' {
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
        } else if c == '\r' {
            // Progress lines rewrite themselves with a carriage return; keep the last state.
            if chars.peek() != Some(&'\n') {
                let line_start = out.rfind('\n').map_or(0, |i| i + 1);
                out.truncate(line_start);
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID_A: &str = "55767794adcf76131c5d6cfd836c2132261d68f8066153dd24775b1cbcd4ab55";
    const ID_B: &str = "18fe878ae47d9ed781b211210972a02cfbba74af6f00b87f4c9ed7c238e7c588";
    const ID_C: &str = "03d5be23ad564bc98ea1912327ee9bdbbcdb251b3f24ca93833159c7ce5747ee";

    /// Shaped like real `docker inspect --format INSPECT_FORMAT` output (Docker 29).
    fn inspect_sample() -> String {
        [
            format!(
                r#"{{"id":"{ID_A}","name":"/media-jellyfin-1","image":"jellyfin/jellyfin:latest","state":"running","startedAt":"2026-11-13T18:39:38.798262602Z","finishedAt":"0001-01-01T00:00:00Z","exitCode":0,"health":"healthy","labels":{{"com.docker.compose.project":"media","com.docker.compose.project.config_files":"/srv/media/compose.yml,/srv/media/compose.override.yml","com.docker.compose.project.working_dir":"/srv/media","com.docker.compose.service":"jellyfin"}},"ports":{{"8096/tcp":[{{"HostIp":"0.0.0.0","HostPort":"8096"}},{{"HostIp":"::","HostPort":"8096"}}],"8920/tcp":null,"1900/udp":[{{"HostIp":"192.168.68.21","HostPort":"1900"}}]}}}}"#
            ),
            "template parsing error: template: :1:237: something odd".to_string(),
            format!(
                r#"{{"id":"{ID_B}","name":"/old-backup","image":"alpine:3.21","state":"exited","startedAt":"2026-11-07T15:30:15Z","finishedAt":"2026-11-07T15:31:15+01:00","exitCode":143,"health":null,"labels":null,"ports":{{}}}}"#
            ),
            format!(
                r#"{{"id":"{ID_C}","name":"/media-sonarr-1","image":"linuxserver/sonarr","state":"restarting","startedAt":"2026-11-17T10:00:00Z","finishedAt":"2026-11-17T09:59:00Z","exitCode":1,"health":"none","labels":{{"com.docker.compose.project":"media","com.docker.compose.service":"sonarr"}},"ports":null}}"#
            ),
        ]
        .join("\n")
    }

    #[test]
    fn containers_from_inspect_output() {
        let list = containers(&inspect_sample());
        assert_eq!(list.len(), 3, "the error line is skipped");
        let jelly = list.iter().find(|c| c.id == ID_A).unwrap();
        assert_eq!(jelly.name, "media-jellyfin-1", "no leading slash");
        assert_eq!(jelly.state, "running");
        assert_eq!(jelly.health.as_deref(), Some("healthy"));
        assert_eq!(jelly.exit_code, None, "a running container has no exit code");
        assert_eq!(jelly.started_at, Some(1_794_595_178));
        assert_eq!(jelly.finished_at, None, "the year-1 date means never");
        assert_eq!(jelly.project.as_deref(), Some("media"));
        assert_eq!(jelly.service.as_deref(), Some("jellyfin"));
        assert_eq!(jelly.working_dir.as_deref(), Some("/srv/media"));
        assert_eq!(jelly.config_files, ["/srv/media/compose.yml", "/srv/media/compose.override.yml"]);
        assert_eq!(
            jelly.ports,
            [
                Port { host_port: Some(1900), host_ip: Some("192.168.68.21".into()), container_port: 1900, protocol: "udp".into() },
                Port { host_port: Some(8096), host_ip: None, container_port: 8096, protocol: "tcp".into() },
                Port { host_port: None, host_ip: None, container_port: 8920, protocol: "tcp".into() },
            ],
            "IPv4 and IPv6 bindings of one port show once; unpublished ports last"
        );

        let old = list.iter().find(|c| c.id == ID_B).unwrap();
        assert_eq!((old.state.as_str(), old.exit_code), ("exited", Some(143)));
        assert_eq!(old.finished_at, Some(1_794_065_475 - 3600), "the offset is taken off");
        assert_eq!(old.project, None);
        assert!(old.ports.is_empty());

        let sonarr = list.iter().find(|c| c.id == ID_C).unwrap();
        assert_eq!(sonarr.health, None, "\"none\" is no health check");
        assert_eq!(sonarr.exit_code, None, "restarting counts as running");
        assert_eq!(sonarr.config_files, Vec::<String>::new());
    }

    #[test]
    fn containers_are_grouped_into_stacks() {
        let (stacks, loose) = by_project(containers(&inspect_sample()));
        assert_eq!(stacks.keys().collect::<Vec<_>>(), ["media"]);
        let names: Vec<&str> = stacks["media"].iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["media-jellyfin-1", "media-sonarr-1"], "in service order");
        assert_eq!(loose.len(), 1);
    }

    #[test]
    fn odd_lines_never_become_containers() {
        let bad = [
            "",
            "not json",
            r#"{"id":"zzzz","name":"/x"}"#,
            r#"{"id":"abc","name":"/too-short-id"}"#,
            r#"["a list"]"#,
            r#"{"name":"/no-id"}"#,
        ];
        assert!(containers(&bad.join("\n")).is_empty());
        // A container with no name keeps a short id instead.
        let unnamed = format!(r#"{{"id":"{ID_A}","name":"","state":"created"}}"#);
        assert_eq!(containers(&unnamed)[0].name, &ID_A[..12]);
    }

    #[test]
    fn times_are_read_in_any_zone() {
        assert_eq!(unix_time("1970-01-01T00:00:00Z"), None, "before 1971 counts as never");
        assert_eq!(unix_time("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(unix_time("2000-03-01T01:30:00.5+01:30"), Some(951_868_800));
        assert_eq!(unix_time("2000-02-29T19:00:00-05:00"), Some(951_868_800));
        assert_eq!(unix_time("0001-01-01T00:00:00Z"), None);
        assert_eq!(unix_time("yesterday"), None);
        assert_eq!(unix_time("2000-13-01T00:00:00Z"), None);
        assert_eq!(unix_time("2000-03-01T00:00:00"), None, "no zone, no guess");
    }

    #[test]
    fn stats_lines() {
        let out = format!(
            "{}\n{}\nWARNING: odd\n",
            format!(r#"{{"BlockIO":"0B / 0B","CPUPerc":"0.86%","Container":"{ID_A}","ID":"{ID_A}","MemPerc":"1.13%","MemUsage":"432.3MiB / 37.24GiB","Name":"neo4j","NetIO":"899kB / 43.1MB","PIDs":"100"}}"#),
            format!(r#"{{"CPUPerc":"--","ID":"{ID_B}","MemUsage":"-- / --"}}"#),
        );
        let s = stats(&out);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].cpu_pct, Some(0.86));
        assert_eq!(s[0].mem_used_bytes, Some(453_299_405));
        assert_eq!(s[0].mem_limit_bytes, Some(39_986_145_526));
        assert_eq!((s[1].cpu_pct, s[1].mem_used_bytes), (None, None), "no reading yet");
    }

    #[test]
    fn sizes_in_both_kinds_of_unit() {
        assert_eq!(size("0B"), Some(0));
        assert_eq!(size("2B"), Some(2));
        assert_eq!(size("15.73kB"), Some(15_730));
        assert_eq!(size("2.5GB"), Some(2_500_000_000));
        assert_eq!(size(" 1.066GB "), Some(1_066_000_000));
        assert_eq!(size("1KiB"), Some(1024));
        assert_eq!(size("1.5MiB"), Some(1_572_864));
        assert_eq!(size("2B (virtual 120MB)"), Some(2));
        assert_eq!(size("N/A"), None);
        assert_eq!(size("--"), None);
        assert_eq!(size("12"), None, "a number with no unit is not a size");
        assert_eq!(size("5 parsecs"), None);
    }

    fn df_sample() -> String {
        let v = serde_json::json!({
            "Images": [
                {"Containers": "0", "CreatedSince": "4 hours ago", "ID": "sha256:6bfef6b36e197482787095c6c7b0ff827f68cb72733fbc4769cf78c420b697f6", "Repository": "nginx", "Tag": "1.27", "SharedSize": "2B", "Size": "2.5GB", "UniqueSize": "2.501GB"},
                {"Containers": "0", "CreatedSince": "4 hours ago", "ID": "sha256:6bfef6b36e197482787095c6c7b0ff827f68cb72733fbc4769cf78c420b697f6", "Repository": "nginx", "Tag": "stable", "SharedSize": "2B", "Size": "2.5GB", "UniqueSize": "2.501GB"},
                {"Containers": "2", "ID": "sha256:4327e93832653e4b08f70ba4c1c4274ff6300cff3b01cd817a916a32dc480aac", "Repository": "postgres", "Tag": "18", "UniqueSize": "13.54kB"},
                {"Containers": "0", "CreatedSince": "2 weeks ago", "ID": "sha256:74286bbcc5c7d4798ee0e15848aac6cc815c15635a165639a12fd0f7bdedd2d2", "Repository": "<none>", "Tag": "<none>", "Size": "1.07GB", "UniqueSize": "15.73kB"},
                {"Containers": "0", "ID": "not-an-id", "Repository": "evil", "Tag": "x"}
            ],
            "Containers": [
                {"ID": ID_A, "Names": "media-jellyfin-1", "Image": "jellyfin/jellyfin", "State": "running", "Status": "Up 2 days", "Size": "135MB"},
                {"ID": ID_B, "Names": "old-backup", "Image": "alpine:3.21", "State": "exited", "Status": "Exited (143) 2 days ago", "Size": "250kB"},
                {"ID": ID_C, "Names": "", "Image": "busybox", "State": "created", "Status": "Created", "Size": "0B"}
            ],
            "Volumes": [
                {"Links": "1", "Name": "media_config", "Size": "558.7MB", "Labels": "com.docker.compose.project=media,com.docker.compose.volume=config"},
                {"Links": "0", "Name": "photos_db", "Size": "1.2GB", "Labels": "com.docker.compose.project=photos,com.docker.compose.volume=db"},
                {"Links": "0", "Name": "c48bfb23c40211efbf97ceccbca86f8e5b39c266c350246cf41b7e680073a631", "Size": "0B", "Labels": "com.docker.volume.anonymous="},
                {"Links": "0", "Name": "scratch", "Size": "N/A", "Labels": ""},
                {"Links": "0", "Name": "-rf", "Size": "1B", "Labels": ""}
            ],
            "BuildCache": [
                {"ID": "a", "InUse": "false", "Shared": "false", "Size": "4.1kB"},
                {"ID": "b", "InUse": "true", "Shared": "false", "Size": "100MB"},
                {"ID": "c", "InUse": "false", "Shared": "true", "Size": "50MB"},
                {"ID": "d", "InUse": "false", "Shared": "false", "Size": "358.4MB"}
            ]
        });
        format!("WARNING: something to say first\n{v}\n")
    }

    #[test]
    fn cleanup_preview_lists_only_what_is_unused() {
        let p = cleanup_preview(&df_sample()).unwrap();

        let containers: Vec<(&str, &str)> = p.containers.iter().map(|c| (c.key.as_str(), c.label.as_str())).collect();
        assert_eq!(containers, [(ID_B, "old-backup"), (ID_C, &ID_C[..12])], "running ones are never listed");
        assert_eq!(p.containers[0].detail, "alpine:3.21 · Exited (143) 2 days ago");
        assert_eq!(p.containers[0].size_bytes, Some(250_000));

        let images: Vec<(&str, Option<u64>)> = p.images.iter().map(|i| (i.key.as_str(), i.size_bytes)).collect();
        assert_eq!(
            images,
            [
                ("nginx:1.27", Some(2_501_000_000)),
                ("nginx:stable", Some(0)),
                ("sha256:74286bbcc5c7d4798ee0e15848aac6cc815c15635a165639a12fd0f7bdedd2d2", Some(15_730)),
            ],
            "used images are left out, a second tag frees nothing more, odd ids are skipped"
        );
        assert_eq!(p.images[2].label, "Untagged image 74286bbcc5c7");

        let volumes: Vec<(&str, &str)> = p.volumes.iter().map(|v| (v.key.as_str(), v.detail.as_str())).collect();
        assert_eq!(
            volumes,
            [
                ("photos_db", "Named volume of the photos stack"),
                ("c48bfb23c40211efbf97ceccbca86f8e5b39c266c350246cf41b7e680073a631", "Unnamed volume"),
                ("scratch", "Named volume"),
            ],
            "a volume in use, or with a name docker would read as an option, is never listed"
        );
        assert_eq!(p.volumes[2].size_bytes, None);

        assert_eq!(p.build_cache_count, 3);
        assert_eq!(p.build_cache_bytes, 4_100 + 358_400_000, "in-use entries left out, shared ones not added twice");
    }

    #[test]
    fn cleanup_preview_needs_the_json() {
        assert_eq!(cleanup_preview("Error: something"), None);
        assert_eq!(cleanup_preview(""), None);
        assert_eq!(cleanup_preview("{}"), Some(CleanupPreview::default()));
    }

    #[test]
    fn names_and_refs_that_are_safe_to_pass_on() {
        assert!(is_volume_name("photos_db"));
        assert!(is_volume_name("a.b-c_d"));
        assert!(!is_volume_name("-rf"));
        assert!(!is_volume_name("a b"));
        assert!(!is_volume_name("a;rm"));
        assert!(!is_volume_name(""));
        assert!(is_image_ref("registry.local:5000/team/app:1.2"));
        assert!(is_image_ref("sha256:74286bbcc5c7"));
        assert!(!is_image_ref("--force"));
        assert!(!is_image_ref("a b"));
        assert!(!is_image_ref("a$(x)"));
        assert!(is_container_id(ID_A));
        assert!(is_container_id(&ID_A[..12]));
        assert!(!is_container_id(&ID_A[..11]));
        assert!(!is_container_id("media-jellyfin-1"));
    }

    #[test]
    fn probe_output() {
        assert_eq!(probe("motd\n@@hcc-docker none\n"), Some(Probe::Missing));
        assert_eq!(
            probe("@@hcc-docker cli\n\"29.9.3\"\n@@exit 0\n@@compose plugin\n"),
            Some(Probe::Present { version: Some("29.9.3".into()), compose: Compose::Plugin, error: None })
        );
        let denied = "@@hcc-docker cli\npermission denied while trying to connect to the docker API at \
                      unix:///var/run/docker.sock\n@@exit 1\n@@compose standalone\n";
        match probe(denied) {
            Some(Probe::Present { version: None, compose: Compose::Standalone, error: Some(e) }) => {
                assert!(e.starts_with("permission denied"));
                assert_eq!(refusal(&e, Some(1)), Some(Refusal::Permission));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(probe("nothing to see"), None);
        assert_eq!(probe("@@hcc-docker cli\n\"29\"\n"), None, "no exit line: cut short");
    }

    #[test]
    fn docker_refusals_in_old_and_new_wording() {
        let cases = [
            ("Got permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock: Get ...: dial unix /var/run/docker.sock: connect: permission denied", Some(Refusal::Permission)),
            ("permission denied while trying to connect to the docker API at unix:///var/run/docker.sock", Some(Refusal::Permission)),
            ("Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?", Some(Refusal::Daemon)),
            ("failed to connect to the docker API at unix:///x.sock; check if the path is correct and if the daemon is running", Some(Refusal::Daemon)),
            ("Error response from daemon: No such container: x", None),
            ("cat: /srv/x: Permission denied", None),
        ];
        for (said, want) in cases {
            assert_eq!(refusal(said, Some(1)), want, "{said}");
        }
        assert_eq!(refusal("sh: 1: docker: not found", Some(127)), Some(Refusal::Missing));
        assert_eq!(refusal("bash: docker: command not found", Some(127)), Some(Refusal::Missing));
    }

    #[test]
    fn colour_codes_and_progress_rewrites_are_removed() {
        assert_eq!(strip_ansi("\u{1b}[32mok\u{1b}[0m done"), "ok done");
        assert_eq!(strip_ansi("Pulling 10%\rPulling 50%\rPulling done\nnext\r\n"), "Pulling done\nnext\n");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}text"), "text");
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
