//! Pi-hole: how each Pi-hole is doing (queries and blocked today, clients,
//! blocklists), pausing and resuming blocking, and a comparison of the
//! blocklists between Pi-holes (done on the screen). Everything goes through
//! Pi-hole v6's own API under /api, with its web or app password; nothing is
//! installed on the Pi-hole. Pi-hole v5 is recognised and said to be
//! unsupported.
//!
//! - client.rs: one HTTPS (pinned certificate) or HTTP call.
//! - auth.rs: logging in, keeping one session per Pi-hole, logging out.
//! - model.rs: reading Pi-hole's answers.
//! - store.rs: which devices are Pi-holes, their scheme, port and certificate.
//! - commands.rs: what the Pi-hole screen can ask for.
//!
//! The screens ask for a fresh reading every 30 seconds while they are open;
//! nothing runs in the background here.

pub mod auth;
pub mod client;
pub mod commands;
pub mod model;
pub mod store;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hyper::Method;
use serde::Serialize;
use serde_json::json;

use crate::error::{Error, Result};
use crate::inventory::{Device, DeviceKind, Inventory, now};
use crate::proxmox::pin::same_fingerprint;
use crate::secrets;
use auth::{Failure, Password, PiholeErrorKind, Session, Transport};
use client::{Endpoint, HTTP_PORT, HTTPS_PORT, Scheme};
use model::{AdList, Blocking, Summary, Version};
use store::HostConfig;

/// Keychain entry for a Pi-hole's password.
pub fn password_key(device_id: i64) -> String {
    format!("pihole-password:{device_id}")
}

/// Longest pause the screen offers is "until I turn it back on" (no timer); a timer longer than a day is
/// almost certainly a typing mistake.
pub const MAX_PAUSE_SECS: u32 = 24 * 3600;

/// One Pi-hole's settings as the screen sees them. The password is never sent back, only whether one is
/// stored.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiholeSettings {
    pub device_id: i64,
    pub https: bool,
    pub port: u16,
    pub pinned_fingerprint: Option<String>,
    pub has_password: bool,
}

/// One reading of a Pi-hole.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reading {
    pub summary: Summary,
    pub blocking: Blocking,
    /// None when the lists couldn't be read (lists_error says why); the rest is still shown.
    pub lists: Option<Vec<AdList>>,
    pub lists_error: Option<String>,
    pub version: Option<Version>,
}

/// One Pi-hole's state, as the screens see it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PiholeStatus {
    pub device_id: i64,
    pub reading: Option<Reading>,
    /// Unix seconds of the reading.
    pub fetched_at: Option<i64>,
    pub error: Option<String>,
    pub error_kind: Option<PiholeErrorKind>,
    /// With newCert and certChanged: the certificate the Pi-hole showed, for the owner to check.
    pub offered_fingerprint: Option<String>,
    /// With certChanged: the one approved before.
    pub expected_fingerprint: Option<String>,
    pub pinned_fingerprint: Option<String>,
    pub https: bool,
    pub port: u16,
}

/// What a device answered when home-cc looked for Pi-hole on it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub device_id: i64,
    /// "v6", "v5", or None when nothing like Pi-hole answered.
    pub found: Option<String>,
}

/// Checks a port typed by the owner.
pub fn check_port(port: u16) -> std::result::Result<(), String> {
    if port == 0 {
        return Err(format!("The port must be between 1 and 65535. Pi-hole uses {HTTPS_PORT} for HTTPS and {HTTP_PORT} for HTTP."));
    }
    Ok(())
}

/// Builds the endpoint and password from what is stored, or says what is missing.
pub fn endpoint(device: &Device, config: &HostConfig, password: Option<String>) -> std::result::Result<(Endpoint, Password), Failure> {
    let Some(password) = password.filter(|p| !p.is_empty()) else {
        return Err(Failure::new(
            PiholeErrorKind::NoPassword,
            "Add this Pi-hole's password: an app password (best) or the password of its web interface.",
        ));
    };
    let ep = Endpoint {
        host: device.host.trim().to_string(),
        port: config.port,
        scheme: config.scheme,
        pinned: if config.scheme == Scheme::Https { config.certificate.clone() } else { None },
    };
    Ok((ep, Password(password)))
}

/// One Pi-hole's session, and where it was made (to log out there).
#[derive(Default)]
struct Slot {
    session: Option<Session>,
    ep: Option<Endpoint>,
}

#[derive(Default)]
pub struct PiHole {
    /// One session per Pi-hole. Behind an async lock, so two readings of the same Pi-hole take turns and
    /// never log in twice.
    slots: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<Slot>>>>,
    /// The certificate each Pi-hole showed that the owner hasn't approved yet.
    offered: Mutex<HashMap<i64, String>>,
}

/// Reads everything the screen shows, with the session in `slot`. The summary and blocking state must come
/// back; the lists and version may fail without spoiling the rest.
async fn read_all<T: Transport>(t: &T, slot: &mut Option<Session>, pw: &Password) -> std::result::Result<Reading, Failure> {
    let other = |m: String| Failure::new(PiholeErrorKind::Other, m);
    let summary = model::parse_summary(&auth::call(t, slot, pw, Method::GET, "/api/stats/summary", None).await?).map_err(other)?;
    let blocking = model::parse_blocking(&auth::call(t, slot, pw, Method::GET, "/api/dns/blocking", None).await?).map_err(other)?;
    let (lists, lists_error) = match auth::call(t, slot, pw, Method::GET, "/api/lists", None).await {
        Ok(data) => match model::parse_lists(&data) {
            Ok(l) => (Some(l), None),
            Err(m) => (None, Some(m)),
        },
        Err(f) if f.kind == PiholeErrorKind::Other => (None, Some(f.message)),
        Err(f) => return Err(f),
    };
    let version = match auth::call(t, slot, pw, Method::GET, "/api/info/version", None).await {
        Ok(data) => Some(model::parse_version(&data)),
        Err(f) if f.kind == PiholeErrorKind::Other => None,
        Err(f) => return Err(f),
    };
    Ok(Reading { summary, blocking, lists, lists_error, version })
}

/// The body for POST /api/dns/blocking: on for good, or off for `timer` seconds (None: until switched on).
pub fn blocking_body(blocking: bool, timer: Option<u32>) -> std::result::Result<serde_json::Value, String> {
    if blocking {
        // A timer here would switch blocking off again later, which the screen never wants.
        return Ok(json!({ "blocking": true, "timer": null }));
    }
    match timer {
        Some(0) => Err("A pause needs at least one second.".into()),
        Some(t) if t > MAX_PAUSE_SECS => Err("A pause can be at most a day. To keep blocking off for longer, choose \"Until I turn it back on\".".into()),
        Some(t) => Ok(json!({ "blocking": false, "timer": t })),
        None => Ok(json!({ "blocking": false, "timer": null })),
    }
}

impl PiHole {
    fn slot(&self, device_id: i64) -> Arc<tokio::sync::Mutex<Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner()).entry(device_id).or_default().clone()
    }

    fn offered(&self) -> std::sync::MutexGuard<'_, HashMap<i64, String>> {
        self.offered.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The device and its settings, or an error if it isn't a Pi-hole.
    fn config(&self, inv: &Inventory, device_id: i64) -> Result<(Device, HostConfig)> {
        let device = inv.get(device_id)?;
        let Some(config) = inv.pihole_host(device_id)? else {
            return Err(Error::Invalid(format!("\"{}\" isn't set up as a Pi-hole. Add it on the Pi-hole screen.", device.name)));
        };
        Ok((device, config))
    }

    async fn prepare(&self, inv: &Inventory, device_id: i64) -> Result<(HostConfig, std::result::Result<(Endpoint, Password), Failure>)> {
        let (device, config) = self.config(inv, device_id)?;
        let key = password_key(device_id);
        let password = tauri::async_runtime::spawn_blocking(move || secrets::get(&key))
            .await
            .map_err(|e| Error::Invalid(format!("Couldn't read the keychain: {e}")))??;
        let ready = endpoint(&device, &config, password);
        Ok((config, ready))
    }

    /// Notes a certificate that waits for the owner's approval; forgets it once calls work.
    fn note(&self, device_id: i64, outcome: std::result::Result<(), &Failure>) {
        match outcome {
            Ok(()) => {
                self.offered().remove(&device_id);
            }
            Err(f) => {
                if let Some(offered) = &f.offered {
                    self.offered().insert(device_id, offered.clone());
                }
            }
        }
    }

    pub fn settings_of(&self, device_id: i64, config: &HostConfig) -> Result<PiholeSettings> {
        Ok(PiholeSettings {
            device_id,
            https: config.scheme == Scheme::Https,
            port: config.port,
            pinned_fingerprint: config.certificate.clone(),
            has_password: secrets::has(&password_key(device_id))?,
        })
    }

    /// Every device marked as a Pi-hole.
    pub fn hosts(&self, inv: &Inventory) -> Result<Vec<PiholeSettings>> {
        inv.pihole_hosts()?.into_iter().map(|(id, c)| self.settings_of(id, &c)).collect()
    }

    pub fn add(&self, inv: &Inventory, device_id: i64) -> Result<PiholeSettings> {
        inv.get(device_id)?;
        inv.add_pihole(device_id)?;
        let (_, config) = self.config(inv, device_id)?;
        self.settings_of(device_id, &config)
    }

    /// Ends the session and forgets everything about the Pi-hole: its row and its password. Done while
    /// holding the Pi-hole's slot, so a reading waiting behind it can't log in again with what is forgotten.
    pub async fn remove(&self, inv: &Inventory, device_id: i64) -> Result<()> {
        let slot = self.slot(device_id);
        let mut held = slot.lock().await;
        end(&mut held).await;
        self.offered().remove(&device_id);
        inv.remove_pihole(device_id)?;
        secrets::delete(&password_key(device_id))?;
        Ok(())
    }

    /// Saves HTTPS or HTTP, the port and, when one is typed, the password (into the keychain). The session
    /// made with the old settings is ended first. The Pi-hole's slot is held throughout, so a reading that
    /// was waiting reads the new settings and password, never the old ones.
    pub async fn save(&self, inv: &Inventory, device_id: i64, https: bool, port: u16, password: Option<String>) -> Result<PiholeSettings> {
        check_port(port).map_err(Error::Invalid)?;
        let slot = self.slot(device_id);
        let mut held = slot.lock().await;
        let (_, before) = self.config(inv, device_id)?;
        let scheme = if https { Scheme::Https } else { Scheme::Http };
        let password = password.filter(|p| !p.is_empty());
        if before.scheme != scheme || before.port != port || password.is_some() {
            end(&mut held).await;
        }
        inv.save_pihole(device_id, scheme, port)?;
        if before.scheme != scheme || before.port != port {
            // What was shown at the old place can't be approved for the new one.
            self.offered().remove(&device_id);
        }
        if let Some(p) = password {
            let key = password_key(device_id);
            tauri::async_runtime::spawn_blocking(move || secrets::set(&key, &p))
                .await
                .map_err(|e| Error::Invalid(format!("Couldn't reach the keychain: {e}")))??;
        }
        let (_, config) = self.config(inv, device_id)?;
        self.settings_of(device_id, &config)
    }

    pub async fn remove_password(&self, inv: &Inventory, device_id: i64) -> Result<PiholeSettings> {
        let (_, config) = self.config(inv, device_id)?;
        let slot = self.slot(device_id);
        let mut held = slot.lock().await;
        end(&mut held).await;
        secrets::delete(&password_key(device_id))?;
        self.settings_of(device_id, &config)
    }

    /// The device got another address: whatever answers there is checked as at first contact. The old
    /// session is ended at the old address.
    pub fn address_changed(&self, inv: &Inventory, device_id: i64) -> Result<()> {
        self.offered().remove(&device_id);
        self.logout_later(device_id);
        inv.forget_pihole_certificate(device_id)
    }

    /// The device is being removed: end its session, in the background.
    pub fn device_removed(&self, device_id: i64) {
        self.offered().remove(&device_id);
        self.logout_later(device_id);
    }

    fn logout_later(&self, device_id: i64) {
        let slot = self.slot(device_id);
        tauri::async_runtime::spawn(async move {
            let mut held = slot.lock().await;
            end(&mut held).await;
        });
    }

    /// Ends every session, for when the app closes. Gives up after a few seconds: they end by themselves.
    pub async fn logout_all(&self) {
        let ids: Vec<i64> = self.slots.lock().unwrap_or_else(|e| e.into_inner()).keys().copied().collect();
        let all = async {
            let mut set = tokio::task::JoinSet::new();
            for id in ids {
                let slot = self.slot(id);
                set.spawn(async move {
                    let mut held = slot.lock().await;
                    end(&mut held).await;
                });
            }
            while set.join_next().await.is_some() {}
        };
        tokio::time::timeout(Duration::from_secs(3), all).await.ok();
    }

    /// Reads one Pi-hole. The settings and password are read once the Pi-hole's slot is held, so a reading
    /// that waited behind a save uses what was saved.
    pub async fn status(&self, inv: &Inventory, device_id: i64) -> Result<PiholeStatus> {
        let slot = self.slot(device_id);
        let mut held = slot.lock().await;
        let (config, ready) = self.prepare(inv, device_id).await?;
        let mut status = PiholeStatus {
            device_id,
            reading: None,
            fetched_at: None,
            error: None,
            error_kind: None,
            offered_fingerprint: None,
            expected_fingerprint: None,
            pinned_fingerprint: config.certificate.clone(),
            https: config.scheme == Scheme::Https,
            port: config.port,
        };
        let result = match ready {
            Err(f) => Err(f),
            Ok((ep, pw)) => {
                end_if_elsewhere(&mut held, &ep).await;
                let r = read_all(&ep, &mut held.session, &pw).await;
                if held.session.is_some() {
                    held.ep = Some(ep);
                }
                self.note(device_id, r.as_ref().map(|_| ()));
                r
            }
        };
        match result {
            Ok(reading) => {
                status.reading = Some(reading);
                status.fetched_at = Some(now());
            }
            Err(f) => {
                status.error = Some(f.message);
                status.error_kind = Some(f.kind);
                status.offered_fingerprint = f.offered;
                status.expected_fingerprint = f.expected;
            }
        }
        Ok(status)
    }

    /// Switches blocking on, or off for `timer` seconds (None: until switched on again).
    pub async fn set_blocking(&self, inv: &Inventory, device_id: i64, blocking: bool, timer: Option<u32>) -> Result<Blocking> {
        let body = blocking_body(blocking, timer).map_err(Error::Invalid)?;
        let slot = self.slot(device_id);
        let mut held = slot.lock().await;
        let (_, ready) = self.prepare(inv, device_id).await?;
        let (ep, pw) = ready.map_err(|f| Error::Network(f.message))?;
        end_if_elsewhere(&mut held, &ep).await;
        let r = auth::call(&ep, &mut held.session, &pw, Method::POST, "/api/dns/blocking", Some(&body)).await;
        if held.session.is_some() {
            held.ep = Some(ep);
        }
        self.note(device_id, r.as_ref().map(|_| ()));
        let data = r.map_err(|f| Error::Network(f.message))?;
        model::parse_blocking(&data).map_err(Error::Network)
    }

    /// The owner checked the certificate a Pi-hole showed and approves it.
    pub fn trust_certificate(&self, inv: &Inventory, device_id: i64, fingerprint: &str) -> Result<()> {
        self.config(inv, device_id)?;
        let offered = self.offered().get(&device_id).cloned();
        let Some(offered) = offered.filter(|o| same_fingerprint(o, fingerprint)) else {
            return Err(Error::Invalid(
                "This Pi-hole has shown a different certificate since you looked, or none. Check the one shown now before you approve it.".into(),
            ));
        };
        inv.approve_pihole_certificate(device_id, &offered)?;
        self.offered().remove(&device_id);
        Ok(())
    }

    /// Looks for Pi-hole on saved devices, over plain HTTP on port 80 (Pi-hole v6 serves its API there too).
    /// Only asks whether the login page is there; nothing secret is sent.
    pub async fn probe(&self, inv: &Inventory, device_ids: &[i64]) -> Result<Vec<Probe>> {
        if device_ids.len() > 64 {
            return Err(Error::Invalid("home-cc looks for Pi-hole on at most 64 machines at a time.".into()));
        }
        let mut set = tokio::task::JoinSet::new();
        for &id in device_ids {
            let device = inv.get(id)?;
            if device.kind == DeviceKind::Ignore {
                continue;
            }
            let ep = Endpoint { host: device.host.trim().to_string(), port: HTTP_PORT, scheme: Scheme::Http, pinned: None };
            set.spawn(async move { Probe { device_id: id, found: probe_one(&ep).await } });
        }
        let mut found = Vec::new();
        while let Some(done) = set.join_next().await {
            if let Ok(p) = done {
                found.push(p);
            }
        }
        found.sort_by_key(|p| p.device_id);
        Ok(found)
    }
}

/// Logs out of the session held, at the place it was made.
async fn end(held: &mut Slot) {
    if let (Some(session), Some(ep)) = (held.session.take(), held.ep.take()) {
        auth::logout(&ep, &session).await;
    }
}

/// Whether the slot holds a session made somewhere other than `ep` (the address, scheme or port changed
/// since). Such a session is never sent to `ep`.
fn made_elsewhere(held: &Slot, ep: &Endpoint) -> bool {
    held.session.as_ref().is_some_and(|s| s.place != ep.key())
}

/// Before a call at `ep`: logs out of a session made elsewhere, at the place it was made, so it doesn't
/// hold one of that Pi-hole's few API seats until it times out.
async fn end_if_elsewhere(held: &mut Slot, ep: &Endpoint) {
    if made_elsewhere(held, ep) {
        end(held).await;
    }
}

/// "v6" if the Pi-hole v6 login answers, "v5" if the older API does.
async fn probe_one<T: Transport>(t: &T) -> Option<String> {
    let r = t.send(Method::GET, "/api/auth", None, None).await.ok()?;
    if model::parse_session(&r.body).is_some() {
        return Some("v6".into());
    }
    let r = t.send(Method::GET, "/admin/api.php?version", None, None).await.ok()?;
    (r.ok() && r.body.get("version").is_some()).then(|| "v5".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use auth::tests::{Fake, logged_in, ok, status, unauthorized};
    use client::{ApiError, Reply};
    use crate::inventory::{DeviceInput, Os, Source};

    fn device() -> Device {
        Device {
            id: 3,
            name: "Pi-hole 1".into(),
            host: " 192.168.68.31 ".into(),
            kind: DeviceKind::Container,
            os: Os::Linux,
            ssh_user: None,
            ssh_port: 22,
            notes: String::new(),
            mac: None,
            vendor: None,
            hostname: None,
            source: Source::Manual,
            reviewed: true,
            first_seen: 0,
            last_seen: None,
            group: None,
            os_source: None,
        }
    }

    #[test]
    fn endpoints_are_built_from_what_is_stored() {
        let d = device();
        let none = endpoint(&d, &HostConfig::default(), None).unwrap_err();
        assert_eq!(none.kind, PiholeErrorKind::NoPassword);
        assert_eq!(endpoint(&d, &HostConfig::default(), Some(String::new())).unwrap_err().kind, PiholeErrorKind::NoPassword);

        let https = HostConfig { scheme: Scheme::Https, port: 443, certificate: Some("AB".into()) };
        let (ep, pw) = endpoint(&d, &https, Some("pw".into())).unwrap();
        assert_eq!(ep, Endpoint { host: "192.168.68.31".into(), port: 443, scheme: Scheme::Https, pinned: Some("AB".into()) });
        assert_eq!(pw.0, "pw");
        // A certificate kept from HTTPS days means nothing over HTTP.
        let http = HostConfig { scheme: Scheme::Http, port: 80, certificate: Some("AB".into()) };
        let (ep, _) = endpoint(&d, &http, Some("pw".into())).unwrap();
        assert_eq!((ep.pinned.as_deref(), ep.key()), (None, "http://192.168.68.31:80".to_string()));
    }

    #[test]
    fn ports_and_pause_lengths_are_checked() {
        assert!(check_port(0).is_err());
        assert!(check_port(443).is_ok());
        assert_eq!(blocking_body(true, Some(300)).unwrap(), json!({"blocking": true, "timer": null}));
        assert_eq!(blocking_body(false, Some(300)).unwrap(), json!({"blocking": false, "timer": 300}));
        assert_eq!(blocking_body(false, None).unwrap(), json!({"blocking": false, "timer": null}));
        assert!(blocking_body(false, Some(0)).is_err());
        assert!(blocking_body(false, Some(MAX_PAUSE_SECS + 1)).unwrap_err().contains("Until I turn it back on"));
    }

    fn summary() -> serde_json::Value {
        serde_json::from_str(include_str!("testdata/summary.json")).unwrap()
    }

    #[tokio::test]
    async fn a_reading_uses_one_session_for_every_part() {
        let fake = Fake::new(vec![
            logged_in("sid-1"),
            ok(summary()),
            ok(json!({"blocking": "disabled", "timer": 120.5})),
            ok(serde_json::from_str(include_str!("testdata/lists.json")).unwrap()),
            ok(serde_json::from_str(include_str!("testdata/version.json")).unwrap()),
        ]);
        let mut slot = None;
        let r = read_all(&fake, &mut slot, &Password("pw".into())).await.unwrap();
        assert_eq!(r.summary.queries, 48213);
        assert_eq!(r.blocking.timer_secs, Some(120.5));
        assert_eq!(r.lists.as_ref().map(Vec::len), Some(3));
        assert_eq!(r.version.and_then(|v| v.core).as_deref(), Some("v6.1.4"));
        let paths: Vec<String> = fake.asked().into_iter().map(|(_, p, _)| p).collect();
        assert_eq!(paths, ["/api/auth", "/api/stats/summary", "/api/dns/blocking", "/api/lists", "/api/info/version"]);
    }

    #[tokio::test]
    async fn lists_that_fail_leave_the_rest() {
        let fake = Fake::new(vec![
            logged_in("sid-1"),
            ok(summary()),
            ok(json!({"blocking": "enabled", "timer": null})),
            status(500, json!({"error": {"key": "database_error", "message": "Could not read lists", "hint": null}})),
            status(404, json!({})),
        ]);
        let mut slot = None;
        let r = read_all(&fake, &mut slot, &Password("pw".into())).await.unwrap();
        assert_eq!(r.lists, None);
        assert!(r.lists_error.unwrap().contains("Could not read lists"));
        assert_eq!(r.version, None);

        // But a session that can't be renewed stops the whole reading.
        let fake = Fake::new(vec![logged_in("sid-1"), ok(summary()), ok(json!({"blocking": "enabled"})), unauthorized(), unauthorized()]);
        let mut slot = None;
        let err = read_all(&fake, &mut slot, &Password("pw".into())).await.unwrap_err();
        assert_eq!(err.kind, PiholeErrorKind::Auth);
    }

    #[test]
    fn a_session_from_an_old_address_scheme_or_port_is_recognised() {
        let old = Endpoint { host: "192.168.68.31".into(), port: 443, scheme: Scheme::Https, pinned: None };
        let session = Session {
            place: old.key(),
            sid: Some("sid-1".into()),
            lifetime: Duration::from_secs(1800),
            until: std::time::Instant::now() + Duration::from_secs(600),
        };
        let held = Slot { session: Some(session), ep: Some(old.clone()) };
        assert!(!made_elsewhere(&held, &old));
        assert!(!made_elsewhere(&held, &Endpoint { pinned: Some("AB".into()), ..old.clone() }), "approving a certificate keeps the place");
        assert!(made_elsewhere(&held, &Endpoint { port: 8443, ..old.clone() }));
        assert!(made_elsewhere(&held, &Endpoint { scheme: Scheme::Http, port: 80, ..old.clone() }));
        assert!(made_elsewhere(&held, &Endpoint { host: "192.168.68.40".into(), ..old.clone() }));
        assert!(!made_elsewhere(&Slot::default(), &old), "nothing to end");
    }

    #[tokio::test]
    async fn looking_for_pi_holes() {
        let v6 = Fake::new(vec![status(401, json!({"session": {"valid": false, "totp": false, "sid": null, "validity": -1}}))]);
        assert_eq!(probe_one(&v6).await.as_deref(), Some("v6"));
        let v5 = Fake::new(vec![Ok(Reply { status: 404, body: serde_json::Value::Null, json: false, location: None }), ok(json!({"version": 3}))]);
        assert_eq!(probe_one(&v5).await.as_deref(), Some("v5"));
        let nothing = Fake::new(vec![Err(ApiError::Unreachable("refused".into()))]);
        assert_eq!(probe_one(&nothing).await, None);
    }

    fn input() -> DeviceInput {
        DeviceInput {
            name: "Pi-hole 1".into(),
            host: "192.168.68.31".into(),
            kind: DeviceKind::Container,
            os: Os::Linux,
            ssh_user: None,
            ssh_port: None,
            notes: String::new(),
            mac: None,
            group: None,
            os_was: None,
            os_auto: false,
        }
    }

    #[test]
    fn an_approval_must_match_what_was_shown() {
        let inv = Inventory::in_memory().unwrap();
        inv.pihole_init().unwrap();
        let id = inv.add(input()).unwrap().id;
        let ph = PiHole::default();
        let shown = "EC:8E:F3:EE:05:91:6C:C0:58:72:9F:5C:73:34:77:8C:70:80:06:EE:49:B9:77:DA:EC:1C:7A:D0:97:1C:9A:93";
        // Not a Pi-hole yet: refused.
        assert!(ph.trust_certificate(&inv, id, shown).is_err());
        inv.add_pihole(id).unwrap();
        // Nothing shown yet: nothing to approve.
        assert!(ph.trust_certificate(&inv, id, shown).is_err());
        ph.offered().insert(id, shown.into());
        assert!(ph.trust_certificate(&inv, id, &shown.replace("EC:8E", "EC:8F")).is_err());
        assert_eq!(inv.pihole_host(id).unwrap().unwrap().certificate, None);
        ph.trust_certificate(&inv, id, &shown.to_lowercase()).unwrap();
        assert_eq!(inv.pihole_host(id).unwrap().unwrap().certificate.as_deref(), Some(shown));
        assert!(ph.trust_certificate(&inv, id, shown).is_err(), "used up");
    }

    #[test]
    fn readings_and_actions_need_a_pi_hole() {
        let inv = Inventory::in_memory().unwrap();
        inv.pihole_init().unwrap();
        let id = inv.add(input()).unwrap().id;
        let err = PiHole::default().config(&inv, id).unwrap_err();
        assert!(err.to_string().contains("isn't set up as a Pi-hole"));
    }
}
