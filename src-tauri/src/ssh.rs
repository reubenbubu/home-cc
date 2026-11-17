//! SSH connections to the owner's machines. Step 3 uses it to run one command
//! and read the answer; terminals (step 4) will build on the same code.
//!
//! [`connect`] checks the server's identity first and only then logs in, with
//! whatever works: keys held by an SSH agent, key files, then the stored
//! password. Everything is bounded by time limits, and text from the server is
//! treated as untrusted. Nothing here needs administrator rights.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use russh::client::KeyboardInteractiveAuthResponse as Asked;
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::{Algorithm, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{AgentAuthError, ChannelMsg, Disconnect, MethodKind, MethodSet, Preferred, Sig, Signer, client};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout, timeout_at};

/// A server's identity. Stored the first time a machine is reached and
/// checked on every later connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostKey {
    /// For example "ssh-ed25519".
    pub algorithm: String,
    /// For example "SHA256:…", the same text `ssh-keygen -lf` prints.
    pub fingerprint: String,
}

/// Everything needed to log in to one machine.
#[derive(Debug, Clone)]
pub struct Login {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// The identity stored for this machine, if it has been reached before.
    /// None means trust whatever the server presents (first use).
    pub known_key: Option<HostKey>,
    /// Private key file chosen in settings. None means try the usual files
    /// in the user's .ssh folder.
    pub key_path: Option<PathBuf>,
    pub key_passphrase: Option<String>,
    /// Password for this machine, if the owner stored one.
    pub password: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    /// The server's identity is not the one stored. Never ignored silently.
    #[error("This machine's identity has changed since home-cc first reached it (was {expected}, now {got})")]
    HostKeyChanged { expected: String, got: String },
    /// No key, agent or password was accepted. The text says what was tried.
    #[error("{0}")]
    AuthFailed(String),
    /// Could not connect at all (refused, timed out, name not found).
    #[error("{0}")]
    Unreachable(String),
    #[error("{0}")]
    Other(String),
}

/// Connecting and logging in together never take longer than this.
const CONNECT_LIMIT: Duration = Duration::from_secs(20);
/// A machine that is switched off answers with silence. Don't spend the whole limit on it.
const TCP_LIMIT: Duration = Duration::from_secs(8);
/// How long an SSH agent gets for one answer. An agent can sit waiting for the owner to
/// unlock a key; the other ways of logging in should still get their turn.
const AGENT_LIMIT: Duration = Duration::from_secs(5);
/// Servers hang up after six failed tries by default. Keys stop at five so that a stored
/// password still gets its turn.
const MAX_KEY_TRIES: usize = 5;
/// Each of stdout and stderr is cut off here, so a runaway command can't fill the memory.
const MAX_OUTPUT: usize = 1024 * 1024;
/// Private keys are a few kilobytes. Anything much larger was picked by mistake.
const MAX_KEY_FILE: u64 = 256 * 1024;
/// The agent holds the keys the owner uses day to day, so it goes first where one normally
/// runs. On Windows the agent service is off unless an administrator turned it on, so the
/// key files go first there.
const AGENT_FIRST: bool = cfg!(not(windows));

/// What the command printed and how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// None if the server closed the channel without one (the command was killed).
    pub exit_status: Option<u32>,
    /// True if stdout or stderr was longer than the limit and the rest was dropped.
    pub truncated: bool,
}

/// Why a command gave no output. Kept apart from [`SshError`] so callers in this crate
/// can tell a slow command (try something lighter) from a broken connection (give up).
#[derive(Debug)]
pub(crate) enum ExecError {
    TimedOut,
    Ssh(SshError),
}

/// An open, logged-in connection to one machine.
pub struct Session {
    handle: client::Handle<Handler>,
    host: String,
    host_key: HostKey,
    server_version: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("host", &self.host)
            .field("host_key", &self.host_key)
            .field("server_version", &self.server_version)
            .finish_non_exhaustive()
    }
}

/// Connect and log in. If `login.known_key` is set and the server shows a different
/// identity, this fails with [`SshError::HostKeyChanged`] before any key or password is
/// offered. If it is not set, whatever the server shows is accepted: read it from
/// [`Session::host_key`] and store it.
pub async fn connect(login: &Login) -> Result<Session, SshError> {
    let deadline = Instant::now() + CONNECT_LIMIT;
    let host = login.host.trim();
    let user = login.user.trim();
    if host.is_empty() {
        return Err(SshError::Unreachable("This machine has no address to connect to.".into()));
    }
    if user.is_empty() {
        return Err(SshError::AuthFailed(
            "No user name is set for this machine. Add one, then try again.".into(),
        ));
    }

    let stream = open_tcp(host, login.port).await?;
    let seen = Arc::new(Mutex::new(Seen::default()));
    let handler =
        Handler { expected: login.known_key.as_ref().map(|k| k.fingerprint.clone()), seen: seen.clone() };
    let config = Arc::new(config(login.known_key.as_ref()));
    // Version exchange, key exchange and the identity check. Nothing secret is sent yet.
    let mut handle = match timeout_at(deadline, client::connect_stream(config, stream, handler)).await {
        Ok(Ok(handle)) => handle,
        Ok(Err(Fail::HostKeyChanged { expected, got })) => {
            return Err(SshError::HostKeyChanged { expected, got });
        }
        Ok(Err(Fail::Russh(e))) => return Err(setup_error(host, login.port, &e)),
        Err(_) => {
            return Err(SshError::Unreachable(format!(
                "{host} accepted the connection on port {} but then went quiet. Check that this is its SSH \
                 port.",
                login.port
            )));
        }
    };
    let (host_key, server_version) = {
        let seen = lock(&seen);
        (seen.key.clone(), seen.server_version.clone())
    };
    let Some(host_key) = host_key else {
        return Err(SshError::Other(format!("{host} did not show its identity, so home-cc did not log in.")));
    };

    let mut auth = Auth::new(&mut handle, user);
    let logged_in = match timeout_at(deadline, auth.run(login)).await {
        Ok(logged_in) => logged_in,
        Err(_) => {
            return Err(SshError::Other(format!(
                "Logging in to {host} took too long. Try again; if it keeps happening, restart that \
                 machine's SSH server."
            )));
        }
    };
    if !logged_in {
        let password_stored = login.password.as_deref().is_some_and(|p| !p.is_empty());
        let windows_server = server_version.contains("Windows");
        let message = auth_message(host, user, &auth.tried, password_stored, windows_server);
        say_goodbye(&handle).await;
        return Err(SshError::AuthFailed(message));
    }
    Ok(Session { handle, host: host.to_string(), host_key, server_version })
}

impl Session {
    /// The identity the server showed. Store it if the machine had none stored.
    pub fn host_key(&self) -> &HostKey {
        &self.host_key
    }

    /// The server's greeting, for example "SSH-2.0-OpenSSH_for_Windows_9.5".
    pub fn server_version(&self) -> &str {
        &self.server_version
    }

    /// False once the connection has dropped. Connect again to carry on.
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Run one command without a terminal and wait for it to end. `stdin`, if given, is
    /// sent and then closed, so a script can be piped into `sh -s` with no quoting at all.
    /// A command still running after `limit` is killed and reported as an error. Don't
    /// start background jobs in the command: the channel stays open while they hold it.
    pub async fn exec(
        &self,
        command: &str,
        stdin: Option<&[u8]>,
        limit: Duration,
    ) -> Result<ExecOutput, SshError> {
        self.try_exec(command, stdin, limit).await.map_err(|e| match e {
            ExecError::TimedOut => SshError::Other(format!("{} took too long to answer.", self.host)),
            ExecError::Ssh(e) => e,
        })
    }

    pub(crate) async fn try_exec(
        &self,
        command: &str,
        stdin: Option<&[u8]>,
        limit: Duration,
    ) -> Result<ExecOutput, ExecError> {
        let deadline = Instant::now() + limit;
        let mut channel = match timeout_at(deadline, self.handle.channel_open_session()).await {
            Ok(Ok(channel)) => channel,
            Ok(Err(e)) => return Err(ExecError::Ssh(self.lost(&e))),
            Err(_) => return Err(ExecError::TimedOut),
        };
        match timeout_at(deadline, run_command(&mut channel, command, stdin)).await {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(CommandError::Refused)) => {
                Err(ExecError::Ssh(SshError::Other(format!("{} refused to run the command.", self.host))))
            }
            Ok(Err(CommandError::Lost)) => Err(ExecError::Ssh(self.lost(&russh::Error::Disconnect))),
            Ok(Err(CommandError::Russh(e))) => Err(ExecError::Ssh(self.lost(&e))),
            Err(_) => {
                // Without a terminal the remote program keeps running after we stop
                // listening, so ask the server to kill it (older servers ignore this).
                let _ = timeout(Duration::from_secs(1), channel.signal(Sig::KILL)).await;
                let _ = timeout(Duration::from_secs(1), channel.close()).await;
                Err(ExecError::TimedOut)
            }
        }
    }

    /// A fresh channel on this connection, for callers that need more than `exec`
    /// (a terminal asks for a PTY and a shell on it).
    pub async fn open_channel(&self) -> Result<russh::Channel<client::Msg>, SshError> {
        self.handle.channel_open_session().await.map_err(|e| self.lost(&e))
    }

    /// Log out. Dropping the session also closes the connection, without the goodbye.
    pub async fn close(self) {
        say_goodbye(&self.handle).await;
    }

    fn lost(&self, e: &russh::Error) -> SshError {
        if self.handle.is_closed() || matches!(e, russh::Error::Disconnect) {
            SshError::Other(format!("The connection to {} was lost.", self.host))
        } else {
            SshError::Other(format!("{} would not start the command ({e}).", self.host))
        }
    }
}

async fn say_goodbye(handle: &client::Handle<Handler>) {
    let _ = timeout(Duration::from_secs(2), handle.disconnect(Disconnect::ByApplication, "", "en")).await;
}

enum CommandError {
    /// The server said no to the request itself.
    Refused,
    /// The connection went away while the command was running.
    Lost,
    Russh(russh::Error),
}

impl From<russh::Error> for CommandError {
    fn from(e: russh::Error) -> Self {
        CommandError::Russh(e)
    }
}

async fn run_command(
    channel: &mut russh::Channel<client::Msg>,
    command: &str,
    stdin: Option<&[u8]>,
) -> Result<ExecOutput, CommandError> {
    channel.exec(true, command).await?;
    if let Some(input) = stdin {
        channel.data(input).await?;
    }
    // Always close our side of stdin, or a remote program that reads it never ends.
    channel.eof().await?;

    let mut output = ExecOutput::default();
    loop {
        // The server always closes the channel when the command is over. No more
        // messages without that means the whole connection is gone.
        let Some(msg) = channel.wait().await else { return Err(CommandError::Lost) };
        match msg {
            ChannelMsg::Data { data } => output.truncated |= push_capped(&mut output.stdout, &data),
            // 1 is the number SSH gives to stderr.
            ChannelMsg::ExtendedData { data, ext: 1 } => {
                output.truncated |= push_capped(&mut output.stderr, &data)
            }
            // The exit status can arrive before the last of the output; keep reading.
            ChannelMsg::ExitStatus { exit_status } => output.exit_status = Some(exit_status),
            ChannelMsg::Failure => return Err(CommandError::Refused),
            ChannelMsg::Close => return Ok(output),
            _ => {}
        }
    }
}

/// Add what fits under the limit. True if some of it had to be dropped.
fn push_capped(buffer: &mut Vec<u8>, data: &[u8]) -> bool {
    let room = MAX_OUTPUT.saturating_sub(buffer.len());
    buffer.extend(data.iter().take(room));
    data.len() > room
}

/// Find the machine and open a plain connection to it.
async fn open_tcp(host: &str, port: u16) -> Result<TcpStream, SshError> {
    let silent = || {
        SshError::Unreachable(format!(
            "{host} did not answer. Check that it is switched on and on the network."
        ))
    };
    let not_found = || {
        SshError::Unreachable(format!(
            "The name {host} was not found. Check the spelling, or use the machine's IP address."
        ))
    };
    let attempt = async {
        let addrs = tokio::net::lookup_host((host, port)).await.map_err(|_| not_found())?;
        let mut last = None;
        for addr in addrs {
            match TcpStream::connect(addr).await {
                Ok(stream) => return Ok(stream),
                Err(e) => last = Some(e),
            }
        }
        Err(match last {
            Some(e) => tcp_error(host, port, &e),
            None => not_found(),
        })
    };
    let stream = timeout(TCP_LIMIT, attempt).await.unwrap_or_else(|_| Err(silent()))?;
    // Terminals send single key presses; don't let the system hold them back to batch them.
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

fn tcp_error(host: &str, port: u16, e: &std::io::Error) -> SshError {
    use std::io::ErrorKind::*;
    SshError::Unreachable(match e.kind() {
        ConnectionRefused => format!(
            "{host} refused the connection on port {port}. Check that SSH is turned on there (on Windows: \
             the OpenSSH Server feature) and that the port is right."
        ),
        TimedOut | HostUnreachable | NetworkUnreachable => {
            format!("{host} did not answer. Check that it is switched on and on the network.")
        }
        _ => {
            format!("This PC could not connect to {host} on port {port}. Check its address and the network.")
        }
    })
}

/// A failure between the connection opening and the login starting.
fn setup_error(host: &str, port: u16, e: &russh::Error) -> SshError {
    use russh::Error::*;
    match e {
        IO(_) | Disconnect | HUP | Version | ConnectionTimeout | KeepaliveTimeout | InactivityTimeout => {
            SshError::Unreachable(format!(
                "{host} accepted the connection on port {port} but closed it before the login. Check that \
                 this is its SSH port."
            ))
        }
        NoCommonAlgo { .. } | KexInit | Kex | UnknownAlgo => SshError::Other(format!(
            "{host} uses SSH settings that home-cc can't work with. Its SSH server may be very old."
        )),
        UnknownKey => SshError::Other(format!(
            "{host} showed a kind of identity that home-cc does not support, so home-cc did not log in."
        )),
        other => SshError::Other(format!("The secure connection to {host} could not be set up ({other}).")),
    }
}

fn config(known: Option<&HostKey>) -> client::Config {
    let mut preferred = Preferred::DEFAULT;
    if let Some(known) = known {
        preferred.key = host_key_preference(&known.algorithm);
    }
    client::Config {
        preferred,
        // A small message every 20 seconds keeps a quiet connection open and notices a
        // machine that has vanished after about 80 seconds. While those are answered the
        // inactivity limit never fires; it is a backstop.
        keepalive_interval: Some(Duration::from_secs(20)),
        keepalive_max: 3,
        inactivity_timeout: Some(Duration::from_secs(120)),
        ..Default::default()
    }
}

/// The usual list of identity types with the stored one moved to the front. A machine
/// often has several identities; asking for the stored type first means one that gains
/// another type later doesn't look like a changed machine.
fn host_key_preference(stored: &str) -> Cow<'static, [Algorithm]> {
    let all = Preferred::DEFAULT.key;
    // Servers offer a stored "ssh-rsa" identity under three names, one per signature kind.
    let wanted = |a: &Algorithm| {
        if stored == "ssh-rsa" { matches!(a, Algorithm::Rsa { .. }) } else { a.as_str() == stored }
    };
    let mut list: Vec<Algorithm> = all.iter().filter(|a| wanted(a)).cloned().collect();
    list.extend(all.iter().filter(|a| !wanted(a)).cloned());
    Cow::Owned(list)
}

/// What can stop the connection before the login. russh needs its own error to convert
/// into the handler's error type, which is why this is not [`SshError`].
#[derive(Debug)]
enum Fail {
    HostKeyChanged { expected: String, got: String },
    Russh(russh::Error),
}

impl From<russh::Error> for Fail {
    fn from(e: russh::Error) -> Self {
        Fail::Russh(e)
    }
}

/// What the server showed while the connection was set up.
#[derive(Default)]
struct Seen {
    key: Option<HostKey>,
    server_version: String,
}

fn lock(seen: &Mutex<Seen>) -> MutexGuard<'_, Seen> {
    // Nothing can leave this half-written, so a poisoned lock is still good to read.
    seen.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Handler {
    /// The fingerprint the server must show. None until the first one is seen.
    expected: Option<String>,
    seen: Arc<Mutex<Seen>>,
}

impl client::Handler for Handler {
    type Error = Fail;

    /// Runs during key exchange, before any login attempt.
    async fn check_server_key(&mut self, server_key: &PublicKeyOrCertificate) -> Result<bool, Fail> {
        let key = match server_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => HostKey {
                algorithm: key.algorithm().to_string(),
                fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            },
            // Never asked for (the certificate list in the config is empty); refuse
            // rather than guess what it should be compared with.
            PublicKeyOrCertificate::Certificate(_) => return Err(Fail::Russh(russh::Error::UnknownKey)),
        };
        if let Some(expected) = &self.expected {
            if *expected != key.fingerprint {
                return Err(Fail::HostKeyChanged { expected: expected.clone(), got: key.fingerprint });
            }
        }
        // Keys are exchanged again on a long connection: the identity must stay the same.
        self.expected = Some(key.fingerprint.clone());
        lock(&self.seen).key = Some(key);
        Ok(true)
    }

    async fn kex_done(
        &mut self,
        _shared_secret: Option<&[u8]>,
        _names: &russh::Names,
        session: &mut client::Session,
    ) -> Result<(), Fail> {
        let version: String = String::from_utf8_lossy(session.remote_sshid())
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect();
        lock(&self.seen).server_version = version.trim().to_string();
        Ok(())
    }
}

/// What came of one way of logging in. Collected so the final message can say what was
/// tried and what to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tried {
    /// The server turned down this many keys held by the SSH agent.
    AgentKeys(usize),
    AgentEmpty,
    AgentUnreachable,
    /// The agent did not answer in time (it may be showing an unlock window).
    AgentSilent,
    KeyRefused(String),
    /// The key file needs a passphrase and none is stored.
    KeyLocked(String),
    KeyWrongPassphrase(String),
    KeyUnreadable(String),
    PasswordRefused,
    /// A password is stored but the server takes keys only.
    PasswordNotTaken,
    /// The server takes a password only.
    KeysNotTaken,
    /// The server closed the connection before everything had its turn.
    CutOff,
}

type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;

/// One login in progress: tries each way in turn on the same connection.
struct Auth<'a> {
    handle: &'a mut client::Handle<Handler>,
    user: &'a str,
    tried: Vec<Tried>,
    key_tries: usize,
    /// What the server said it still accepts, from its latest refusal.
    methods: Option<MethodSet>,
    /// The server stopped answering login attempts.
    gone: bool,
    /// Signature kind for RSA keys, asked for once and only if an RSA key turns up
    /// (asking can cost a second on old servers).
    rsa_hash: Option<Option<HashAlg>>,
    /// Fingerprints already offered. A key that is both in the agent and on disk is
    /// offered once, because every refusal counts towards the server's limit.
    offered: Vec<String>,
}

impl<'a> Auth<'a> {
    fn new(handle: &'a mut client::Handle<Handler>, user: &'a str) -> Self {
        Self {
            handle,
            user,
            tried: Vec::new(),
            key_tries: 0,
            methods: None,
            gone: false,
            rsa_hash: None,
            offered: Vec::new(),
        }
    }

    /// True once the server has let us in.
    async fn run(&mut self, login: &Login) -> bool {
        // Ask what the server accepts before offering anything, as ssh itself does. It
        // does not count as a failed try.
        match self.handle.authenticate_none(self.user).await {
            Ok(result) => {
                if self.accepted(result) {
                    return true;
                }
            }
            Err(_) => self.gone = true,
        }
        if !self.gone && !self.allows(MethodKind::PublicKey) {
            self.tried.push(Tried::KeysNotTaken);
        }
        let with_keys = if AGENT_FIRST {
            self.agents().await || self.key_files(login).await
        } else {
            self.key_files(login).await || self.agents().await
        };
        with_keys || self.password(login).await
    }

    fn allows(&self, method: MethodKind) -> bool {
        !self.gone && self.methods.as_ref().is_none_or(|m| m.contains(&method))
    }

    fn may_try_key(&self) -> bool {
        self.key_tries < MAX_KEY_TRIES && self.allows(MethodKind::PublicKey)
    }

    /// Take in the server's answer to one attempt. True if it let us in.
    fn accepted(&mut self, result: russh::client::AuthResult) -> bool {
        match result {
            russh::client::AuthResult::Success => true,
            russh::client::AuthResult::Failure { remaining_methods, .. } => {
                self.refused(remaining_methods);
                false
            }
        }
    }

    fn refused(&mut self, remaining: MethodSet) {
        // No way left to carry on is also what russh reports when the server hung up.
        self.gone = remaining.is_empty() || self.handle.is_closed();
        self.methods = Some(remaining);
    }

    async fn rsa_hash(&mut self) -> Option<HashAlg> {
        if self.rsa_hash.is_none() {
            // The newest kind the server lists. A server too old to send a list gets the
            // older of the two modern kinds, which is the best guess.
            let best = self.handle.best_supported_rsa_hash().await.ok().flatten();
            self.rsa_hash = Some(best.unwrap_or(Some(HashAlg::Sha256)));
        }
        self.rsa_hash.flatten()
    }

    async fn agents(&mut self) -> bool {
        if !self.may_try_key() {
            return false;
        }
        for agent in open_agents().await {
            match agent {
                Some(agent) => {
                    if self.agent(agent).await {
                        return true;
                    }
                }
                None => self.tried.push(Tried::AgentUnreachable),
            }
        }
        false
    }

    async fn agent(&mut self, mut agent: Agent) -> bool {
        let identities = match timeout(AGENT_LIMIT, agent.request_identities()).await {
            Ok(Ok(identities)) => identities,
            _ => {
                self.tried.push(Tried::AgentUnreachable);
                return false;
            }
        };
        if identities.is_empty() {
            self.tried.push(Tried::AgentEmpty);
            return false;
        }
        let mut refused = 0;
        let mut silent = false;
        for identity in identities {
            if !self.may_try_key() {
                break;
            }
            let fingerprint = identity.public_key().fingerprint(HashAlg::Sha256).to_string();
            if self.offered.contains(&fingerprint) {
                continue;
            }
            let hash = if identity.public_key().algorithm().is_rsa() { self.rsa_hash().await } else { None };
            self.key_tries += 1;
            self.offered.push(fingerprint);
            let mut signer = BoundedAgent { agent: &mut agent, silent: false };
            let result = match identity {
                AgentIdentity::PublicKey { key, .. } => {
                    self.handle.authenticate_publickey_with(self.user, key, hash, &mut signer).await
                }
                AgentIdentity::Certificate { certificate, .. } => {
                    self.handle.authenticate_certificate_with(self.user, certificate, hash, &mut signer).await
                }
            };
            silent = signer.silent;
            match result {
                Ok(result) => {
                    if self.accepted(result) {
                        return true;
                    }
                    refused += 1;
                }
                // The agent would not sign with this key. The connection is still good.
                Err(AgentAuthError::Key(_)) if !silent => {}
                // After a timeout the agent's late answer would be mistaken for the next
                // one, so this agent is not asked again.
                Err(AgentAuthError::Key(_)) => break,
                Err(AgentAuthError::Send(_)) => {
                    self.gone = true;
                    break;
                }
            }
        }
        if refused > 0 {
            self.tried.push(Tried::AgentKeys(refused));
        }
        if silent {
            self.tried.push(Tried::AgentSilent);
        }
        false
    }

    async fn key_files(&mut self, login: &Login) -> bool {
        for (path, name) in key_candidates(login.key_path.as_deref(), home_dir().as_deref()) {
            if !self.may_try_key() {
                break;
            }
            let passphrase = login.key_passphrase.clone();
            // Unlocking a key with a passphrase is slow on purpose; keep it off the
            // threads that serve the screens.
            let loaded = tokio::task::spawn_blocking(move || load_key(&path, passphrase.as_deref())).await;
            let key = match loaded {
                Ok(Ok(key)) => key,
                Ok(Err(KeyProblem::Locked)) => {
                    self.tried.push(Tried::KeyLocked(name));
                    continue;
                }
                Ok(Err(KeyProblem::WrongPassphrase)) => {
                    self.tried.push(Tried::KeyWrongPassphrase(name));
                    continue;
                }
                Ok(Err(KeyProblem::Unreadable)) | Err(_) => {
                    self.tried.push(Tried::KeyUnreadable(name));
                    continue;
                }
            };
            let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
            if self.offered.contains(&fingerprint) {
                continue;
            }
            let hash = if key.algorithm().is_rsa() { self.rsa_hash().await } else { None };
            self.key_tries += 1;
            self.offered.push(fingerprint);
            let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
            match self.handle.authenticate_publickey(self.user, key).await {
                Ok(result) => {
                    if self.accepted(result) {
                        return true;
                    }
                    self.tried.push(Tried::KeyRefused(name));
                }
                Err(_) => {
                    self.gone = true;
                    break;
                }
            }
        }
        false
    }

    async fn password(&mut self, login: &Login) -> bool {
        let password = login.password.as_deref().filter(|p| !p.is_empty());
        let (Some(password), false) = (password, self.gone) else {
            if self.gone {
                self.tried.push(Tried::CutOff);
            }
            return false;
        };
        let logged_in = if self.allows(MethodKind::Password) {
            match self.handle.authenticate_password(self.user, password).await {
                Ok(result) => self.accepted(result),
                Err(_) => {
                    self.tried.push(Tried::CutOff);
                    return false;
                }
            }
        } else if self.allows(MethodKind::KeyboardInteractive) {
            self.typed_password(password).await
        } else {
            self.tried.push(Tried::PasswordNotTaken);
            return false;
        };
        if !logged_in {
            self.tried.push(Tried::PasswordRefused);
        }
        logged_in
    }

    /// Some servers take the password only as the typed answer to a question (their
    /// "keyboard-interactive" way). Answer the hidden-input question with it.
    async fn typed_password(&mut self, password: &str) -> bool {
        let mut reply = self.handle.authenticate_keyboard_interactive_start(self.user, None::<String>).await;
        let mut answered = false;
        // A server may send a notice with no question first; a few rounds are plenty.
        for _ in 0..4 {
            match reply {
                Ok(Asked::Success) => return true,
                Ok(Asked::Failure { remaining_methods, .. }) => {
                    self.refused(remaining_methods);
                    return false;
                }
                Ok(Asked::InfoRequest { prompts, .. }) => {
                    let hidden = prompts.iter().filter(|p| !p.echo).count();
                    // Asked again after an answer means the password was wrong. More than
                    // one hidden question is something else (a code from a phone, say).
                    if hidden > 1 || (hidden == 1 && answered) {
                        return false;
                    }
                    answered |= hidden == 1;
                    let answers = prompts
                        .iter()
                        .map(|p| if p.echo { String::new() } else { password.to_string() })
                        .collect();
                    reply = self.handle.authenticate_keyboard_interactive_respond(answers).await;
                }
                Err(_) => {
                    self.gone = true;
                    return false;
                }
            }
        }
        false
    }
}

/// Gives the agent a time limit for each signature. Without it an agent that is waiting
/// for the owner would hold up the whole login.
struct BoundedAgent<'a> {
    agent: &'a mut Agent,
    silent: bool,
}

impl Signer for BoundedAgent<'_> {
    type Error = AgentAuthError;

    async fn auth_sign(
        &mut self,
        key: &AgentIdentity,
        hash_alg: Option<HashAlg>,
        to_sign: Vec<u8>,
    ) -> Result<Vec<u8>, AgentAuthError> {
        match timeout(AGENT_LIMIT, self.agent.sign_request(key, hash_alg, to_sign)).await {
            Ok(result) => result.map_err(AgentAuthError::Key),
            Err(_) => {
                self.silent = true;
                Err(AgentAuthError::Key(russh::keys::Error::AgentFailure))
            }
        }
    }
}

/// The agents to ask for keys, in order. None stands for one that is set up but could
/// not be reached.
#[cfg(unix)]
async fn open_agents() -> Vec<Option<Agent>> {
    let Some(socket) = std::env::var_os("SSH_AUTH_SOCK").filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    match timeout(AGENT_LIMIT, AgentClient::connect_uds(socket)).await {
        Ok(Ok(agent)) => vec![Some(agent.dynamic())],
        _ => vec![None],
    }
}

/// The agents to ask for keys, in order: the Windows OpenSSH agent service, then PuTTY's
/// Pageant. Neither runs unless the owner set it up, so one that is missing is skipped
/// without a word.
#[cfg(windows)]
async fn open_agents() -> Vec<Option<Agent>> {
    let mut agents = Vec::new();
    // SSH_AUTH_SOCK may name another pipe, as it does for ssh.exe.
    let pipe = std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|s| s.starts_with(r"\\.\pipe\"))
        .unwrap_or_else(|| r"\\.\pipe\openssh-ssh-agent".to_string());
    // Connecting retries for as long as the pipe is busy, so it needs its own limit.
    if let Ok(Ok(agent)) = timeout(AGENT_LIMIT, AgentClient::connect_named_pipe(&pipe)).await {
        agents.push(Some(agent.dynamic()));
    }
    if let Ok(Ok(agent)) = timeout(AGENT_LIMIT, AgentClient::connect_pageant()).await {
        agents.push(Some(agent.dynamic()));
    }
    agents
}

#[cfg(not(any(unix, windows)))]
async fn open_agents() -> Vec<Option<Agent>> {
    Vec::new()
}

/// The user's home folder: from HOME on Linux and macOS, from the user profile on Windows.
fn home_dir() -> Option<PathBuf> {
    std::env::home_dir().filter(|p| !p.as_os_str().is_empty())
}

/// The key files to try, each with the name to call it by in messages: the file chosen
/// in Settings, or else the usual ones that exist in the user's .ssh folder.
fn key_candidates(chosen: Option<&Path>, home: Option<&Path>) -> Vec<(PathBuf, String)> {
    if let Some(chosen) = chosen.filter(|p| !p.as_os_str().is_empty()) {
        // "~/.ssh/key" is how people write it; nothing expands it for us.
        let path = match (chosen.strip_prefix("~"), home) {
            (Ok(rest), Some(home)) => home.join(rest),
            _ => chosen.to_path_buf(),
        };
        let name = path.display().to_string();
        return vec![(path, name)];
    }
    let Some(home) = home else { return Vec::new() };
    ["id_ed25519", "id_ecdsa", "id_rsa"]
        .iter()
        .map(|name| (home.join(".ssh").join(name), name.to_string()))
        .filter(|(path, _)| path.is_file())
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum KeyProblem {
    /// Needs a passphrase and none was given.
    Locked,
    WrongPassphrase,
    /// Missing, not a private key, or a kind that isn't supported.
    Unreadable,
}

fn load_key(path: &Path, passphrase: Option<&str>) -> Result<PrivateKey, KeyProblem> {
    let size = std::fs::metadata(path).map_err(|_| KeyProblem::Unreadable)?.len();
    if size > MAX_KEY_FILE {
        return Err(KeyProblem::Unreadable);
    }
    let text = std::fs::read_to_string(path).map_err(|_| KeyProblem::Unreadable)?;
    decode_key(&text, passphrase)
}

/// Read a private key in any of the usual formats (OpenSSH, PEM, PKCS#8, PuTTY .ppk).
fn decode_key(text: &str, passphrase: Option<&str>) -> Result<PrivateKey, KeyProblem> {
    use russh::keys::decode_secret_key;
    // Without a passphrase first: it is quick, and it tells a locked key from a broken one.
    let problem = match decode_secret_key(text, None) {
        Ok(key) => return Ok(key),
        Err(e) => e,
    };
    if !matches!(problem, russh::keys::Error::KeyIsEncrypted) && !looks_locked(text) {
        return Err(KeyProblem::Unreadable);
    }
    match passphrase.filter(|p| !p.is_empty()) {
        Some(passphrase) => {
            decode_secret_key(text, Some(passphrase)).map_err(|_| KeyProblem::WrongPassphrase)
        }
        None => Err(KeyProblem::Locked),
    }
}

/// Not every format reports "needs a passphrase" as such, so also look for the marks
/// that locked PEM, PKCS#8 and PuTTY files carry.
fn looks_locked(text: &str) -> bool {
    text.lines().take(12).any(|line| {
        let line = line.trim();
        line.contains("ENCRYPTED")
            || line.strip_prefix("Encryption:").is_some_and(|kind| kind.trim() != "none")
    })
}

/// The sentence shown when nothing was accepted: what was tried, what stood in the way,
/// and the most useful next step. Never includes a password or passphrase.
fn auth_message(
    host: &str,
    user: &str,
    tried: &[Tried],
    password_stored: bool,
    windows_server: bool,
) -> String {
    let mut refused: Vec<String> = Vec::new();
    let mut several = false;
    let mut notes: Vec<String> = Vec::new();
    for attempt in tried {
        match attempt {
            Tried::AgentKeys(1) => refused.push("1 key from the SSH agent".into()),
            Tried::AgentKeys(n) => {
                several = true;
                refused.push(format!("{n} keys from the SSH agent"));
            }
            Tried::KeyRefused(name) => refused.push(format!("the key file {name}")),
            Tried::PasswordRefused => refused.push("the stored password".into()),
            Tried::KeyLocked(name) => notes.push(format!(
                "The key file {name} is locked with a passphrase: store the passphrase in Settings."
            )),
            Tried::KeyWrongPassphrase(name) => notes.push(format!(
                "The passphrase stored in Settings does not open the key file {name}: store it again."
            )),
            Tried::KeyUnreadable(name) => notes.push(format!(
                "The key file {name} could not be read as a private key: check the key file in Settings."
            )),
            Tried::PasswordNotTaken => notes.push("This machine does not take passwords, only keys.".into()),
            Tried::KeysNotTaken if !password_stored => {
                notes.push("This machine takes a password only, not keys: store a password for it.".into())
            }
            Tried::KeysNotTaken => {}
            Tried::AgentSilent => notes.push(
                "The SSH agent did not answer in time; it may be waiting for you to unlock a key.".into(),
            ),
            Tried::CutOff => {
                notes.push("The machine closed the connection before everything could be tried.".into())
            }
            // Not worth a sentence: the other ways still had their turn.
            Tried::AgentEmpty | Tried::AgentUnreachable => {}
        }
    }
    several |= refused.len() > 1;

    let mut text = format!("Could not log in to {host} as {user}.");
    if !refused.is_empty() {
        let list = join_list(&refused);
        let mut chars = list.chars();
        let list: String = chars.next().into_iter().flat_map(char::to_uppercase).chain(chars).collect();
        text += &format!(" {list} {} not accepted.", if several { "were" } else { "was" });
    }
    for note in &notes {
        text.push(' ');
        text += note;
    }

    let key_refused = tried.iter().any(|t| matches!(t, Tried::AgentKeys(_) | Tried::KeyRefused(_)));
    if tried.contains(&Tried::PasswordRefused) {
        text += " Check the user name and the password.";
    } else if key_refused {
        text += " Check the user name, and that this PC's public key has been added to that machine";
        text += if password_stored { "." } else { ", or store a password for it." };
        if windows_server {
            text += " On Windows, administrator accounts keep their keys in \
                     C:\\ProgramData\\ssh\\administrators_authorized_keys.";
        }
    } else if notes.is_empty() {
        text += " No SSH key was found on this PC and no password is stored for this machine. Choose a key \
                 file in Settings or store a password for this machine.";
    }
    text
}

/// "a", "a and b", "a, b and c".
fn join_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login() -> Login {
        Login {
            host: "127.0.0.1".into(),
            port: 22,
            user: "jim".into(),
            known_key: None,
            key_path: None,
            key_passphrase: None,
            password: None,
        }
    }

    #[test]
    fn stored_identity_type_is_asked_for_first() {
        let names = |stored: &str| -> Vec<String> {
            host_key_preference(stored).iter().map(|a| a.as_str().to_string()).collect()
        };
        let default = names("something-unknown");
        assert_eq!(
            default.first().map(String::as_str),
            Some("ssh-ed25519"),
            "the usual order starts with ed25519"
        );

        let ecdsa = names("ecdsa-sha2-nistp256");
        assert_eq!(ecdsa.first().map(String::as_str), Some("ecdsa-sha2-nistp256"));
        assert_eq!(ecdsa.len(), default.len(), "nothing is dropped, only reordered");

        // A stored RSA identity is offered under three names; all of them go first.
        let rsa = names("ssh-rsa");
        assert_eq!(rsa.iter().take(3).collect::<Vec<_>>(), ["rsa-sha2-512", "rsa-sha2-256", "ssh-rsa"]);
        assert_eq!(rsa.len(), default.len());
    }

    #[test]
    fn output_is_cut_at_the_limit() {
        let mut buffer = vec![0u8; MAX_OUTPUT - 3];
        assert!(!push_capped(&mut buffer, b"ab"));
        assert!(push_capped(&mut buffer, b"cdef"), "only one more byte fits");
        assert_eq!(buffer.len(), MAX_OUTPUT);
        assert!(push_capped(&mut buffer, b"x"));
        assert_eq!(buffer.len(), MAX_OUTPUT);
        assert!(!push_capped(&mut buffer, b""));
    }

    #[test]
    fn connection_failures_read_as_plain_advice() {
        use std::io::{Error, ErrorKind};
        let refused = tcp_error("nas", 22, &Error::from(ErrorKind::ConnectionRefused));
        assert!(
            matches!(&refused, SshError::Unreachable(m) if m.contains("refused") && m.contains("port 22"))
        );
        let silent = tcp_error("nas", 22, &Error::from(ErrorKind::TimedOut));
        assert!(matches!(&silent, SshError::Unreachable(m) if m.contains("did not answer")));
        let odd = tcp_error("nas", 22, &Error::from(ErrorKind::PermissionDenied));
        assert!(matches!(&odd, SshError::Unreachable(m) if m.contains("could not connect")));

        // A port that answers but isn't SSH, or hangs up during the handshake.
        for e in [russh::Error::Version, russh::Error::Disconnect, russh::Error::HUP] {
            assert!(
                matches!(setup_error("nas", 8080, &e), SshError::Unreachable(m) if m.contains("port 8080"))
            );
        }
        assert!(matches!(setup_error("nas", 22, &russh::Error::KexInit), SshError::Other(_)));
    }

    #[test]
    fn chosen_key_file_replaces_the_usual_ones() {
        let home = std::env::temp_dir().join(format!("home-cc-ssh-test-{}", std::process::id()));
        let ssh = home.join(".ssh");
        std::fs::create_dir_all(&ssh).unwrap();
        std::fs::write(ssh.join("id_rsa"), "x").unwrap();
        std::fs::write(ssh.join("id_ed25519"), "x").unwrap();
        std::fs::write(ssh.join("id_other"), "x").unwrap();

        let usual = key_candidates(None, Some(&home));
        let names: Vec<&str> = usual.iter().map(|(_, name)| name.as_str()).collect();
        assert_eq!(names, ["id_ed25519", "id_rsa"], "only the usual names that exist, best kind first");
        assert_eq!(usual.first().map(|(p, _)| p.clone()), Some(ssh.join("id_ed25519")));
        // An empty path in settings means "not chosen".
        assert_eq!(key_candidates(Some(Path::new("")), Some(&home)), usual);

        let chosen = key_candidates(Some(Path::new("/keys/mine")), Some(&home));
        assert_eq!(chosen, [(PathBuf::from("/keys/mine"), "/keys/mine".to_string())]);
        let tilde = key_candidates(Some(Path::new("~/.ssh/id_other")), Some(&home));
        assert_eq!(tilde.first().map(|(p, _)| p.clone()), Some(ssh.join("id_other")));
        // "~name" is another user's home, which nothing here can look up.
        let other = key_candidates(Some(Path::new("~bob/key")), Some(&home));
        assert_eq!(other.first().map(|(p, _)| p.clone()), Some(PathBuf::from("~bob/key")));

        assert!(key_candidates(None, None).is_empty());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn files_that_are_not_keys_are_unreadable_not_a_crash() {
        assert_eq!(decode_key("", None).err(), Some(KeyProblem::Unreadable));
        assert_eq!(decode_key("hello", Some("secret")).err(), Some(KeyProblem::Unreadable));
        let broken = "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n";
        assert_eq!(decode_key(broken, None).err(), Some(KeyProblem::Unreadable));
        assert_eq!(load_key(Path::new("/nonexistent/home-cc/key"), None).err(), Some(KeyProblem::Unreadable));
    }

    #[test]
    fn locked_key_formats_are_recognised() {
        assert!(looks_locked("-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIB\n"));
        assert!(looks_locked(
            "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,00\n"
        ));
        assert!(looks_locked("PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: aes256-cbc\nComment: x\n"));
        assert!(!looks_locked("PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\nComment: x\n"));
        assert!(!looks_locked("-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEA\n"));
        // A locked key with no passphrase stored, and one with a passphrase that can't open it.
        let locked = "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIB\n-----END ENCRYPTED PRIVATE KEY-----\n";
        assert_eq!(decode_key(locked, None).err(), Some(KeyProblem::Locked));
        assert_eq!(decode_key(locked, Some("")).err(), Some(KeyProblem::Locked));
        assert_eq!(decode_key(locked, Some("guess")).err(), Some(KeyProblem::WrongPassphrase));
    }

    #[test]
    fn lists_read_naturally() {
        let items =
            |n: usize| -> Vec<String> { ["a", "b", "c"].iter().take(n).map(|s| s.to_string()).collect() };
        assert_eq!(join_list(&items(0)), "");
        assert_eq!(join_list(&items(1)), "a");
        assert_eq!(join_list(&items(2)), "a and b");
        assert_eq!(join_list(&items(3)), "a, b and c");
    }

    #[test]
    fn login_failure_says_what_was_tried_and_what_to_do() {
        let nothing = auth_message("nas", "jim", &[Tried::AgentEmpty], false, false);
        assert_eq!(
            nothing,
            "Could not log in to nas as jim. No SSH key was found on this PC and no password is stored for \
             this machine. Choose a key file in Settings or store a password for this machine."
        );

        let password = auth_message("nas", "jim", &[Tried::PasswordRefused], true, false);
        assert_eq!(
            password,
            "Could not log in to nas as jim. The stored password was not accepted. Check the user name and \
             the password."
        );

        let key = auth_message("nas", "jim", &[Tried::KeyRefused("id_ed25519".into())], false, false);
        assert_eq!(
            key,
            "Could not log in to nas as jim. The key file id_ed25519 was not accepted. Check the user name, \
             and that this PC's public key has been added to that machine, or store a password for it."
        );

        let all = [Tried::AgentKeys(2), Tried::KeyRefused("id_rsa".into()), Tried::PasswordRefused];
        let all = auth_message("nas", "jim", &all, true, false);
        assert!(
            all.contains(
                "2 keys from the SSH agent, the key file id_rsa and the stored password were not accepted."
            ),
            "{all}"
        );
        let one = auth_message("nas", "jim", &[Tried::AgentKeys(1)], true, false);
        assert!(one.contains("1 key from the SSH agent was not accepted."), "{one}");
        assert!(one.ends_with("has been added to that machine."), "{one}");
        let two = auth_message("nas", "jim", &[Tried::AgentKeys(2)], true, false);
        assert!(two.contains("2 keys from the SSH agent were not accepted."), "{two}");
    }

    #[test]
    fn login_failure_points_at_the_passphrase() {
        let locked = auth_message("nas", "jim", &[Tried::KeyLocked("id_ed25519".into())], false, false);
        assert_eq!(
            locked,
            "Could not log in to nas as jim. The key file id_ed25519 is locked with a passphrase: store the \
             passphrase in Settings."
        );
        let wrong = auth_message("nas", "jim", &[Tried::KeyWrongPassphrase("id_rsa".into())], false, false);
        assert!(wrong.contains("does not open the key file id_rsa"), "{wrong}");
        let missing = auth_message("nas", "jim", &[Tried::KeyUnreadable("/keys/mine".into())], false, false);
        assert!(missing.contains("/keys/mine could not be read"), "{missing}");
        assert!(!missing.contains("No SSH key was found"), "{missing}");
    }

    #[test]
    fn login_failure_explains_server_limits() {
        let keys_only = [Tried::KeyRefused("id_rsa".into()), Tried::PasswordNotTaken];
        let keys_only = auth_message("pc", "jim", &keys_only, true, true);
        assert!(keys_only.contains("does not take passwords, only keys"), "{keys_only}");
        assert!(
            keys_only.contains("administrators_authorized_keys"),
            "a Windows server gets the Windows hint"
        );
        let linux = auth_message("pc", "jim", &[Tried::KeyRefused("id_rsa".into())], false, false);
        assert!(!linux.contains("administrators_authorized_keys"), "{linux}");

        let cut = auth_message("pc", "jim", &[Tried::AgentKeys(5), Tried::CutOff], true, false);
        assert!(cut.contains("closed the connection before everything could be tried"), "{cut}");
        let silent = auth_message("pc", "jim", &[Tried::AgentSilent], false, false);
        assert!(silent.contains("waiting for you to unlock a key"), "{silent}");

        let password_only = auth_message("pc", "jim", &[Tried::KeysNotTaken], false, false);
        assert_eq!(
            password_only,
            "Could not log in to pc as jim. This machine takes a password only, not keys: store a password \
             for it."
        );
        let wrong = auth_message("pc", "jim", &[Tried::KeysNotTaken, Tried::PasswordRefused], true, false);
        assert_eq!(
            wrong,
            "Could not log in to pc as jim. The stored password was not accepted. Check the user name and \
             the password."
        );
    }

    #[test]
    fn login_failure_never_shows_secrets() {
        let mut login = login();
        login.password = Some("hunter2-password".into());
        login.key_passphrase = Some("hunter2-passphrase".into());
        let tried = [
            Tried::KeyWrongPassphrase("id_ed25519".into()),
            Tried::KeyRefused("id_rsa".into()),
            Tried::PasswordRefused,
        ];
        let message = auth_message(&login.host, &login.user, &tried, true, true);
        assert!(!message.contains("hunter2"), "{message}");
    }

    #[tokio::test]
    async fn missing_address_or_user_fails_without_connecting() {
        let mut no_host = login();
        no_host.host = "  ".into();
        assert!(matches!(connect(&no_host).await, Err(SshError::Unreachable(_))));
        let mut no_user = login();
        no_user.user = String::new();
        assert!(matches!(connect(&no_user).await, Err(SshError::AuthFailed(_))));
    }

    #[tokio::test]
    async fn closed_port_and_unknown_name_are_unreachable() {
        // Bind a port, then free it: nothing listens there now.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut closed = login();
        closed.port = port;
        match connect(&closed).await {
            Err(SshError::Unreachable(m)) => assert!(m.contains("refused the connection"), "{m}"),
            other => panic!("expected a refused connection, got {other:?}"),
        }

        // ".invalid" is reserved and never exists, and a part longer than 63 letters is
        // not a name at all, so this fails without asking anyone on the network.
        let mut unknown = login();
        unknown.host = format!("{}.invalid", "x".repeat(70));
        match connect(&unknown).await {
            Err(SshError::Unreachable(m)) => assert!(m.contains("was not found"), "{m}"),
            other => panic!("expected a name that is not found, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn something_that_is_not_ssh_is_unreachable() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
            }
        });
        let mut web = login();
        web.port = port;
        match connect(&web).await {
            Err(SshError::Unreachable(m)) => assert!(m.contains("closed it before the login"), "{m}"),
            other => panic!("expected a failed handshake, got {other:?}"),
        }
        server.join().unwrap();
    }

    /// Tauri commands run on several threads, so everything here has to be movable
    /// between them. Checked by the compiler; nothing runs.
    #[test]
    fn sessions_and_futures_can_cross_threads() {
        fn sendable<T: Send>(_: &T) {}
        fn shareable<T: Send + Sync>() {}
        shareable::<Session>();
        shareable::<SshError>();
        let login = login();
        sendable(&connect(&login));
        fn exec_is_sendable(session: &Session) {
            sendable(&session.exec("true", None, Duration::from_secs(1)));
            sendable(&session.open_channel());
        }
        let _ = exec_is_sendable;
    }
}
