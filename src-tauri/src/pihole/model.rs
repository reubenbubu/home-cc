//! Reading Pi-hole v6's answers (its REST API under /api). The field names
//! were checked against the OpenAPI description that ships with Pi-hole FTL
//! (src/api/docs/content/specs/ in the pi-hole/FTL repository): stats.yaml,
//! dns.yaml, lists.yaml, info.yaml, auth.yaml and common.yaml.
//!
//! Anything a Pi-hole leaves out is shown as "not known" rather than failing
//! the whole reading, except the few numbers the screen is built around.

use serde::Serialize;
use serde_json::Value;

use super::client::clean;

/// GET /api/stats/summary, the parts the screen shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// Queries in the last 24 hours (Pi-hole's "today").
    pub queries: u64,
    pub blocked: u64,
    pub percent_blocked: f64,
    /// Clients seen in the last 24 hours.
    pub clients_active: Option<u64>,
    /// Clients Pi-hole has ever seen.
    pub clients_total: Option<u64>,
    /// Domains on the blocklists, after Pi-hole merged them ("gravity").
    pub domains_on_lists: Option<u64>,
    /// Unix seconds of the last blocklist update; None when Pi-hole doesn't know (it sends 0).
    pub gravity_updated: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BlockingState {
    Enabled,
    Disabled,
    /// Pi-hole couldn't switch blocking on (its own word).
    Failed,
    Unknown,
}

/// GET or POST /api/dns/blocking.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Blocking {
    pub state: BlockingState,
    /// Seconds until Pi-hole switches back by itself; None when the state stays until changed.
    pub timer_secs: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ListKind {
    Block,
    Allow,
}

/// One list from GET /api/lists: a blocklist ("adlist") or an allowlist, by its address.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdList {
    pub address: String,
    pub kind: ListKind,
    pub enabled: bool,
    pub comment: Option<String>,
    /// Domains on it at the last update.
    pub domains: Option<u64>,
    /// Pi-hole's own number for its state: 1 updated, 2 unchanged, 3 couldn't be downloaded (an older copy is
    /// used), 4 couldn't be downloaded and no copy; 0 not downloaded yet.
    pub status: Option<i64>,
}

/// GET /api/info/version: what runs on the Pi-hole.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub core: Option<String>,
    pub web: Option<String>,
    pub ftl: Option<String>,
    /// Set when it runs in Docker: the image's version.
    pub docker: Option<String>,
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|n| n.is_finite())
}

fn count(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| num(v).filter(|n| *n >= 0.0).map(|n| n.round() as u64))
}

fn text(v: &Value) -> Option<String> {
    v.as_str().map(clean).filter(|s| !s.is_empty())
}

pub fn parse_summary(data: &Value) -> Result<Summary, String> {
    let q = &data["queries"];
    let (Some(queries), Some(blocked)) = (count(&q["total"]), count(&q["blocked"])) else {
        return Err("Pi-hole's summary had no query counts. Is this Pi-hole v6?".into());
    };
    let percent_blocked = num(&q["percent_blocked"]).unwrap_or_else(|| {
        if queries == 0 { 0.0 } else { blocked as f64 * 100.0 / queries as f64 }
    });
    Ok(Summary {
        queries,
        blocked,
        percent_blocked: percent_blocked.clamp(0.0, 100.0),
        clients_active: count(&data["clients"]["active"]),
        clients_total: count(&data["clients"]["total"]),
        domains_on_lists: count(&data["gravity"]["domains_being_blocked"]),
        gravity_updated: data["gravity"]["last_update"].as_i64().filter(|t| *t > 0),
    })
}

pub fn parse_blocking(data: &Value) -> Result<Blocking, String> {
    let state = match data["blocking"].as_str() {
        Some("enabled") => BlockingState::Enabled,
        Some("disabled") => BlockingState::Disabled,
        Some("failed") => BlockingState::Failed,
        Some(_) => BlockingState::Unknown,
        // Some older answers gave a plain true or false.
        None => match data["blocking"].as_bool() {
            Some(true) => BlockingState::Enabled,
            Some(false) => BlockingState::Disabled,
            None => return Err("Pi-hole didn't say whether blocking is on.".into()),
        },
    };
    Ok(Blocking { state, timer_secs: num(&data["timer"]).filter(|t| *t > 0.0) })
}

pub fn parse_lists(data: &Value) -> Result<Vec<AdList>, String> {
    let Some(items) = data["lists"].as_array() else {
        return Err("Pi-hole's answer had no lists in it.".into());
    };
    let mut out = Vec::new();
    for item in items {
        let Some(address) = item["address"].as_str().map(str::trim).filter(|a| !a.is_empty()) else { continue };
        let kind = match item["type"].as_str() {
            Some("allow") => ListKind::Allow,
            _ => ListKind::Block,
        };
        out.push(AdList {
            address: clean(address),
            kind,
            enabled: item["enabled"].as_bool().unwrap_or(true),
            comment: text(&item["comment"]),
            domains: count(&item["number"]),
            status: item["status"].as_i64(),
        });
    }
    Ok(out)
}

pub fn parse_version(data: &Value) -> Version {
    let v = &data["version"];
    let local = |part: &str| text(&v[part]["local"]["version"]);
    Version { core: local("core"), web: local("web"), ftl: local("ftl"), docker: text(&v["docker"]["local"]) }
}

/// Pi-hole's error object: {"error": {"key", "message", "hint"}}.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiFault {
    pub key: String,
    pub message: String,
    pub hint: Option<String>,
}

pub fn parse_fault(body: &Value) -> Option<ApiFault> {
    let e = body.get("error")?;
    Some(ApiFault {
        key: e["key"].as_str().unwrap_or_default().to_string(),
        message: text(&e["message"]).unwrap_or_default(),
        hint: text(&e["hint"]),
    })
}

/// The session part of POST or GET /api/auth.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub valid: bool,
    /// Two-factor login is switched on.
    pub totp: bool,
    pub sid: Option<String>,
    /// Seconds the session lasts without use; -1 or 0 when no login is needed.
    pub validity: i64,
    pub message: Option<String>,
}

pub fn parse_session(body: &Value) -> Option<SessionInfo> {
    let s = body.get("session")?;
    Some(SessionInfo {
        valid: s["valid"].as_bool()?,
        totp: s["totp"].as_bool().unwrap_or(false),
        sid: s["sid"].as_str().filter(|x| !x.is_empty()).map(str::to_string),
        validity: s["validity"].as_i64().unwrap_or(0),
        message: text(&s["message"]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn reads_the_summary() {
        let s = parse_summary(&json(include_str!("testdata/summary.json"))).unwrap();
        assert_eq!(
            s,
            Summary {
                queries: 48213,
                blocked: 9872,
                percent_blocked: 20.475805,
                clients_active: Some(23),
                clients_total: Some(31),
                domains_on_lists: Some(412880),
                gravity_updated: Some(1763190000),
            }
        );
        // Without a percentage it is worked out; an unknown list update is None, not 1970.
        let s = parse_summary(&json(r#"{"queries":{"total":200,"blocked":50},"gravity":{"last_update":0}}"#)).unwrap();
        assert_eq!((s.percent_blocked, s.gravity_updated, s.clients_active), (25.0, None, None));
        assert_eq!(parse_summary(&json(r#"{"queries":{"total":0,"blocked":0}}"#)).unwrap().percent_blocked, 0.0);
        assert!(parse_summary(&json(r#"{"took":0.1}"#)).is_err());
    }

    #[test]
    fn reads_blocking() {
        let b = parse_blocking(&json(r#"{"blocking":"disabled","timer":287.5,"took":0.0001}"#)).unwrap();
        assert_eq!(b, Blocking { state: BlockingState::Disabled, timer_secs: Some(287.5) });
        let b = parse_blocking(&json(r#"{"blocking":"enabled","timer":null}"#)).unwrap();
        assert_eq!(b, Blocking { state: BlockingState::Enabled, timer_secs: None });
        assert_eq!(parse_blocking(&json(r#"{"blocking":"failed"}"#)).unwrap().state, BlockingState::Failed);
        assert_eq!(parse_blocking(&json(r#"{"blocking":"something new"}"#)).unwrap().state, BlockingState::Unknown);
        assert_eq!(parse_blocking(&json(r#"{"blocking":false}"#)).unwrap().state, BlockingState::Disabled);
        assert!(parse_blocking(&json(r#"{}"#)).is_err());
    }

    #[test]
    fn reads_the_lists() {
        let lists = parse_lists(&json(include_str!("testdata/lists.json"))).unwrap();
        assert_eq!(lists.len(), 3, "the entry without an address is left out");
        assert_eq!(
            lists[0],
            AdList {
                address: "https://raw.githubusercontent.com/StevenBlack/hosts/master/hosts".into(),
                kind: ListKind::Block,
                enabled: true,
                comment: Some("Migrated from /etc/pihole/adlists.list".into()),
                domains: Some(81543),
                status: Some(2),
            }
        );
        assert_eq!((lists[1].enabled, lists[1].comment.as_deref()), (false, None));
        assert_eq!(lists[2].kind, ListKind::Allow);
        assert!(parse_lists(&json(r#"{"took":1}"#)).is_err());
    }

    #[test]
    fn reads_the_version() {
        let v = parse_version(&json(include_str!("testdata/version.json")));
        assert_eq!(v.core.as_deref(), Some("v6.1.4"));
        assert_eq!(v.web.as_deref(), Some("v6.2.1"));
        assert_eq!(v.ftl.as_deref(), Some("v6.2.3"));
        assert_eq!(v.docker, None);
        assert_eq!(parse_version(&json("{}")), Version { core: None, web: None, ftl: None, docker: None });
    }

    #[test]
    fn reads_errors_and_sessions() {
        let f = parse_fault(&json(
            r#"{"error":{"key":"api_seats_exceeded","message":"API seats exceeded","hint":"increase webserver.api.max_sessions"},"took":0}"#,
        ))
        .unwrap();
        assert_eq!(f.key, "api_seats_exceeded");
        assert_eq!(f.hint.as_deref(), Some("increase webserver.api.max_sessions"));
        assert_eq!(parse_fault(&json(r#"{"session":{}}"#)), None);

        let s = parse_session(&json(
            r#"{"session":{"valid":true,"totp":false,"sid":"vFA+EP4MQ5JJvJg+3Q2Jnw=","csrf":"Ux87YTIiMOf/GKCefVIOMw=","validity":1800,"message":"app-password correct"},"took":0.03}"#,
        ))
        .unwrap();
        assert_eq!((s.valid, s.sid.as_deref(), s.validity), (true, Some("vFA+EP4MQ5JJvJg+3Q2Jnw="), 1800));
        let s = parse_session(&json(r#"{"session":{"valid":true,"totp":false,"sid":null,"validity":-1,"message":"no password set"}}"#)).unwrap();
        assert_eq!((s.valid, s.sid), (true, None));
        let s = parse_session(&json(r#"{"session":{"valid":false,"totp":true,"sid":null,"validity":-1,"message":"password incorrect"}}"#)).unwrap();
        assert_eq!((s.valid, s.totp), (false, true));
    }
}
