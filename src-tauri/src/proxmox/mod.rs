//! Proxmox (spec 5.6): each node's CPU, RAM and storage, its VMs and
//! containers, starting and stopping them, and snapshots. Everything goes
//! through Proxmox VE's own HTTPS API with an API token the owner creates in
//! Proxmox; nothing is installed on the Proxmox host.
//!
//! - client.rs: the HTTPS calls, refused unless the host shows the certificate
//!   the owner approved (pin.rs).
//! - model.rs: reading Proxmox's answers.
//! - store.rs: the token ID, port and approved certificate, per device.
//! - commands.rs: what the Proxmox screen can ask for.
//!
//! The screen asks for a fresh overview every few seconds while it is open;
//! nothing runs in the background here.

pub mod client;
pub mod commands;
pub mod model;
pub mod pin;
pub mod store;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hyper::Method;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::inventory::{Device, Inventory, Os, now};
use crate::secrets;
use client::{ApiError, Endpoint, Token, encode};
use model::{GuestAddresses, GuestKind, Overview, SnapshotList, TaskState};
use pin::same_fingerprint;
use store::HostConfig;

/// Keychain entry for a Proxmox host's API token secret.
pub fn token_key(device_id: i64) -> String {
    format!("proxmox-token:{device_id}")
}

/// How long an action waits for Proxmox to finish it before saying it is
/// still going.
const TASK_WAIT: Duration = Duration::from_secs(45);
const TASK_POLL: Duration = Duration::from_millis(800);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProxmoxErrorKind {
    /// No token ID, or no secret stored for it.
    NoToken,
    /// First contact: the owner must approve the certificate. Nothing was sent.
    NewCert,
    /// The certificate isn't the approved one. Nothing was sent.
    CertChanged,
    /// The token wasn't accepted.
    Auth,
    /// The token lacks a permission.
    Permission,
    Unreachable,
    Other,
}

/// Why a call failed, worded for the owner.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub kind: ProxmoxErrorKind,
    pub message: String,
    /// With NewCert and CertChanged: the certificate the host showed.
    pub offered: Option<String>,
    /// With CertChanged: the one approved before.
    pub expected: Option<String>,
}

impl Failure {
    fn new(kind: ProxmoxErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), offered: None, expected: None }
    }
}

impl From<Failure> for Error {
    fn from(f: Failure) -> Self {
        Error::Network(f.message)
    }
}

/// One host's state, as the Proxmox screen sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxmoxStatus {
    pub device_id: i64,
    /// Set when the reading worked.
    pub overview: Option<Overview>,
    /// Unix seconds of the reading in `overview`.
    pub fetched_at: Option<i64>,
    pub error: Option<String>,
    pub error_kind: Option<ProxmoxErrorKind>,
    /// With newCert and certChanged: the certificate the host showed, for the owner to check.
    pub offered_fingerprint: Option<String>,
    /// With certChanged: the one approved before.
    pub expected_fingerprint: Option<String>,
    /// The certificate approved for this host, if any.
    pub pinned_fingerprint: Option<String>,
}

/// A host's settings as the screen sees them. The secret is never sent back,
/// only whether one is stored.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxmoxSettings {
    pub device_id: i64,
    pub token_id: Option<String>,
    pub port: u16,
    pub pinned_fingerprint: Option<String>,
    pub has_secret: bool,
}

/// Which VM or container an action is for.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestRef {
    pub node: String,
    pub kind: GuestKind,
    pub vmid: u32,
}

impl GuestRef {
    fn path(&self) -> Result<String> {
        model::check_node_name(&self.node).map_err(Error::Invalid)?;
        Ok(format!("/nodes/{}/{}/{}", encode(&self.node), self.kind.as_str(), self.vmid))
    }

    fn label(&self) -> String {
        match self.kind {
            GuestKind::Qemu => format!("VM {}", self.vmid),
            GuestKind::Lxc => format!("container {}", self.vmid),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PowerAction {
    /// Also resumes a hibernated VM: Proxmox lets a VM locked "suspended"
    /// start, and starting it is how it comes back.
    Start,
    /// Carries on with a paused VM or container.
    Resume,
    /// Asks the guest's own system to shut down.
    Shutdown,
    Reboot,
    /// Pulls the plug.
    Stop,
}

impl PowerAction {
    fn as_str(self) -> &'static str {
        match self {
            PowerAction::Start => "start",
            PowerAction::Resume => "resume",
            PowerAction::Shutdown => "shutdown",
            PowerAction::Reboot => "reboot",
            PowerAction::Stop => "stop",
        }
    }
}

/// How an action Proxmox runs in the background went.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskOutcome {
    /// False if it was still going when home-cc stopped waiting. It carries on
    /// in Proxmox, and the list shows the result when it is done.
    pub finished: bool,
}

/// Checks a token ID: "user@realm!name".
pub fn check_token_id(id: &str) -> std::result::Result<(), String> {
    let example = "It looks like user@realm!name, for example root@pam!homecc: the user, the realm, then the token's own name.";
    if id.contains('=') {
        return Err(format!(
            "Put only the token ID here, and the secret in its own box. {example}"
        ));
    }
    let Some((user, name)) = id.split_once('!') else {
        return Err(format!("The token ID needs a \"!\" before the token's name. {example}"));
    };
    let Some((who, realm)) = user.split_once('@') else {
        return Err(format!("The token ID needs the user's realm, such as @pam or @pve. {example}"));
    };
    let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '!' | '@' | '='));
    if !plain(who) || !plain(realm) || !plain(name) {
        return Err(format!("That token ID has a space or another character Proxmox doesn't use there. {example}"));
    }
    Ok(())
}

/// What to tell the owner to run on the Proxmox host to give `token` the
/// permissions home-cc uses.
fn grant_command(token_id: &str) -> String {
    format!("pveum acl modify / --tokens '{token_id}' --roles PVEAuditor,PVEVMAdmin")
}

/// Turns what went wrong in a call into words that say what to fix.
pub fn explain(e: ApiError, token_id: &str) -> Failure {
    use ProxmoxErrorKind as K;
    match e {
        ApiError::NewCert { offered } => Failure {
            kind: K::NewCert,
            message: "First connection to this Proxmox host. Check its certificate before home-cc sends it the API token.".into(),
            offered: Some(offered),
            expected: None,
        },
        ApiError::CertChanged { expected, offered } => Failure {
            kind: K::CertChanged,
            message: "This Proxmox host shows a different certificate from the one you approved, so home-cc sent it nothing. Check the new one before you approve it.".into(),
            offered: Some(offered),
            expected: Some(expected),
        },
        ApiError::Unreachable(m) => Failure::new(K::Unreachable, m),
        ApiError::Auth(why) => Failure::new(
            K::Auth,
            format!(
                "Proxmox didn't accept the API token {token_id} ({why}). Check the token ID and the secret: in Proxmox under \
                 Datacenter > Permissions > API Tokens, see that the token is there and hasn't expired. Proxmox shows the \
                 secret only once, when the token is made; if you no longer have it, remove the token there, add it again, \
                 and paste the new secret here."
            ),
        ),
        ApiError::Permission(why) => Failure::new(
            K::Permission,
            format!(
                "Proxmox refused: {why}. The token {token_id} is missing a permission. A token made with \"Privilege \
                 Separation\" ticked (the default) only gets the permissions given to the token itself, not its user's. \
                 Give it the PVEAuditor role (to see nodes, storage and guests) and PVEVMAdmin (to start, stop and take \
                 snapshots) on \"/\": in Proxmox under Datacenter > Permissions > Add > API Token Permission, or run this in \
                 the Proxmox host's shell:\n{}",
                grant_command(token_id)
            ),
        ),
        ApiError::Other(m) => Failure::new(K::Other, m),
    }
}

/// Builds the endpoint from what is stored. The secret is only read here, and
/// is only used once the certificate checks out (client.rs).
pub fn endpoint(device: &Device, config: &HostConfig, secret: Option<String>) -> std::result::Result<Endpoint, Failure> {
    let token_id = config.token_id.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let secret = secret.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let (id, secret) = match (token_id, secret) {
        (Some(id), Some(secret)) => (id.to_string(), secret),
        (None, _) => {
            return Err(Failure::new(
                ProxmoxErrorKind::NoToken,
                "Add an API token for this Proxmox host: its ID (such as root@pam!homecc) and its secret.",
            ));
        }
        (Some(_), None) => {
            return Err(Failure::new(
                ProxmoxErrorKind::NoToken,
                "The API token's secret isn't stored. Paste it into the secret box and save.",
            ));
        }
    };
    Ok(Endpoint {
        host: device.host.trim().to_string(),
        port: config.port,
        pinned: config.certificate.clone(),
        token: Token { id, secret },
    })
}

#[derive(Default)]
pub struct Proxmox {
    /// The certificate each host showed that the owner hasn't approved yet,
    /// so an approval can be checked against what was really shown.
    offered: Mutex<HashMap<i64, String>>,
}

impl Proxmox {
    /// The device, its settings and a ready endpoint, or why there is none.
    async fn prepare(&self, inv: &Inventory, device_id: i64) -> Result<(Device, HostConfig, std::result::Result<Endpoint, Failure>)> {
        let device = inv.get(device_id)?;
        if device.os != Os::Proxmox {
            return Err(Error::Invalid(format!(
                "\"{}\" isn't set up as a Proxmox host. On the Machines screen, set its System to Proxmox VE.",
                device.name
            )));
        }
        let config = inv.proxmox_host(device_id)?;
        let key = token_key(device_id);
        let secret = tauri::async_runtime::spawn_blocking(move || secrets::get(&key))
            .await
            .map_err(|e| Error::Invalid(format!("Couldn't read the keychain: {e}")))??;
        let ep = endpoint(&device, &config, secret);
        Ok((device, config, ep))
    }

    /// Makes one call and words any failure. Notes a certificate that waits
    /// for the owner's approval.
    async fn call(
        &self,
        device_id: i64,
        ep: &Endpoint,
        method: Method,
        path: &str,
        form: &[(&str, &str)],
    ) -> std::result::Result<serde_json::Value, Failure> {
        match client::call(ep, method, path, form).await {
            Ok(data) => {
                self.lock().remove(&device_id);
                Ok(data)
            }
            Err(e) => {
                let failure = explain(e, &ep.token.id);
                if let Some(offered) = &failure.offered {
                    self.lock().insert(device_id, offered.clone());
                }
                Err(failure)
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<i64, String>> {
        self.offered.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn settings(&self, inv: &Inventory, device_id: i64) -> Result<ProxmoxSettings> {
        inv.get(device_id)?;
        let config = inv.proxmox_host(device_id)?;
        Ok(ProxmoxSettings {
            device_id,
            token_id: config.token_id,
            port: config.port,
            pinned_fingerprint: config.certificate,
            has_secret: secrets::has(&token_key(device_id))?,
        })
    }

    pub fn save_settings(&self, inv: &Inventory, device_id: i64, token_id: &str, port: u16) -> Result<ProxmoxSettings> {
        self.store_settings(inv, device_id, token_id, port)?;
        self.settings(inv, device_id)
    }

    /// save_settings without reading them back (which asks the keychain).
    fn store_settings(&self, inv: &Inventory, device_id: i64, token_id: &str, port: u16) -> Result<()> {
        let token_id = token_id.trim();
        if !token_id.is_empty() {
            check_token_id(token_id).map_err(Error::Invalid)?;
        }
        if port == 0 {
            return Err(Error::Invalid("The port must be between 1 and 65535. Proxmox uses 8006.".into()));
        }
        let before = inv.proxmox_host(device_id)?;
        // A new port drops the approved certificate (store.rs), and what the
        // old port showed can't be approved for the new one either.
        inv.save_proxmox_host(device_id, Some(token_id).filter(|t| !t.is_empty()), port)?;
        if before.port != port {
            self.lock().remove(&device_id);
        }
        Ok(())
    }

    /// The owner gave the device another address: whatever answers there is
    /// another server as far as home-cc knows, so its certificate is checked
    /// as at first contact, not reported as changed. The same rule as for an
    /// SSH server's identity (Inventory::update).
    pub fn address_changed(&self, inv: &Inventory, device_id: i64) -> Result<()> {
        self.lock().remove(&device_id);
        inv.forget_proxmox_certificate(device_id)
    }

    /// Reads every node, VM, container and storage the token may see.
    pub async fn status(&self, inv: &Inventory, device_id: i64) -> Result<ProxmoxStatus> {
        let (_, config, ep) = self.prepare(inv, device_id).await?;
        let mut status = ProxmoxStatus {
            device_id,
            overview: None,
            fetched_at: None,
            error: None,
            error_kind: None,
            offered_fingerprint: None,
            expected_fingerprint: None,
            pinned_fingerprint: config.certificate.clone(),
        };
        let result = match ep {
            Err(f) => Err(f),
            Ok(ep) => match self.call(device_id, &ep, Method::GET, "/cluster/resources", &[]).await {
                Ok(data) => model::parse_resources(&data).map_err(|m| Failure::new(ProxmoxErrorKind::Other, m)),
                Err(f) => Err(f),
            },
        };
        match result {
            Ok(overview) => {
                // Its API answering shows it is a Proxmox host: a System only
                // guessed from the network counts as found now.
                if let Err(e) = inv.learn_os(device_id, Os::Proxmox, crate::inventory::OsSource::Proxmox) {
                    crate::diag::note(format!("could not store that device {device_id} is a Proxmox host: {e}"));
                }
                status.overview = Some(overview);
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

    /// The owner checked the certificate a host showed and approves it.
    pub fn trust_certificate(&self, inv: &Inventory, device_id: i64, fingerprint: &str) -> Result<()> {
        let offered = self.lock().get(&device_id).cloned();
        let Some(offered) = offered.filter(|o| same_fingerprint(o, fingerprint)) else {
            return Err(Error::Invalid(
                "This Proxmox host has shown a different certificate since you looked, or none. Check the one shown now before you approve it.".into(),
            ));
        };
        inv.approve_proxmox_certificate(device_id, &offered)?;
        self.lock().remove(&device_id);
        Ok(())
    }

    /// Ready to call, or the reason why not as an error for the screen.
    async fn ready(&self, inv: &Inventory, device_id: i64) -> Result<Endpoint> {
        let (_, _, ep) = self.prepare(inv, device_id).await?;
        Ok(ep?)
    }

    /// Asks Proxmox for a change it carries out as a background task, then
    /// waits a while for that task to end.
    async fn run_task(
        &self,
        device_id: i64,
        ep: &Endpoint,
        (method, path, form): (Method, &str, &[(&str, &str)]),
        node: &str,
        what: &str,
    ) -> Result<TaskOutcome> {
        let data = self.call(device_id, ep, method, path, form).await?;
        let Some(upid) = model::task_id(&data) else {
            // Done straight away.
            return Ok(TaskOutcome { finished: true });
        };
        let status_path = format!("/nodes/{}/tasks/{}/status", encode(node), encode(&upid));
        let until = Instant::now() + TASK_WAIT;
        loop {
            tokio::time::sleep(TASK_POLL).await;
            match self.call(device_id, ep, Method::GET, &status_path, &[]).await.map(|d| model::parse_task(&d)) {
                Ok(TaskState::Done) => return Ok(TaskOutcome { finished: true }),
                Ok(TaskState::Failed(why)) => {
                    return Err(Error::Network(format!("Proxmox tried to {what}, but it failed: {why}")));
                }
                Ok(TaskState::Running) => {}
                // The task did start; losing sight of it isn't its failure.
                Err(_) => return Ok(TaskOutcome { finished: false }),
            }
            if Instant::now() >= until {
                return Ok(TaskOutcome { finished: false });
            }
        }
    }

    pub async fn power(&self, inv: &Inventory, device_id: i64, guest: &GuestRef, action: PowerAction) -> Result<TaskOutcome> {
        let path = format!("{}/status/{}", guest.path()?, action.as_str());
        let ep = self.ready(inv, device_id).await?;
        let verb = match action {
            PowerAction::Start => "start",
            PowerAction::Resume => "resume",
            PowerAction::Shutdown => "shut down",
            PowerAction::Reboot => "restart",
            PowerAction::Stop => "force stop",
        };
        let what = format!("{verb} {}", guest.label());
        self.run_task(device_id, &ep, (Method::POST, &path, &[]), &guest.node, &what).await
    }

    /// A running guest's IPv4 addresses: a VM's from its QEMU guest agent, a
    /// container's from Proxmox itself. A VM without the agent running isn't an
    /// error: `unknown_why` says so.
    pub async fn guest_addresses(&self, inv: &Inventory, device_id: i64, guest: &GuestRef) -> Result<GuestAddresses> {
        let (path, read): (String, fn(&serde_json::Value) -> Vec<String>) = match guest.kind {
            GuestKind::Qemu => (format!("{}/agent/network-get-interfaces", guest.path()?), model::parse_agent_addresses),
            GuestKind::Lxc => (format!("{}/interfaces", guest.path()?), model::parse_lxc_addresses),
        };
        let ep = self.ready(inv, device_id).await?;
        match self.call(device_id, &ep, Method::GET, &path, &[]).await {
            Ok(data) => Ok(GuestAddresses { addresses: read(&data), unknown_why: None }),
            Err(f) if f.kind == ProxmoxErrorKind::Other && guest.kind == GuestKind::Qemu && model::no_guest_agent(&f.message) => {
                Ok(GuestAddresses {
                    addresses: Vec::new(),
                    unknown_why: Some(
                        "Proxmox can't tell this VM's address: the QEMU guest agent isn't running in it.".into(),
                    ),
                })
            }
            Err(f) => Err(f.into()),
        }
    }

    pub async fn snapshots(&self, inv: &Inventory, device_id: i64, guest: &GuestRef) -> Result<SnapshotList> {
        let path = format!("{}/snapshot", guest.path()?);
        let ep = self.ready(inv, device_id).await?;
        let data = self.call(device_id, &ep, Method::GET, &path, &[]).await?;
        model::parse_snapshots(&data).map_err(Error::Network)
    }

    pub async fn take_snapshot(
        &self,
        inv: &Inventory,
        device_id: i64,
        guest: &GuestRef,
        name: &str,
        description: &str,
        with_ram: bool,
    ) -> Result<TaskOutcome> {
        let name = name.trim();
        model::check_snapshot_name(name).map_err(Error::Invalid)?;
        let path = format!("{}/snapshot", guest.path()?);
        let ep = self.ready(inv, device_id).await?;
        let mut form = vec![("snapname", name), ("description", description.trim())];
        // Only VMs can keep their memory in a snapshot.
        if with_ram && guest.kind == GuestKind::Qemu {
            form.push(("vmstate", "1"));
        }
        let what = format!("take snapshot \"{name}\" of {}", guest.label());
        self.run_task(device_id, &ep, (Method::POST, &path, &form), &guest.node, &what).await
    }

    /// Rolls a guest back to a snapshot. Proxmox stops a running guest to
    /// roll it back to a snapshot without RAM and leaves it stopped, so with
    /// `start_after` it is started again once the rollback is done. That is
    /// done here as a second step, not with the rollback's own "start"
    /// option, which older Proxmox versions don't have.
    pub async fn rollback(
        &self,
        inv: &Inventory,
        device_id: i64,
        guest: &GuestRef,
        name: &str,
        start_after: bool,
    ) -> Result<TaskOutcome> {
        model::check_snapshot_name(name).map_err(Error::Invalid)?;
        let path = format!("{}/snapshot/{}/rollback", guest.path()?, encode(name));
        let ep = self.ready(inv, device_id).await?;
        let what = format!("roll {} back to \"{name}\"", guest.label());
        let rolled = self.run_task(device_id, &ep, (Method::POST, &path, &[]), &guest.node, &what).await?;
        // Still rolling back: it can't be started yet. The screen says to start it once it's done.
        if !start_after || !rolled.finished {
            return Ok(rolled);
        }
        let start = format!("{}/status/start", guest.path()?);
        let what = format!("start {} again after rolling it back to \"{name}\"", guest.label());
        self.run_task(device_id, &ep, (Method::POST, &start, &[]), &guest.node, &what).await
    }

    pub async fn delete_snapshot(&self, inv: &Inventory, device_id: i64, guest: &GuestRef, name: &str) -> Result<TaskOutcome> {
        model::check_snapshot_name(name).map_err(Error::Invalid)?;
        let path = format!("{}/snapshot/{}", guest.path()?, encode(name));
        let ep = self.ready(inv, device_id).await?;
        let what = format!("delete snapshot \"{name}\" of {}", guest.label());
        self.run_task(device_id, &ep, (Method::DELETE, &path, &[]), &guest.node, &what).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{DeviceKind, Source};

    fn device() -> Device {
        Device {
            id: 7,
            name: "Proxmox".into(),
            host: " 192.168.68.30 ".into(),
            kind: DeviceKind::Server,
            os: Os::Proxmox,
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

    fn config(token: Option<&str>, cert: Option<&str>) -> HostConfig {
        HostConfig { token_id: token.map(Into::into), port: 8006, certificate: cert.map(Into::into) }
    }

    #[test]
    fn token_ids_are_checked() {
        for good in ["root@pam!homecc", "homecc@pve!screen-1", "jim@ldap.lan!a_b"] {
            assert!(check_token_id(good).is_ok(), "{good}");
        }
        let said = |id: &str| check_token_id(id).unwrap_err();
        assert!(said("root@pam!homecc=0b5c-…").contains("secret in its own box"));
        assert!(said("root@pam").contains("\"!\""));
        assert!(said("root!homecc").contains("realm"));
        assert!(said("root@pam!").contains("character"));
        assert!(said("root @pam!homecc").contains("character"));
    }

    #[test]
    fn no_token_says_what_is_missing() {
        let d = device();
        let none = endpoint(&d, &config(None, None), Some("s".into())).unwrap_err();
        assert_eq!(none.kind, ProxmoxErrorKind::NoToken);
        assert!(none.message.contains("Add an API token"));
        let no_secret = endpoint(&d, &config(Some("root@pam!homecc"), None), Some("  ".into())).unwrap_err();
        assert_eq!(no_secret.kind, ProxmoxErrorKind::NoToken);
        assert!(no_secret.message.contains("secret isn't stored"));

        let ep = endpoint(&d, &config(Some("root@pam!homecc"), Some("AB")), Some(" s3cret\n".into())).unwrap();
        assert_eq!((ep.host.as_str(), ep.port, ep.pinned.as_deref()), ("192.168.68.30", 8006, Some("AB")));
        assert_eq!(ep.token.secret, "s3cret");
    }

    #[test]
    fn failures_say_what_to_fix() {
        let t = "root@pam!homecc";
        let new = explain(ApiError::NewCert { offered: "AA".into() }, t);
        assert_eq!((new.kind, new.offered.as_deref(), new.expected.as_deref()), (ProxmoxErrorKind::NewCert, Some("AA"), None));
        let changed = explain(ApiError::CertChanged { expected: "AA".into(), offered: "BB".into() }, t);
        assert_eq!((changed.kind, changed.offered.as_deref(), changed.expected.as_deref()), (ProxmoxErrorKind::CertChanged, Some("BB"), Some("AA")));
        assert!(changed.message.contains("sent it nothing"));

        let auth = explain(ApiError::Auth("authentication failure (HTTP 401)".into()), t);
        assert_eq!(auth.kind, ProxmoxErrorKind::Auth);
        assert!(auth.message.contains("API Tokens") && auth.message.contains(t));

        let perm = explain(ApiError::Permission("Permission check failed (/vms/100, VM.PowerMgmt) (HTTP 403)".into()), t);
        assert_eq!(perm.kind, ProxmoxErrorKind::Permission);
        for needed in ["VM.PowerMgmt", "Privilege", "PVEAuditor", "PVEVMAdmin", "pveum acl modify / --tokens 'root@pam!homecc'"] {
            assert!(perm.message.contains(needed), "{needed} in {}", perm.message);
        }
    }

    fn input(host: &str) -> crate::inventory::DeviceInput {
        crate::inventory::DeviceInput {
            name: "Proxmox".into(),
            host: host.into(),
            kind: DeviceKind::Server,
            os: Os::Proxmox,
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
        inv.proxmox_init().unwrap();
        let id = inv.add(input("192.168.68.30")).unwrap().id;
        let px = Proxmox::default();
        let shown = "EC:8E:F3:EE:05:91:6C:C0:58:72:9F:5C:73:34:77:8C:70:80:06:EE:49:B9:77:DA:EC:1C:7A:D0:97:1C:9A:93";
        // Nothing shown yet: nothing to approve.
        assert!(px.trust_certificate(&inv, id, shown).is_err());
        px.lock().insert(id, shown.into());
        // Another certificate than the one shown is refused.
        let other = shown.replace("EC:8E", "EC:8F");
        assert!(px.trust_certificate(&inv, id, &other).is_err());
        assert_eq!(inv.proxmox_host(id).unwrap().certificate, None);
        px.trust_certificate(&inv, id, &shown.to_lowercase()).unwrap();
        assert_eq!(inv.proxmox_host(id).unwrap().certificate.as_deref(), Some(shown));
        // Used up: approving again needs a new showing.
        assert!(px.trust_certificate(&inv, id, shown).is_err());
    }

    #[test]
    fn another_address_or_port_is_a_first_contact_again() {
        let inv = Inventory::in_memory().unwrap();
        inv.proxmox_init().unwrap();
        let id = inv.add(input("192.168.68.30")).unwrap().id;
        let px = Proxmox::default();
        let shown = "EC:8E:F3:EE:05:91:6C:C0:58:72:9F:5C:73:34:77:8C:70:80:06:EE:49:B9:77:DA:EC:1C:7A:D0:97:1C:9A:93";

        // A certificate shown at the old address can't be approved for the new one.
        px.lock().insert(id, shown.into());
        inv.approve_proxmox_certificate(id, "AB:CD").unwrap();
        px.address_changed(&inv, id).unwrap();
        assert_eq!(inv.proxmox_host(id).unwrap().certificate, None);
        assert!(px.trust_certificate(&inv, id, shown).is_err());

        // Saving the same port keeps the approval and what is waiting for it; another port drops both.
        inv.approve_proxmox_certificate(id, "AB:CD").unwrap();
        px.lock().insert(id, shown.into());
        px.store_settings(&inv, id, "root@pam!homecc", 8006).unwrap();
        assert_eq!(inv.proxmox_host(id).unwrap().certificate.as_deref(), Some("AB:CD"));
        assert!(px.lock().contains_key(&id));
        px.store_settings(&inv, id, "root@pam!homecc", 8443).unwrap();
        assert_eq!(inv.proxmox_host(id).unwrap().certificate, None);
        assert!(!px.lock().contains_key(&id));
    }
}
