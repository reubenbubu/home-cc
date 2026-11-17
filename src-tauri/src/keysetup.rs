//! Key login ("passwordless" SSH) for a machine, like ssh-copy-id built into the app.
//!
//! - [`local_ssh_key`] says which key this PC logs in with (the file chosen in Settings,
//!   or the usual ones in the .ssh folder) and gives its public half, to add by hand.
//! - [`create_ssh_key`] makes an ed25519 key when this PC has none. It never overwrites
//!   a file.
//! - [`setup_key_login`] logs in to a machine once, with the password if it has to,
//!   adds this PC's public key to the right file there without doubling a line, and
//!   then tries a login with keys only, so the owner knows straight away if it worked.
//!
//! The password rules are the ones in `ssh.rs` and `monitor.rs`: a password, typed or
//! stored, is only ever sent to a machine whose identity the owner has approved. If the
//! identity isn't approved yet, nothing is sent and the screen is handed the identity to
//! approve ([`KeySetupOutcome::Approve`], then [`approve_key_setup_identity`]).
//!
//! Nothing here logs or returns a password, a passphrase or a private key.

use crate::error::{Error, Result};
use crate::inventory::{Inventory, Os};
use crate::monitor::{self, Monitor, OfferedKey};
use crate::secrets;
use crate::ssh::{self, HostKey, Login, SshError};
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, HashAlg, PrivateKey, PublicKey};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

/// How long the command that adds the key may take. PowerShell can take a while to start
/// on a slow Windows machine.
const SCRIPT_LIMIT: Duration = Duration::from_secs(45);
/// Public key files are one line. Anything much larger was picked by mistake.
const MAX_PUBLIC_FILE: u64 = 16 * 1024;
/// Private keys are a few kilobytes (same limit as `ssh.rs`).
const MAX_PRIVATE_FILE: u64 = 256 * 1024;
/// The longest key line accepted. An 8192-bit RSA key is about 1,400 characters.
const MAX_KEY_LINE: usize = 2000;
/// cmd.exe, the usual shell of Windows' SSH server, refuses longer command lines.
const CMD_LIMIT: usize = 8191;
/// The longest comment kept on a key line.
const MAX_COMMENT: usize = 48;
/// Name of a new key, in the .ssh folder.
const NEW_KEY: &str = "id_ed25519";

/// Which key this PC logs in with, as the screens show it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalKey {
    /// Full path of the private key file. None when this PC has no key.
    pub path: Option<String>,
    /// What to call it: the file name, or the path chosen in Settings.
    pub name: Option<String>,
    /// True if the key file is the one chosen in Settings.
    pub from_settings: bool,
    /// The line that goes in authorized_keys on a machine. None if it couldn't be read.
    pub public_key: Option<String>,
    /// For example "ssh-ed25519".
    pub algorithm: Option<String>,
    /// "SHA256:…", as `ssh-keygen -lf` prints it.
    pub fingerprint: Option<String>,
    /// Why the public key couldn't be read, or why the chosen key file is missing.
    pub problem: Option<String>,
    /// Something worth knowing that is not a problem, such as a .pub file that didn't match.
    pub note: Option<String>,
    /// Other key files in the .ssh folder that logins offer as well.
    pub others: Vec<String>,
    /// Where a new key would be written. Only set when this PC has no key and one can be made.
    pub new_key_path: Option<String>,
}

/// What came of [`setup_key_login`]. A failure is an error with a message saying why.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum KeySetupOutcome {
    /// Nothing was sent: getting in needs the password, and the owner hasn't approved the
    /// machine's identity. Approve it with [`approve_key_setup_identity`], then try again.
    Approve {
        offered_key: OfferedKey,
        /// Set when the machine's identity is not the one stored: the stored one.
        expected_fingerprint: Option<String>,
    },
    /// The key is in place on the machine (added now, or there already), and a login with
    /// keys only was tried.
    Done {
        /// The file on the machine that holds the key.
        file: String,
        already_there: bool,
        /// True if home-cc then got in without a password.
        key_login_works: bool,
        windows: bool,
        /// One or two sentences for the owner: what was done and how the test went.
        message: String,
        /// With `key_login_works` false: what to check on the machine.
        advice: Option<String>,
    },
}

/// Identities shown to the owner by [`setup_key_login`], so an approval can only ever
/// store the one they were shown, for the place it was seen at.
#[derive(Default)]
pub struct KeySetup {
    shown: Mutex<HashMap<i64, Shown>>,
}

#[derive(Debug, Clone)]
struct Shown {
    host: String,
    port: u16,
    key: HostKey,
}

// --- Commands ---

/// Which key this PC would log in with, and its public half.
#[tauri::command]
pub async fn local_ssh_key(app: AppHandle) -> Result<LocalKey> {
    let chosen = key_path_setting(&app)?;
    tauri::async_runtime::spawn_blocking(move || local_key(chosen.as_deref()))
        .await
        .map_err(|e| Error::Invalid(format!("Couldn't look for this PC's SSH key: {e}")))
}

/// Make an ed25519 key in the .ssh folder, for a PC that has none. A passphrase, if one
/// is given, locks the key and is stored in the keychain like the one set in Settings.
#[tauri::command]
pub async fn create_ssh_key(app: AppHandle, passphrase: Option<String>) -> Result<LocalKey> {
    if key_path_setting(&app)?.is_some() {
        return Err(Error::Invalid(
            "A key file is chosen in Settings, so home-cc uses that one. To make a new key, empty that box in \
             Settings first."
                .into(),
        ));
    }
    let passphrase = passphrase.filter(|p| !p.is_empty());
    let made = tauri::async_runtime::spawn_blocking(move || -> Result<LocalKey> {
        let home = ssh::home_dir().ok_or_else(|| {
            Error::Invalid("home-cc couldn't find your home folder, so it doesn't know where to put a key.".into())
        })?;
        if !ssh::key_candidates(None, Some(&home)).is_empty() {
            return Err(Error::Invalid("This PC already has an SSH key, so home-cc didn't make another.".into()));
        }
        create_key_in(&home.join(".ssh"), &key_comment(), passphrase.as_deref()).map_err(Error::Invalid)?;
        if let Some(passphrase) = &passphrase {
            secrets::set(monitor::KEY_PASSPHRASE, passphrase).map_err(|e| {
                Error::Invalid(format!(
                    "The key was made, but its passphrase couldn't be stored in the keychain ({e}). Store it in \
                     Settings, or home-cc can't use the key."
                ))
            })?;
        }
        Ok(local_key(None))
    })
    .await
    .map_err(|e| Error::Invalid(format!("Couldn't make the key: {e}")))??;
    // A key may be what a failed login was missing.
    app.state::<Monitor>().retry_logins();
    Ok(made)
}

/// Add this PC's public key to the machine, then check that a login with keys only works.
/// `password` is what was typed in the form, if anything; it is used for this login only
/// and not stored. Without it the stored password is used, if there is one.
#[tauri::command]
pub async fn setup_key_login(app: AppHandle, device_id: i64, password: Option<String>) -> Result<KeySetupOutcome> {
    let inv = app.state::<Inventory>();
    let device = inv.get(device_id)?;
    let user = device.ssh_user.clone().unwrap_or_default().trim().to_string();
    if user.is_empty() {
        return Err(Error::Invalid("This machine has no SSH user. Add one and save, then try again.".into()));
    }
    let chosen = key_path_setting(&app)?;
    let lookup = chosen.clone();
    let local = tauri::async_runtime::spawn_blocking(move || local_key(lookup.as_deref()))
        .await
        .map_err(|e| Error::Invalid(format!("Couldn't look for this PC's SSH key: {e}")))?;
    let key = match (&local.public_key, &local.problem) {
        (Some(line), _) => parse_key_line(line).map_err(Error::Invalid)?,
        (None, Some(problem)) => return Err(Error::Invalid(problem.clone())),
        (None, None) => {
            return Err(Error::Invalid("This PC has no SSH key yet. Make one first (above).".into()));
        }
    };

    let known_key = inv.host_key(device_id)?;
    let approved = known_key.is_some() && inv.host_key_approved(device_id)?;
    let typed = password.filter(|p| !p.is_empty());
    // The keychain can be slow to answer, so keep it off the async threads. One that
    // can't be read at all is no reason to stop: a typed password or a key may do.
    let (key_passphrase, stored) = tauri::async_runtime::spawn_blocking(move || {
        let read = |key: &str| {
            secrets::get(key).unwrap_or_else(|e| {
                crate::diag::note(format!("could not read {key} from the keychain: {e}"));
                None
            })
        };
        (read(monitor::KEY_PASSPHRASE), read(&monitor::password_key(device_id)))
    })
    .await
    .unwrap_or((None, None));
    let password_typed = typed.is_some();
    let password = typed.or(stored.filter(|p| !p.is_empty()));
    let job = Job {
        password_held_back: password.is_some() && !approved,
        password_typed,
        login: Login {
            host: device.host.clone(),
            port: device.ssh_port,
            user,
            known_key,
            key_path: chosen,
            key_passphrase,
            // A password only ever goes to an identity the owner approved.
            password: password.filter(|_| approved),
        },
        key,
        os: device.os,
        name: device.name.clone(),
    };

    // An identity seen on first contact is remembered (not approved) at once, as the
    // stats timer does, so that a later change is noticed.
    let (remember_app, host, port) = (app.clone(), device.host.clone(), device.ssh_port);
    let first_use = move |key: &HostKey| {
        if let Err(e) = remember_app.state::<Inventory>().remember_host_key(device_id, &host, port, key) {
            crate::diag::note(format!("could not store the identity of device {device_id}: {e}"));
        }
    };
    let step = run(&job, &first_use).await.map_err(Error::Invalid)?;
    let setup = app.state::<KeySetup>();
    match &step {
        KeySetupOutcome::Approve { offered_key, .. } => {
            let key = HostKey { algorithm: offered_key.algorithm.clone(), fingerprint: offered_key.fingerprint.clone() };
            setup.shown.lock().unwrap().insert(device_id, Shown { host: device.host, port: device.ssh_port, key });
        }
        KeySetupOutcome::Done { .. } => {
            setup.shown.lock().unwrap().remove(&device_id);
            // The card may be showing a failed login that this has just fixed.
            app.state::<Monitor>().retry_login(device_id);
        }
    }
    Ok(step)
}

/// The owner checked the identity [`setup_key_login`] showed them and approves it. Only
/// that identity is stored, and only while the machine is still at the same address.
#[tauri::command(async)]
pub fn approve_key_setup_identity(
    inv: State<Inventory>,
    monitor: State<Monitor>,
    setup: State<KeySetup>,
    device_id: i64,
    fingerprint: String,
) -> Result<()> {
    let device = inv.get(device_id)?;
    let shown = setup.shown.lock().unwrap().get(&device_id).cloned();
    let Some(shown) = shown.filter(|s| {
        s.key.fingerprint == fingerprint && s.host == device.host && s.port == device.ssh_port
    }) else {
        return Err(Error::Invalid(
            "This machine has shown a different identity since, or its address changed. Try setting up key login \
             again and check the identity it shows then."
                .into(),
        ));
    };
    inv.approve_host_key(device_id, &shown.key)?;
    setup.shown.lock().unwrap().remove(&device_id);
    monitor.retry_login(device_id);
    Ok(())
}

/// The key file chosen in Settings, if any.
fn key_path_setting(app: &AppHandle) -> Result<Option<PathBuf>> {
    let path = app.state::<Inventory>().settings()?.ssh_key_path;
    Ok(Some(path).filter(|p| !p.trim().is_empty()).map(PathBuf::from))
}

// --- This PC's key ---

/// Which key this PC logs in with: the file chosen in Settings, or else the first of the
/// usual ones in the .ssh folder (the order `ssh.rs` offers them in).
fn local_key(chosen: Option<&Path>) -> LocalKey {
    let home = ssh::home_dir();
    find_local_key(chosen, home.as_deref(), &|| {
        secrets::get(monitor::KEY_PASSPHRASE).ok().flatten()
    })
}

/// [`local_key`] with the home folder given, for tests. `passphrase` is only asked for if
/// the public key can't be read without it.
fn find_local_key(chosen: Option<&Path>, home: Option<&Path>, passphrase: &dyn Fn() -> Option<String>) -> LocalKey {
    let chosen = chosen.filter(|p| !p.as_os_str().is_empty());
    let files = ssh::key_candidates(chosen, home);
    let mut local = LocalKey { from_settings: chosen.is_some(), ..Default::default() };
    let Some((path, name)) = files.first() else {
        match home {
            Some(home) => local.new_key_path = Some(home.join(".ssh").join(NEW_KEY).display().to_string()),
            None => {
                local.problem =
                    Some("home-cc couldn't find your home folder, so it doesn't know where SSH keys are.".into())
            }
        }
        return local;
    };
    local.path = Some(path.display().to_string());
    local.name = Some(name.clone());
    local.others = files.iter().skip(1).map(|(_, name)| name.clone()).collect();
    if !path.is_file() {
        local.problem = Some(format!(
            "The key file chosen in Settings ({}) isn't there. Choose the right file in Settings, or empty that \
             box to use the usual ones in your .ssh folder.",
            path.display()
        ));
        return local;
    }
    match read_public(path, name, passphrase).and_then(|(key, note)| Ok((key_line(&key)?, note))) {
        Ok((line, note)) => {
            local.public_key = Some(line.line);
            local.algorithm = Some(line.algorithm);
            local.fingerprint = Some(line.fingerprint);
            local.note = note;
        }
        Err(problem) => local.problem = Some(problem),
    }
    local
}

/// The public half of a private key file: from the .pub file next to it, or worked out
/// from the key itself (an OpenSSH key file holds its public half in the clear, even when
/// it is locked). The text is a note for the owner, when there is something to say.
fn read_public(
    private: &Path,
    name: &str,
    passphrase: &dyn Fn() -> Option<String>,
) -> std::result::Result<(PublicKey, Option<String>), String> {
    let mut pub_path = private.as_os_str().to_owned();
    pub_path.push(".pub");
    let from_file = read_small(Path::new(&pub_path), MAX_PUBLIC_FILE).and_then(|text| {
        let line = text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('#'))?;
        PublicKey::from_openssh(line).ok()
    });
    let derived = read_small(private, MAX_PRIVATE_FILE).and_then(|text| {
        if let Ok(key) = PrivateKey::from_openssh(text.trim()) {
            return Some(key.public_key().clone());
        }
        if let Ok(key) = russh::keys::decode_secret_key(&text, None) {
            return Some(key.public_key().clone());
        }
        // Locked PEM, PKCS#8 or PuTTY files don't show their public half without the
        // passphrase. Asked for only now, as reading the keychain can be slow.
        let passphrase = passphrase().filter(|p| !p.is_empty())?;
        russh::keys::decode_secret_key(&text, Some(&passphrase)).ok().map(|key| key.public_key().clone())
    });
    match (from_file, derived) {
        (Some(file), Some(key)) if fingerprint(&file) != fingerprint(&key) => Ok((
            key,
            Some(format!(
                "{name}.pub doesn't belong to {name}, so home-cc worked the public key out from {name} itself."
            )),
        )),
        (Some(file), _) => Ok((file, None)),
        (None, Some(key)) => Ok((key, None)),
        (None, None) => Err(format!(
            "Couldn't read the public half of {name}: there is no {name}.pub next to it, and {name} itself is \
             locked with a passphrase or isn't a key home-cc can read. If it is locked, store its passphrase in \
             Settings."
        )),
    }
}

/// The text of a small ordinary file, or None. Never reads past `limit`.
fn read_small(path: &Path, limit: u64) -> Option<String> {
    use std::io::Read;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > limit {
        return None;
    }
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(limit + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= limit).then_some(text)
}

fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// A public key as one line for authorized_keys, written out afresh from the parsed key
/// so it holds only characters that are safe inside single quotes in sh and PowerShell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyLine {
    /// "ssh-ed25519 AAAA… comment".
    line: String,
    /// The same without the comment: what tells two keys apart.
    body: String,
    algorithm: String,
    fingerprint: String,
}

fn key_line(key: &PublicKey) -> std::result::Result<KeyLine, String> {
    let bare = PublicKey::new(key.key_data().clone(), "");
    let body = bare.to_openssh().map_err(|e| format!("This key can't be written out as a public key ({e})."))?;
    let body = body.trim().to_string();
    let comment = safe_comment(key.comment().as_str_lossy());
    let line = if comment.is_empty() { body.clone() } else { format!("{body} {comment}") };
    // Written out by ssh-key itself, so this can't fail; checked anyway, because these
    // characters are all that keeps the line from being read as a command.
    if body.split(' ').count() != 2 || !line.chars().all(safe_char) {
        return Err("This key has characters in it that home-cc won't send to another machine.".into());
    }
    if line.len() > MAX_KEY_LINE {
        return Err("This key is too long to send to another machine. Add it by hand.".into());
    }
    Ok(KeyLine { line, body, algorithm: key.algorithm().to_string(), fingerprint: fingerprint(key) })
}

/// A key line that came back from the screens: checked again in full.
fn parse_key_line(text: &str) -> std::result::Result<KeyLine, String> {
    let key = PublicKey::from_openssh(text.trim()).map_err(|_| "That isn't a public key home-cc can read.".to_string())?;
    key_line(&key)
}

fn safe_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "+/=@._:- ".contains(c)
}

/// The comment of a key, kept to letters, digits and a few signs (no spaces), so it can't
/// break out of the quotes it is sent in. Often "user@computer".
fn safe_comment(comment: &str) -> String {
    let kept: String = comment
        .trim()
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .filter(|&c| c != ' ' && safe_char(c))
        .take(MAX_COMMENT)
        .collect();
    kept.trim_matches(|c| c == '_' || c == '-').to_string()
}

/// "user@computer" for a new key, like ssh-keygen writes, so the owner can tell in a
/// machine's authorized_keys which PC a key belongs to.
fn key_comment() -> String {
    let env = |name: &str| std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let user = env("USERNAME").or_else(|| env("USER"));
    let computer = env("COMPUTERNAME").or_else(|| env("HOSTNAME")).or_else(|| {
        std::fs::read_to_string("/etc/hostname").ok().map(|h| h.trim().to_string()).filter(|h| !h.is_empty())
    });
    let comment = match (user, computer) {
        (Some(user), Some(computer)) => format!("{user}@{computer}"),
        (None, Some(computer)) => format!("home-cc@{computer}"),
        _ => "home-cc".into(),
    };
    Some(safe_comment(&comment)).filter(|c| !c.is_empty()).unwrap_or_else(|| "home-cc".into())
}

/// Make id_ed25519 and id_ed25519.pub in `dir`, which is made if it is missing. Fails,
/// changing nothing, if either file is already there. On Linux the private key is
/// readable by its owner only (0600) and a new folder is 0700; on Windows both inherit
/// the permissions of the user's own profile folder.
fn create_key_in(dir: &Path, comment: &str, passphrase: Option<&str>) -> std::result::Result<PathBuf, String> {
    let private = dir.join(NEW_KEY);
    let public = dir.join(format!("{NEW_KEY}.pub"));
    for path in [&private, &public] {
        // symlink_metadata also sees a link that points nowhere.
        if path.symlink_metadata().is_ok() {
            return Err(format!("{} is already there. home-cc never overwrites a key file.", path.display()));
        }
    }
    let mut folder = std::fs::DirBuilder::new();
    folder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut folder, 0o700);
    folder.create(dir).map_err(|e| format!("Couldn't make the folder {} ({e}).", dir.display()))?;

    let made = |e: russh::keys::ssh_key::Error| format!("Couldn't make the key ({e}).");
    let mut rng = rand::rng();
    let mut key = PrivateKey::random(&mut rng, Algorithm::Ed25519).map_err(made)?;
    key.set_comment(comment);
    // Before locking: in a locked file the comment is hidden with the rest.
    let public_text = key.public_key().to_openssh().map_err(made)? + "\n";
    let key = match passphrase.filter(|p| !p.is_empty()) {
        Some(passphrase) => key.encrypt(&mut rng, passphrase).map_err(made)?,
        None => key,
    };
    let private_text = key.to_openssh(LineEnding::LF).map_err(made)?;

    write_new(&private, private_text.as_bytes(), 0o600)
        .map_err(|e| format!("Couldn't write {} ({e}).", private.display()))?;
    if let Err(e) = write_new(&public, public_text.as_bytes(), 0o644) {
        // Half a key pair is no use, and would stop the next try.
        let _ = std::fs::remove_file(&private);
        return Err(format!("Couldn't write {} ({e}).", public.display()));
    }
    Ok(private)
}

/// Write a file that must not exist yet. `mode` is the Linux permissions.
fn write_new(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, mode);
    #[cfg(not(unix))]
    let _ = mode;
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

// --- Adding the key to a machine ---

/// Everything [`run`] needs, gathered by [`setup_key_login`].
struct Job {
    /// The password is in here only if the owner approved the machine's identity.
    login: Login,
    /// A password is typed or stored, but it was left out of `login`: the identity
    /// isn't approved.
    password_held_back: bool,
    /// The password (in `login` or held back) is the one typed in the form.
    password_typed: bool,
    key: KeyLine,
    /// What the owner said the machine runs. The server's own greeting wins.
    os: Os,
    /// The machine's name in home-cc, for messages.
    name: String,
}

impl Job {
    fn has_password(&self) -> bool {
        self.login.has_password() || self.password_held_back
    }
}

/// Log in, add the key, log in again with keys only. An error is a message for the owner.
async fn run(job: &Job, first_use: &(dyn Fn(&HostKey) + Send + Sync)) -> std::result::Result<KeySetupOutcome, String> {
    let shown: Arc<Mutex<Option<HostKey>>> = Arc::default();
    let seen = shown.clone();
    let remember = move |key: &HostKey| {
        *seen.lock().unwrap() = Some(key.clone());
        first_use(key);
    };
    let session = match ssh::connect(&job.login, &remember).await {
        Ok(session) => session,
        Err(SshError::HostKeyChanged { expected, got }) => {
            return Ok(KeySetupOutcome::Approve {
                offered_key: OfferedKey::from(&got),
                expected_fingerprint: Some(expected.fingerprint),
            });
        }
        Err(SshError::NewHost(key)) => {
            return Ok(KeySetupOutcome::Approve { offered_key: OfferedKey::from(&key), expected_fingerprint: None });
        }
        // No key got in, and the password was kept back because nobody has approved
        // this identity: show it to the owner.
        Err(SshError::AuthFailed(message)) if job.password_held_back => {
            let key = job.login.known_key.clone().or_else(|| shown.lock().unwrap().clone());
            return match key {
                Some(key) => {
                    Ok(KeySetupOutcome::Approve { offered_key: OfferedKey::from(&key), expected_fingerprint: None })
                }
                None => Err(format!("{message} The machine did not show an identity to approve.")),
            };
        }
        Err(SshError::AuthFailed(message)) => return Err(login_failed(job, &message)),
        Err(e) => return Err(format!("Couldn't connect to {} to set up key login. {e}", job.name)),
    };
    let host_key = session.host_key().clone();
    let windows = is_windows(session.server_version(), job.os);
    let ran = if windows {
        match windows_command(&job.key) {
            Ok(command) => session.exec(&command, None, SCRIPT_LIMIT).await,
            Err(e) => {
                session.close().await;
                return Err(e);
            }
        }
    } else {
        session.exec("sh -s", Some(linux_script(&job.key).as_bytes()), SCRIPT_LIMIT).await
    };
    session.close().await;
    let output = ran.map_err(|e| format!("Logged in to {}, but couldn't run the command that adds the key. {e}", job.name))?;
    let report = parse_report(&output.stdout);
    if report.error.is_some() || !(report.added || report.already) {
        return Err(not_added(job, windows, &report, &String::from_utf8_lossy(&output.stderr)));
    }
    let file = report.file.clone().unwrap_or_else(|| "authorized_keys".into());

    // The test: the same login with keys only, to the identity just seen.
    let test = Login { password: None, known_key: Some(host_key), ..job.login.clone() };
    let tested = ssh::connect(&test, &|_: &HostKey| {}).await;
    let (key_login_works, why) = match tested {
        Ok(session) => {
            session.close().await;
            (true, None)
        }
        Err(e) => (false, Some(e)),
    };
    let what = if report.already {
        format!("This PC's key was already in {file} on {}", job.name)
    } else {
        format!("home-cc added this PC's key to {file} on {}", job.name)
    };
    let (message, advice) = match &why {
        None => (format!("Key login works. {what}, and then logged in with the key alone."), None),
        Some(SshError::AuthFailed(seen)) => (
            format!("{what}, but logging in with the key alone still didn't work. What home-cc saw: {seen}"),
            Some(key_refused_advice(job, windows, &report, seen)),
        ),
        Some(other) => (
            format!("{what}, but home-cc couldn't log in again to check that the key works: {other}"),
            Some("Press Set up key login again to repeat the check; the key won't be added twice.".into()),
        ),
    };
    Ok(KeySetupOutcome::Done { file, already_there: report.already, key_login_works, windows, message, advice })
}

/// True if the machine is to be treated as Windows. Windows' own SSH server says so in
/// its greeting ("SSH-2.0-OpenSSH_for_Windows_9.5"). Other servers on Windows say nothing,
/// so then what the owner chose goes; but an OpenSSH greeting without "Windows" is the
/// Linux one, whatever the form says.
fn is_windows(server_version: &str, os: Os) -> bool {
    server_version.contains("Windows") || (os == Os::Windows && !server_version.contains("OpenSSH"))
}

/// What the script for Linux and other Unix systems does, read from its stdin by `sh -s`,
/// so no shell sees it as a command line. The key goes in single quotes; [`KeyLine`] has
/// no character that could end them. Like ssh-copy-id: make ~/.ssh if needed, add the line
/// unless the key is already there (with any comment or options), fix the permissions
/// StrictModes wants, and report what it found as "HOMECC:" lines. chmod is allowed to
/// fail: on a Proxmox host root's authorized_keys is a link into /etc/pve, which has no
/// permissions to change.
const LINUX_SCRIPT: &str = r#"key='__KEY__'
body='__BODY__'
say() { printf 'HOMECC:%s\n' "$*"; }
fail() { say "ERR $1"; exit 1; }
[ -n "${HOME:-}" ] && [ -d "$HOME" ] || fail home
say "HOME $HOME"
say "HOMEMODE $(ls -ld "$HOME" 2>/dev/null | cut -c1-10)"
umask 077
dir="$HOME/.ssh"
file="$dir/authorized_keys"
[ -d "$dir" ] || mkdir "$dir" || fail mkdir
chmod 700 "$dir" 2>/dev/null || say "NOTE chmod-dir"
if [ -f "$file" ] && grep -qF -e "$body" "$file" 2>/dev/null; then
  say ALREADY
else
  if [ -s "$file" ] && [ -n "$(tail -c 1 "$file")" ]; then
    printf '\n' >> "$file" || fail write
  fi
  printf '%s\n' "$key" >> "$file" || fail write
  say ADDED
fi
chmod 600 "$file" 2>/dev/null || say "NOTE chmod-file"
command -v restorecon >/dev/null 2>&1 && restorecon -F "$dir" "$file" >/dev/null 2>&1
say "FILE $file"
say "DIRMODE $(ls -ld "$dir" 2>/dev/null | cut -c1-10)"
say "FILEMODE $(ls -lLd "$file" 2>/dev/null | cut -c1-10)"
for f in /etc/ssh/sshd_config /etc/ssh/sshd_config.d/*.conf; do
  [ -r "$f" ] && grep -iE '^[[:space:]]*(PubkeyAuthentication|AuthorizedKeysFile|StrictModes)[[:space:]]' "$f" 2>/dev/null | head -n 5 | sed 's/^/HOMECC:CFG /'
done
exit 0
"#;

fn linux_script(key: &KeyLine) -> String {
    LINUX_SCRIPT.replace("__BODY__", &key.body).replace("__KEY__", &key.line)
}

/// The script for Windows' OpenSSH server, run by PowerShell whatever the account's shell.
/// For a member of the Administrators group the server reads keys only from
/// C:\ProgramData\ssh\administrators_authorized_keys (the "Match Group administrators"
/// lines of its default sshd_config); that file must be open to Administrators and SYSTEM
/// only, or the server ignores it. Everyone else's keys go in %USERPROFILE%\.ssh. The
/// group is found by its SID, which is the same in every language. The file is written as
/// UTF-8 without a byte order mark: one saved as UTF-16 (as `echo >>` does in Windows
/// PowerShell) can't be read by the server, so such a file is rewritten. Kept short: the
/// whole command has to fit in cmd.exe's 8191 characters.
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$key='__KEY__'
$body=($key -split ' ')[0..1] -join ' '
function Say($t){[Console]::Out.WriteLine("HOMECC:$t")}
$admin=$false
try{$admin=[bool]((whoami.exe /groups /fo csv) -match 'S-1-5-32-544')}catch{Say 'NOTE groups'}
Say "ADMIN $([int]$admin)"
$match=$true
try{$cfg=@(Get-Content -LiteralPath "$env:ProgramData\ssh\sshd_config")
$match=[bool]($cfg -match '^\s*Match\s+Group\s+administrators\b')
$cfg -match '^\s*(PubkeyAuthentication|AuthorizedKeysFile|StrictModes)\s'|Select-Object -First 6|ForEach-Object{Say "CFG $_"}}catch{Say 'NOTE cfg'}
Say "MATCH $([int]$match)"
if($admin -and $match){$file="$env:ProgramData\ssh\administrators_authorized_keys"}else{$dir="$env:USERPROFILE\.ssh";$file="$dir\authorized_keys"
try{if(-not(Test-Path -LiteralPath $dir)){New-Item -ItemType Directory -Path $dir|Out-Null}}catch{Say "ERR mkdir $($_.Exception.Message)";exit 1}}
Say "FILE $file"
try{$text='';$bom=$false
if(Test-Path -LiteralPath $file){$b=[IO.File]::ReadAllBytes($file)
$bom=$b.Length -gt 1 -and $b[0] -in 0xEF,0xFE,0xFF
$text=[IO.File]::ReadAllText($file)}
$new=-not $text.Contains($body)
if($new){if($text.Length -gt 0 -and -not $text.EndsWith("`n")){$text+="`r`n"};$text+=$key+"`r`n"}
if($new -or $bom){[IO.File]::WriteAllText($file,$text,(New-Object Text.UTF8Encoding $false))}
if($bom){Say 'NOTE encoding'}
if($new){Say 'ADDED'}else{Say 'ALREADY'}}catch{Say "ERR write $($_.Exception.Message)";exit 1}
if($admin -and $match){try{icacls.exe $file /inheritance:r /grant '*S-1-5-32-544:F' '*S-1-5-18:F'|Out-Null
if($LASTEXITCODE){throw "icacls ended with $LASTEXITCODE"}
foreach($r in (Get-Acl -LiteralPath $file).Access){$s=$r.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
if($s -notin 'S-1-5-32-544','S-1-5-18'){icacls.exe $file /remove:g "*$s"|Out-Null}}
Say 'NOTE acl'}catch{Say "ERR acl $($_.Exception.Message)";exit 1}}
exit 0
"#;

fn windows_command(key: &KeyLine) -> std::result::Result<String, String> {
    let command = crate::stats::powershell_command(&WINDOWS_SCRIPT.replace("__KEY__", &key.line));
    if command.len() > CMD_LIMIT {
        return Err("This key is too long to send to a Windows machine this way. Add it by hand (see Do it by hand).".into());
    }
    Ok(command)
}

/// What a script reported in its "HOMECC:" lines.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Report {
    added: bool,
    already: bool,
    /// The file the key went into.
    file: Option<String>,
    home: Option<String>,
    /// Permissions as `ls -l` shows them, such as "drwxr-xr-x" (Linux).
    home_mode: Option<String>,
    dir_mode: Option<String>,
    file_mode: Option<String>,
    /// Windows: the account is in the Administrators group.
    admin: Option<bool>,
    /// Windows: sshd_config sends administrators to the shared file.
    admin_match: Option<bool>,
    /// Lines of sshd_config about keys.
    config: Vec<String>,
    /// Small things that went wrong or were fixed, such as "chmod-file" or "encoding".
    notes: Vec<String>,
    /// The step that failed ("home", "mkdir", "write", "acl"), with Windows' own message.
    error: Option<(String, String)>,
}

/// Read the "HOMECC:" lines. Everything else the machine printed (a login banner, say)
/// is left alone. The text came from another machine, so it is cut short and cleaned of
/// control characters before anything shows it.
fn parse_report(stdout: &[u8]) -> Report {
    let text = String::from_utf8_lossy(stdout);
    let mut report = Report::default();
    for line in text.lines() {
        let Some(rest) = line.trim_end_matches('\r').strip_prefix("HOMECC:") else { continue };
        let (tag, value) = rest.split_once(' ').unwrap_or((rest, ""));
        let value = clean(value);
        let flag = |v: &str| Some(v.trim() == "1");
        match tag.trim() {
            "ADDED" => report.added = true,
            "ALREADY" => report.already = true,
            "FILE" => report.file = Some(value),
            "HOME" => report.home = Some(value),
            "HOMEMODE" => report.home_mode = Some(value).filter(|v| !v.is_empty()),
            "DIRMODE" => report.dir_mode = Some(value).filter(|v| !v.is_empty()),
            "FILEMODE" => report.file_mode = Some(value).filter(|v| !v.is_empty()),
            "ADMIN" => report.admin = flag(&value),
            "MATCH" => report.admin_match = flag(&value),
            "CFG" if report.config.len() < 12 => report.config.push(value),
            "NOTE" => report.notes.push(value),
            "ERR" => {
                let (step, why) = value.split_once(' ').unwrap_or((&value, ""));
                report.error = Some((step.to_string(), why.trim().to_string()));
            }
            _ => {}
        }
    }
    report
}

/// Text from another machine, made safe to show: one line, no control characters, short.
fn clean(text: &str) -> String {
    let one_line: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(300).collect();
    one_line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Why the login that was to add the key failed, with what to do. `message` is the one
/// from `ssh.rs`, which already says what was tried.
fn login_failed(job: &Job, message: &str) -> String {
    let user = job.login.user.as_str();
    let mut text = if job.password_typed {
        message.replace("the stored password", "the password typed in the form").replace(
            "The stored password",
            "The password typed in the form",
        )
    } else {
        message.to_string()
    };
    if !job.has_password() {
        return format!(
            "home-cc couldn't get in to {} with a key, and no password is typed or stored for it. Type {user}'s \
             password in the box above, then press Set up key login again. It is used for this once and stored \
             only if you press Save. What home-cc saw: {text}",
            job.name
        );
    }
    if message.contains("does not take passwords") {
        text += " Its SSH server has password logins turned off (PasswordAuthentication no), so home-cc can't use \
                 the password to add the key. Add the key by hand with a login that works there (see Do it by \
                 hand), or turn password logins on for a moment.";
    } else if message.contains("password") && message.contains("not accepted") {
        if user == "root" {
            text += " Many Linux systems don't let root log in with a password over SSH (PermitRootLogin in \
                     /etc/ssh/sshd_config). Use another user, or add the key by hand.";
        } else if job.os == Os::Windows {
            text += " On Windows, use the account's own password (for a Microsoft account, the one for that \
                     account, not the PIN).";
        }
    }
    text
}

/// Why the key couldn't be added, from the step that failed.
fn not_added(job: &Job, windows: bool, report: &Report, stderr: &str) -> String {
    let user = job.login.user.as_str();
    let name = job.name.as_str();
    let said = |why: &str| {
        let why = if why.is_empty() { clean(stderr) } else { why.to_string() };
        if why.is_empty() { String::new() } else { format!(" The machine said: {why}") }
    };
    let home = report.home.clone().unwrap_or_else(|| "the home folder".into());
    let file = report.file.clone().unwrap_or_else(|| "authorized_keys".into());
    let Some((step, why)) = &report.error else {
        return format!(
            "Logged in to {name}, but the command that adds the key gave no answer home-cc understands, so the \
             key may not have been added.{} {}",
            said(""),
            if windows {
                "Check that Windows PowerShell is installed there."
            } else {
                "Check that the account has an ordinary shell (sh). If it is really a Windows machine, set its \
                 System to Windows."
            }
        );
    };
    match (windows, step.as_str()) {
        (false, "home") => format!(
            "{user} has no home folder on {name}, so there is nowhere to put the key. Check the user name.{}",
            said(why)
        ),
        (false, "mkdir") => format!(
            "Couldn't make the .ssh folder in {home} on {name}. The home folder may not belong to {user}, or may not \
             be writable (check with ls -ld {home} on the machine).{}",
            said(why)
        ),
        (false, _) => format!(
            "Couldn't add the key to {home}/.ssh/authorized_keys on {name}. Check that {user} owns the .ssh folder \
             and the file in it (ls -la ~/.ssh on the machine), and that the disk isn't full.{}",
            said(why)
        ),
        (true, "mkdir") => format!("Couldn't make the .ssh folder in {user}'s profile on {name}.{}", said(why)),
        (true, "acl") => format!(
            "The key was added to {file} on {name}, but the file's permissions couldn't be set to Administrators \
             and SYSTEM only, and Windows' SSH server ignores the file until they are. In PowerShell run as \
             administrator on {name}: icacls.exe \"{file}\" /inheritance:r /grant \"*S-1-5-32-544:F\" \
             \"*S-1-5-18:F\"{}",
            said(why)
        ),
        (true, _) if report.admin == Some(true) && report.admin_match != Some(false) => format!(
            "{user} is an administrator on {name}, so Windows' SSH server reads its keys from \
             C:\\ProgramData\\ssh\\administrators_authorized_keys, but this login wasn't allowed to change that \
             file. Add the key by hand in PowerShell run as administrator (see Do it by hand).{}",
            said(why)
        ),
        (true, _) => format!("Couldn't add the key to {file} on {name}.{}", said(why)),
    }
}

/// What to check when the key is on the machine but a login with it was still refused.
fn key_refused_advice(job: &Job, windows: bool, report: &Report, seen: &str) -> String {
    let name = job.name.as_str();
    let mut tips: Vec<String> = Vec::new();
    if seen.contains("locked with a passphrase") || seen.contains("passphrase stored in Settings") {
        tips.push("This PC's key is locked with a passphrase: store the passphrase in Settings.".into());
    }
    for line in &report.config {
        let lower = line.to_ascii_lowercase();
        let words: Vec<&str> = lower.split_whitespace().collect();
        match words.as_slice() {
            ["pubkeyauthentication", "no", ..] => tips.push(format!(
                "The SSH server has key logins turned off ({line}). Change it to PubkeyAuthentication yes and \
                 restart the SSH server."
            )),
            ["authorizedkeysfile", rest @ ..]
                if !windows && !rest.iter().any(|f| f.contains(".ssh/authorized_keys")) =>
            {
                tips.push(format!("The SSH server reads keys from somewhere else ({line}). Add the key there."))
            }
            _ => {}
        }
    }
    if windows {
        match (report.admin, report.admin_match) {
            (Some(true), Some(false)) => tips.push(format!(
                "{} is an administrator, but the Match Group administrators lines in sshd_config are turned off, \
                 so the key went to the account's own file. If they are on after all, the key has to be in \
                 C:\\ProgramData\\ssh\\administrators_authorized_keys.",
                job.login.user
            )),
            (Some(true), _) => tips.push(format!(
                "{} is an administrator, so the key has to be in C:\\ProgramData\\ssh\\administrators_authorized_keys \
                 (home-cc put it there), open to Administrators and SYSTEM only.",
                job.login.user
            )),
            _ => {}
        }
        tips.push(format!(
            "Check C:\\ProgramData\\ssh\\sshd_config on {name} (PubkeyAuthentication must not be no; after a change, \
             run Restart-Service sshd in PowerShell as administrator), and Windows' own log of the refusal: Event \
             Viewer, Applications and Services Logs, OpenSSH, Operational."
        ));
    } else {
        let home = report.home.clone().unwrap_or_else(|| "the home folder".into());
        if report.home_mode.as_deref().is_some_and(others_can_write) {
            tips.push(format!(
                "{home} can be changed by other users ({}). The SSH server then ignores the keys in it \
                 (StrictModes). On the machine: chmod go-w {home}",
                report.home_mode.as_deref().unwrap_or_default()
            ));
        }
        if report.dir_mode.as_deref().is_some_and(others_can_write) {
            tips.push("~/.ssh can be changed by other users. On the machine: chmod 700 ~/.ssh".into());
        }
        if report.file_mode.as_deref().is_some_and(others_can_write) {
            tips.push("~/.ssh/authorized_keys can be changed by other users. On the machine: chmod 600 ~/.ssh/authorized_keys".into());
        }
        tips.push(format!(
            "Check /etc/ssh/sshd_config on {name} (PubkeyAuthentication must not be no), the permissions of {home}, \
             ~/.ssh and the file in it, and the SSH server's log, which says why: sudo journalctl -u ssh -n 20 (on \
             some systems the service is called sshd)."
        ));
    }
    tips.join(" ")
}

/// For permissions as `ls -l` shows them ("drwxrwxr-x"): true if the group or everyone
/// else may change it.
fn others_can_write(mode: &str) -> bool {
    let chars: Vec<char> = mode.chars().collect();
    chars.len() >= 10 && (chars[5] == 'w' || chars[8] == 'w')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own for one test, removed afterwards.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let dir = std::env::temp_dir().join(format!("home-cc-keys-{name}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn no_passphrase() -> Option<String> {
        None
    }

    fn sample_line(comment: &str) -> KeyLine {
        use russh::keys::ssh_key::private::Ed25519Keypair;
        let mut key = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]));
        key.set_comment(comment);
        key_line(key.public_key()).unwrap()
    }

    fn job(login: Login, key: KeyLine) -> Job {
        Job { login, password_held_back: false, password_typed: false, key, os: Os::Linux, name: "proxmox".into() }
    }

    fn login() -> Login {
        Login {
            host: "192.168.68.54".into(),
            port: 22,
            user: "reuben".into(),
            known_key: None,
            key_path: None,
            key_passphrase: None,
            password: None,
        }
    }

    #[test]
    fn a_new_key_reads_back_and_is_never_overwritten() {
        let scratch = Scratch::new("create");
        let dir = scratch.0.join(".ssh");
        let private = create_key_in(&dir, "jim@main-pc", None).unwrap();
        assert_eq!(private, dir.join("id_ed25519"));

        let text = std::fs::read_to_string(&private).unwrap();
        assert!(text.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        let key = russh::keys::decode_secret_key(&text, None).unwrap();
        assert_eq!(key.algorithm(), Algorithm::Ed25519);
        let public = std::fs::read_to_string(dir.join("id_ed25519.pub")).unwrap();
        assert!(public.starts_with("ssh-ed25519 ") && public.trim_end().ends_with(" jim@main-pc"), "{public}");
        let parsed = PublicKey::from_openssh(public.trim()).unwrap();
        assert_eq!(fingerprint(&parsed), fingerprint(key.public_key()), "the .pub belongs to the key");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&private), 0o600);
            assert_eq!(mode(&dir), 0o700);
        }

        // A second try changes nothing.
        let error = create_key_in(&dir, "other", None).unwrap_err();
        assert!(error.contains("never overwrites"), "{error}");
        assert_eq!(std::fs::read_to_string(&private).unwrap(), text);

        // Nor does a .pub left behind on its own.
        let lone = Scratch::new("lone");
        std::fs::create_dir_all(lone.0.join(".ssh")).unwrap();
        std::fs::write(lone.0.join(".ssh/id_ed25519.pub"), "old").unwrap();
        assert!(create_key_in(&lone.0.join(".ssh"), "x", None).is_err());
        assert!(!lone.0.join(".ssh/id_ed25519").exists());
    }

    #[test]
    fn a_new_key_with_a_passphrase_needs_it() {
        let scratch = Scratch::new("locked");
        let dir = scratch.0.join(".ssh");
        let private = create_key_in(&dir, "jim@main-pc", Some("sesame open")).unwrap();
        let text = std::fs::read_to_string(&private).unwrap();
        assert!(matches!(russh::keys::decode_secret_key(&text, None), Err(russh::keys::Error::KeyIsEncrypted)));
        let key = russh::keys::decode_secret_key(&text, Some("sesame open")).unwrap();
        assert!(!text.contains("sesame"));

        // The public half is still shown, from the .pub file or from the locked file itself.
        let found = find_local_key(None, Some(&scratch.0), &no_passphrase);
        assert_eq!(found.fingerprint.as_deref(), Some(fingerprint(key.public_key()).as_str()));
        std::fs::remove_file(dir.join("id_ed25519.pub")).unwrap();
        let found = find_local_key(None, Some(&scratch.0), &no_passphrase);
        assert_eq!(found.fingerprint.as_deref(), Some(fingerprint(key.public_key()).as_str()));
        assert_eq!(found.problem, None);
    }

    #[test]
    fn the_key_shown_is_the_one_logins_use() {
        let scratch = Scratch::new("find");
        // Nothing there yet: offer to make one.
        let none = find_local_key(None, Some(&scratch.0), &no_passphrase);
        assert_eq!(none.path, None);
        assert_eq!(none.new_key_path, Some(scratch.0.join(".ssh").join("id_ed25519").display().to_string()));

        create_key_in(&scratch.0.join(".ssh"), "jim@main-pc", None).unwrap();
        let found = find_local_key(None, Some(&scratch.0), &no_passphrase);
        assert_eq!(found.name.as_deref(), Some("id_ed25519"));
        assert!(!found.from_settings);
        assert_eq!(found.new_key_path, None);
        let line = found.public_key.unwrap();
        assert!(line.starts_with("ssh-ed25519 AAAA") && line.ends_with(" jim@main-pc"), "{line}");

        // A key file chosen in Settings that isn't there is a problem, not an offer to make one.
        let missing = scratch.0.join("nope");
        let chosen = find_local_key(Some(missing.as_path()), Some(&scratch.0), &no_passphrase);
        assert!(chosen.from_settings);
        assert!(chosen.problem.unwrap().contains("isn't there"));
        assert_eq!(chosen.new_key_path, None);
    }

    #[test]
    fn a_pub_file_that_does_not_match_is_not_trusted() {
        let scratch = Scratch::new("mismatch");
        let dir = scratch.0.join(".ssh");
        create_key_in(&dir, "a", None).unwrap();
        std::fs::write(dir.join("id_ed25519.pub"), sample_line("someone-else").line).unwrap();
        let found = find_local_key(None, Some(&scratch.0), &no_passphrase);
        assert!(found.note.unwrap().contains("doesn't belong"));
        let text = std::fs::read_to_string(dir.join("id_ed25519")).unwrap();
        let key = russh::keys::decode_secret_key(&text, None).unwrap();
        assert_eq!(found.fingerprint.unwrap(), fingerprint(key.public_key()));
    }

    #[test]
    fn key_lines_hold_only_safe_characters() {
        let line = sample_line("jim@main-pc");
        assert!(line.line.starts_with("ssh-ed25519 AAAA"));
        assert_eq!(line.line, format!("{} jim@main-pc", line.body));

        // A comment can't break out of the quotes.
        for hostile in ["x' ; rm -rf ~ ; '", "$(reboot)", "`id`", "a\nb", "\"q\"", "a;b|c&d", "100%"] {
            let line = sample_line(hostile);
            assert!(!line.line.contains(['\'', '"', ';', '$', '`', '\n', '|', '&', '%', '(']), "{hostile} gave {}", line.line);
            assert!(line.line.starts_with(&line.body));
        }
        assert_eq!(sample_line("  ").line, sample_line("").body, "an empty comment is left out");

        // What comes back from the screens is checked again.
        assert!(parse_key_line(&sample_line("ok").line).is_ok());
        assert!(parse_key_line("ssh-ed25519 AAAA' ; rm -rf ~ '").is_err());
        assert!(parse_key_line("not a key").is_err());
    }

    #[test]
    fn the_linux_script_holds_the_key_in_single_quotes() {
        let line = sample_line("jim@main-pc");
        let script = linux_script(&line);
        assert!(script.starts_with(&format!("key='{}'\nbody='{}'\n", line.line, line.body)));
        assert!(!script.contains("__KEY__") && !script.contains("__BODY__"));
        assert_eq!(script.matches('\'').count(), LINUX_SCRIPT.matches('\'').count(), "no quote came in with the key");
    }

    /// Runs the Linux script here, as the machine would, with HOME pointing at `home`.
    /// It ends with 0 unless it reports an error.
    #[cfg(unix)]
    fn run_linux_script(home: &Path, key: &KeyLine) -> Report {
        use std::io::Write;
        let mut child = std::process::Command::new("sh")
            .arg("-s")
            .env("HOME", home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(linux_script(key).as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        let report = parse_report(&output.stdout);
        assert_eq!(output.status.success(), report.error.is_none(), "{}", String::from_utf8_lossy(&output.stderr));
        report
    }

    #[cfg(unix)]
    #[test]
    fn the_linux_script_adds_the_key_once() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("script");
        let line = sample_line("jim@main-pc");

        let first = run_linux_script(&scratch.0, &line);
        assert!(first.added && !first.already, "{first:?}");
        let file = scratch.0.join(".ssh/authorized_keys");
        assert_eq!(first.file.as_deref(), Some(file.display().to_string().as_str()));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("{}\n", line.line));
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&scratch.0.join(".ssh")), 0o700);
        assert_eq!(mode(&file), 0o600);
        assert_eq!(first.dir_mode.as_deref(), Some("drwx------"));

        // Again: nothing is doubled. Nor when the same key is there with another comment.
        let again = run_linux_script(&scratch.0, &line);
        assert!(again.already && !again.added);
        let other_comment = run_linux_script(&scratch.0, &sample_line("renamed"));
        assert!(other_comment.already);
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn the_linux_script_keeps_what_is_there() {
        let scratch = Scratch::new("existing");
        std::fs::create_dir_all(scratch.0.join(".ssh")).unwrap();
        let file = scratch.0.join(".ssh/authorized_keys");
        // A file that doesn't end in a new line, and a key that is there with options in front.
        std::fs::write(&file, "ssh-rsa AAAAB3Nza other@pc").unwrap();
        let line = sample_line("jim@main-pc");
        assert!(run_linux_script(&scratch.0, &line).added);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), format!("ssh-rsa AAAAB3Nza other@pc\n{}\n", line.line));

        std::fs::write(&file, format!("from=\"192.168.68.0/22\" {}\n", line.body)).unwrap();
        assert!(run_linux_script(&scratch.0, &line).already);
    }

    #[cfg(unix)]
    #[test]
    fn the_linux_script_says_when_the_home_folder_is_missing() {
        let scratch = Scratch::new("nohome");
        let report = run_linux_script(&scratch.0.join("gone"), &sample_line("x"));
        assert_eq!(report.error, Some(("home".into(), String::new())));
        let message = not_added(&job(login(), sample_line("x")), false, &report, "");
        assert!(message.contains("reuben has no home folder on proxmox"), "{message}");
    }

    #[test]
    fn the_windows_command_fits_cmd_and_names_both_files() {
        let line = sample_line("jim@main-pc");
        let command = windows_command(&line).unwrap();
        assert!(command.starts_with("powershell.exe ") && command.len() <= CMD_LIMIT);
        let script = WINDOWS_SCRIPT.replace("__KEY__", &line.line);
        assert!(script.contains(&format!("$key='{}'", line.line)));
        assert!(script.contains(r"$env:ProgramData\ssh\administrators_authorized_keys"));
        assert!(script.contains(r#"$env:USERPROFILE\.ssh";$file="$dir\authorized_keys""#));
        // Administrators are found by SID, and their file is opened to Administrators and SYSTEM only.
        assert!(script.contains("S-1-5-32-544") && script.contains("'*S-1-5-32-544:F' '*S-1-5-18:F'"));
        assert!(script.contains("/inheritance:r"));
        assert!(script.contains(r"Match\s+Group\s+administrators"));
        assert!(script.contains("UTF8Encoding $false"), "written without a byte order mark");

        // A 4096-bit RSA key still fits; one far too long is refused, not cut.
        let rsa_4096 = KeyLine {
            line: format!("ssh-rsa {} jim@main-pc", "A".repeat(716)),
            body: format!("ssh-rsa {}", "A".repeat(716)),
            algorithm: "ssh-rsa".into(),
            fingerprint: String::new(),
        };
        assert!(windows_command(&rsa_4096).is_ok());
        let huge = KeyLine { line: format!("ssh-rsa {}", "A".repeat(1900)), ..rsa_4096 };
        assert!(windows_command(&huge).unwrap_err().contains("by hand"));
    }

    #[test]
    fn windows_is_told_by_the_server_greeting() {
        assert!(is_windows("SSH-2.0-OpenSSH_for_Windows_9.5", Os::Linux));
        assert!(!is_windows("SSH-2.0-OpenSSH_9.2p1 Debian-2+deb12u3", Os::Windows));
        assert!(is_windows("SSH-2.0-WeOnlyDo 2.4.3", Os::Windows));
        assert!(!is_windows("SSH-2.0-dropbear_2022.83", Os::Proxmox));
    }

    #[test]
    fn the_report_says_where_the_key_went_on_windows() {
        let admin = parse_report(
            b"banner\r\nHOMECC:ADMIN 1\r\nHOMECC:MATCH 1\r\nHOMECC:FILE C:\\ProgramData\\ssh\\administrators_authorized_keys\r\nHOMECC:ADDED\r\nHOMECC:NOTE acl\r\n",
        );
        assert_eq!(admin.admin, Some(true));
        assert_eq!(admin.admin_match, Some(true));
        assert_eq!(admin.file.as_deref(), Some(r"C:\ProgramData\ssh\administrators_authorized_keys"));
        assert!(admin.added && admin.error.is_none());

        let user = parse_report(b"HOMECC:ADMIN 0\nHOMECC:MATCH 1\nHOMECC:FILE C:\\Users\\sam\\.ssh\\authorized_keys\nHOMECC:ALREADY\n");
        assert_eq!(user.admin, Some(false));
        assert!(user.already);

        let denied = parse_report(
            b"HOMECC:ADMIN 1\nHOMECC:MATCH 1\nHOMECC:FILE C:\\ProgramData\\ssh\\administrators_authorized_keys\nHOMECC:ERR write Access to the path is denied.\x07\n",
        );
        assert_eq!(denied.error, Some(("write".into(), "Access to the path is denied.".into())));
        let mut windows_job = job(Login { user: "sam".into(), ..login() }, sample_line("x"));
        windows_job.os = Os::Windows;
        windows_job.name = "NAS".into();
        let message = not_added(&windows_job, true, &denied, "");
        assert!(message.contains("sam is an administrator on NAS"), "{message}");
        assert!(message.contains("administrators_authorized_keys") && message.contains("Access to the path is denied."));

        let advice = key_refused_advice(&windows_job, true, &admin, "Could not log in.");
        assert!(advice.contains("Administrators and SYSTEM only") && advice.contains("Restart-Service sshd"), "{advice}");
        let unmatched = Report { admin_match: Some(false), ..admin.clone() };
        assert!(key_refused_advice(&windows_job, true, &unmatched, "").contains("turned off"));
    }

    #[test]
    fn advice_for_a_refused_key_on_linux_points_at_the_cause() {
        let report = parse_report(
            b"HOMECC:HOME /home/reuben\nHOMECC:HOMEMODE drwxrwxr-x\nHOMECC:ADDED\nHOMECC:DIRMODE drwx------\nHOMECC:FILEMODE -rw-------\nHOMECC:CFG PubkeyAuthentication no\nHOMECC:CFG AuthorizedKeysFile /etc/ssh/keys/%u\n",
        );
        let advice = key_refused_advice(&job(login(), sample_line("x")), false, &report, "Could not log in.");
        assert!(advice.contains("/home/reuben can be changed by other users (drwxrwxr-x)"), "{advice}");
        assert!(advice.contains("chmod go-w /home/reuben"));
        assert!(advice.contains("key logins turned off (PubkeyAuthentication no)"));
        assert!(advice.contains("reads keys from somewhere else"));
        assert!(advice.contains("journalctl"));

        let tidy = parse_report(b"HOMECC:HOMEMODE drwxr-x---\nHOMECC:CFG AuthorizedKeysFile .ssh/authorized_keys .ssh/authorized_keys2\n");
        let advice = key_refused_advice(&job(login(), sample_line("x")), false, &tidy, "");
        assert!(!advice.contains("changed by other users") && !advice.contains("somewhere else"), "{advice}");
        assert!(key_refused_advice(&job(login(), sample_line("x")), false, &tidy, "The key file id_ed25519 is locked with a passphrase").contains("store the passphrase in Settings"));
    }

    #[test]
    fn a_failed_login_says_what_to_do() {
        let message = "Could not log in to 192.168.68.54 as reuben. The key file id_rsa was not accepted. Check the user name, and that this PC's public key has been added to that machine, or store a password for it.";
        let no_password = login_failed(&job(login(), sample_line("x")), message);
        assert!(no_password.contains("no password is typed or stored") && no_password.contains(message));

        let mut typed = job(Login { password: Some("pw".into()), ..login() }, sample_line("x"));
        typed.password_typed = true;
        let refused = "Could not log in to 192.168.68.54 as reuben. The key file id_rsa and the stored password were not accepted. Check the user name and the password.";
        let text = login_failed(&typed, refused);
        assert!(text.contains("the password typed in the form were not accepted"), "{text}");
        assert!(!text.contains("pw"), "the password itself is never in a message");

        let root = job(Login { user: "root".into(), password: Some("pw".into()), ..login() }, sample_line("x"));
        assert!(login_failed(&root, refused).contains("PermitRootLogin"));

        let keys_only = "Could not log in to 192.168.68.54 as reuben. The key file id_rsa was not accepted. This machine does not take passwords, only keys.";
        assert!(login_failed(&typed, keys_only).contains("PasswordAuthentication no"));
    }

    #[test]
    fn text_from_the_machine_is_cleaned() {
        assert_eq!(clean("a\u{1b}[31mb\tc\r\n"), "a [31mb c");
        assert_eq!(clean(&"x".repeat(1000)).len(), 300);
        assert!(others_can_write("drwxrwxr-x"));
        assert!(others_can_write("drwxr-xrwx"));
        assert!(!others_can_write("drwxr-xr-x"));
        assert!(!others_can_write("short"));
    }

    /// Whole runs against a small SSH server in this process that runs commands with
    /// `sh`, with HOME in a scratch folder, and lets in the keys in its authorized_keys.
    #[cfg(unix)]
    mod end_to_end {
        use super::*;
        use russh::server::{self, Auth, Msg, PublicKeyOffer, VerifiedPublicKey};
        use russh::{Channel, ChannelId, ChannelMsg, MethodKind, MethodSet};
        use russh::keys::ssh_key::private::Ed25519Keypair;

        const USER: &str = "reuben";
        const PASSWORD: &str = "correct horse battery";

        struct Server {
            port: u16,
            host_key: HostKey,
            passwords: Arc<Mutex<Vec<String>>>,
        }

        async fn start(home: PathBuf) -> Server {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server_key = PrivateKey::from(Ed25519Keypair::from_seed(&[1; 32]));
            let host_key = HostKey {
                algorithm: server_key.algorithm().to_string(),
                fingerprint: fingerprint(server_key.public_key()),
            };
            let methods = MethodSet::from(&[MethodKind::PublicKey, MethodKind::Password][..]);
            let config = Arc::new(server::Config {
                keys: vec![server_key],
                methods: methods.clone(),
                auth_rejection_time: Duration::ZERO,
                auth_rejection_time_initial: Some(Duration::ZERO),
                ..Default::default()
            });
            let passwords: Arc<Mutex<Vec<String>>> = Arc::default();
            let noted = passwords.clone();
            tokio::spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    let handler = Handler { home: home.clone(), methods: methods.clone(), passwords: noted.clone() };
                    let config = config.clone();
                    tokio::spawn(async move {
                        if let Ok(session) = server::run_stream(config, socket, handler).await {
                            let _ = session.await;
                        }
                    });
                }
            });
            Server { port, host_key, passwords }
        }

        struct Handler {
            home: PathBuf,
            methods: MethodSet,
            passwords: Arc<Mutex<Vec<String>>>,
        }

        impl Handler {
            fn answer(&self, accept: bool) -> Auth {
                if accept {
                    Auth::Accept
                } else {
                    Auth::Reject { proceed_with_methods: Some(self.methods.clone()), partial_success: false }
                }
            }

            /// Like sshd: is this key in the user's authorized_keys?
            fn authorized(&self, user: &str, key: &PublicKey) -> bool {
                let text = std::fs::read_to_string(self.home.join(".ssh/authorized_keys")).unwrap_or_default();
                user == USER
                    && text.lines().any(|line| {
                        PublicKey::from_openssh(line.trim()).is_ok_and(|k| fingerprint(&k) == fingerprint(key))
                    })
            }
        }

        impl server::Handler for Handler {
            type Error = russh::Error;

            async fn auth_none(&mut self, _user: &str) -> std::result::Result<Auth, russh::Error> {
                Ok(self.answer(false))
            }

            async fn auth_password(&mut self, user: &str, password: &str) -> std::result::Result<Auth, russh::Error> {
                self.passwords.lock().unwrap().push(password.to_string());
                Ok(self.answer(user == USER && password == PASSWORD))
            }

            async fn auth_publickey_offered(
                &mut self,
                user: &str,
                offer: PublicKeyOffer<'_>,
            ) -> std::result::Result<Auth, russh::Error> {
                Ok(self.answer(self.authorized(user, offer.public_key)))
            }

            async fn auth_publickey(
                &mut self,
                user: &str,
                key: VerifiedPublicKey<'_>,
            ) -> std::result::Result<Auth, russh::Error> {
                Ok(self.answer(self.authorized(user, key.public_key)))
            }

            async fn channel_open_session(
                &mut self,
                channel: Channel<Msg>,
                reply: server::ChannelOpenHandle,
                _session: &mut server::Session,
            ) -> std::result::Result<(), russh::Error> {
                reply.accept().await;
                tokio::spawn(run_command(channel, self.home.clone()));
                Ok(())
            }

            async fn exec_request(
                &mut self,
                channel: ChannelId,
                _data: &[u8],
                session: &mut server::Session,
            ) -> std::result::Result<(), russh::Error> {
                session.channel_success(channel)
            }
        }

        /// Run what the client asked for with `sh -c`, its stdin fed from the channel.
        async fn run_command(mut channel: Channel<Msg>, home: PathBuf) {
            let mut command = Vec::new();
            let mut stdin = Vec::new();
            while let Some(msg) = channel.wait().await {
                match msg {
                    ChannelMsg::Exec { command: c, .. } => command = c,
                    ChannelMsg::Data { data } => stdin.extend_from_slice(&data),
                    ChannelMsg::Eof => break,
                    _ => {}
                }
            }
            let output = tokio::task::spawn_blocking(move || {
                use std::io::Write;
                let mut child = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(String::from_utf8_lossy(&command).into_owned())
                    .env("HOME", &home)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .spawn()
                    .unwrap();
                child.stdin.take().unwrap().write_all(&stdin).unwrap();
                child.wait_with_output().unwrap()
            })
            .await
            .unwrap();
            let _ = channel.data_bytes(output.stdout).await;
            let _ = channel.extended_data_bytes(1, output.stderr).await;
            let _ = channel.exit_status(output.status.code().unwrap_or(1) as u32).await;
            let _ = channel.eof().await;
            let _ = channel.close().await;
        }

        fn job_for(server: &Server, client: &Path, key: &KeyLine) -> Job {
            Job {
                login: Login {
                    host: "127.0.0.1".into(),
                    port: server.port,
                    user: USER.into(),
                    known_key: Some(server.host_key.clone()),
                    key_path: Some(client.join(".ssh/id_ed25519")),
                    key_passphrase: None,
                    password: Some(PASSWORD.into()),
                },
                password_held_back: false,
                password_typed: true,
                key: key.clone(),
                os: Os::Linux,
                name: "proxmox".into(),
            }
        }

        fn never(key: &HostKey) {
            panic!("first_use was called with {key:?}");
        }

        #[tokio::test]
        async fn the_password_adds_the_key_and_then_the_key_alone_gets_in() {
            let remote = Scratch::new("remote");
            let client = Scratch::new("client");
            create_key_in(&client.0.join(".ssh"), "jim@main-pc", None).unwrap();
            let local = find_local_key(None, Some(&client.0), &no_passphrase);
            let key = parse_key_line(local.public_key.as_deref().unwrap()).unwrap();
            let server = start(remote.0.clone()).await;

            let job = job_for(&server, &client.0, &key);
            let outcome = run(&job, &never).await.unwrap();
            let KeySetupOutcome::Done { file, already_there, key_login_works, windows, message, advice } = outcome else {
                panic!("{outcome:?}")
            };
            assert!(key_login_works, "{message} {advice:?}");
            assert!(!already_there && !windows);
            assert_eq!(file, remote.0.join(".ssh/authorized_keys").display().to_string());
            assert!(message.starts_with("Key login works."), "{message}");
            assert_eq!(*server.passwords.lock().unwrap(), [PASSWORD], "the password was sent once");
            let lines = std::fs::read_to_string(&file).unwrap();
            assert_eq!(lines, format!("{}\n", key.line));

            // Again, with no password at all: the key gets in by itself, and nothing is added twice.
            let again = Job { login: Login { password: None, ..job.login.clone() }, password_typed: false, ..job };
            let outcome = run(&again, &never).await.unwrap();
            assert!(matches!(outcome, KeySetupOutcome::Done { already_there: true, key_login_works: true, .. }), "{outcome:?}");
            assert_eq!(server.passwords.lock().unwrap().len(), 1, "no password the second time");
            assert_eq!(std::fs::read_to_string(&file).unwrap(), lines);
        }

        #[tokio::test]
        async fn a_password_is_never_sent_to_an_identity_nobody_approved() {
            let remote = Scratch::new("remote-new");
            let client = Scratch::new("client-new");
            create_key_in(&client.0.join(".ssh"), "jim@main-pc", None).unwrap();
            let key = parse_key_line(&find_local_key(None, Some(&client.0), &no_passphrase).public_key.unwrap()).unwrap();
            let server = start(remote.0.clone()).await;

            // First contact: the password is held back, as setup_key_login does it.
            let mut job = job_for(&server, &client.0, &key);
            job.login.known_key = None;
            job.login.password = None;
            job.password_held_back = true;
            let remembered = Mutex::new(Vec::new());
            let outcome = run(&job, &|k: &HostKey| remembered.lock().unwrap().push(k.clone())).await.unwrap();
            assert_eq!(
                outcome,
                KeySetupOutcome::Approve { offered_key: OfferedKey::from(&server.host_key), expected_fingerprint: None }
            );
            assert_eq!(*remembered.lock().unwrap(), [server.host_key.clone()], "remembered, to notice a change");
            assert!(server.passwords.lock().unwrap().is_empty(), "no password was sent");
            assert!(!remote.0.join(".ssh/authorized_keys").exists());

            // A machine that shows another identity than the stored one: nothing is sent either.
            job.login.known_key = Some(HostKey { algorithm: "ssh-ed25519".into(), fingerprint: "SHA256:old".into() });
            let outcome = run(&job, &never).await.unwrap();
            assert_eq!(
                outcome,
                KeySetupOutcome::Approve {
                    offered_key: OfferedKey::from(&server.host_key),
                    expected_fingerprint: Some("SHA256:old".into())
                }
            );
            assert!(server.passwords.lock().unwrap().is_empty());
        }

        #[tokio::test]
        async fn a_wrong_password_is_a_plain_failure() {
            let remote = Scratch::new("remote-wrong");
            let client = Scratch::new("client-wrong");
            create_key_in(&client.0.join(".ssh"), "jim@main-pc", None).unwrap();
            let key = parse_key_line(&find_local_key(None, Some(&client.0), &no_passphrase).public_key.unwrap()).unwrap();
            let server = start(remote.0.clone()).await;
            let mut job = job_for(&server, &client.0, &key);
            job.login.password = Some("hunter2-guess".into());
            let error = run(&job, &never).await.unwrap_err();
            assert!(error.contains("the password typed in the form were not accepted"), "{error}");
            assert!(!error.contains("hunter2"), "the password itself is never in a message");
            assert!(!remote.0.join(".ssh/authorized_keys").exists());
        }
    }
}
