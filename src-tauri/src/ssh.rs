//! SSH connections to the owner's machines. Step 3 uses it to run one command
//! and read the answer; terminals (step 4) will build on the same code.
//!
//! [`connect`] checks the server's identity first and only then logs in, with
//! whatever works: keys held by an SSH agent, key files, then the stored
//! password. A key file the owner set up in Settings goes before the agent.
//! The stored password is only ever sent to a machine whose identity is
//! already stored; on a first connection it is held back (see
//! [`SshError::NewHost`]). Everything is bounded by time limits, and text from
//! the server is treated as untrusted. Nothing here needs administrator rights.

use std::borrow::Cow;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
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
    /// None means a first connection: whatever the server presents is accepted for
    /// logging in with a key, but the stored password is not sent to it. An identity in
    /// here is what lets the stored password out, so [`connect`] says when one may be
    /// stored.
    pub known_key: Option<HostKey>,
    /// Private key file chosen in settings. None means try the usual files
    /// in the user's .ssh folder.
    pub key_path: Option<PathBuf>,
    pub key_passphrase: Option<String>,
    /// Password for this machine, if the owner stored one.
    pub password: Option<String>,
}

impl Login {
    /// True if a password is stored for this machine; an empty one does not count. On a
    /// first connection this decides whether the identity handed to `first_use` may be
    /// stored without asking the owner: see [`connect`].
    pub fn has_password(&self) -> bool {
        self.password.as_deref().is_some_and(|p| !p.is_empty())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    /// The server's identity is not the one stored. Never ignored silently, and nothing
    /// is offered to the server. `got` is what it shows now, for the owner to check.
    #[error(
        "This machine's identity has changed since home-cc first reached it (was {}, now {})",
        .expected.fingerprint,
        .got.fingerprint
    )]
    HostKeyChanged { expected: HostKey, got: HostKey },
    /// A first connection (no identity stored) that could only get in with the stored
    /// password, to a server that takes passwords. The password was not sent: the owner
    /// has to approve the identity in here first, and it then comes back in as
    /// [`Login::known_key`].
    #[error(
        "This is the first connection to this machine, so the stored password was not sent. Approve this \
         machine's identity ({}) first.",
        .0.fingerprint
    )]
    NewHost(HostKey),
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
/// A name can stand for several addresses (often an IPv6 one and an IPv4 one). A silent
/// address is given up on after this long, so the next one still gets its turn.
const TCP_EACH: Duration = Duration::from_secs(3);
/// How long the server gets to open one more channel on a connection that is already up.
const CHANNEL_LIMIT: Duration = Duration::from_secs(10);
/// How long an SSH agent gets for one answer. An agent can sit waiting for the owner to
/// unlock a key; the other ways of logging in should still get their turn.
const AGENT_LIMIT: Duration = Duration::from_secs(5);
/// Servers hang up after six failed tries by default. Keys stop at five so that a stored
/// password still gets its turn. Only keys the server turned down count, as on the server.
const MAX_KEY_TRIES: usize = 5;
/// Each of stdout and stderr is cut off here, so a runaway command can't fill the memory.
const MAX_OUTPUT: usize = 1024 * 1024;
/// Private keys are a few kilobytes. Anything much larger was picked by mistake.
const MAX_KEY_FILE: u64 = 256 * 1024;
/// The agent holds the keys the owner uses day to day, so it goes first where one normally
/// runs. On Windows the agent service is off unless an administrator turned it on, so the
/// key files go first there. See [`files_go_first`] for the exception.
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
/// offered.
///
/// If it is not set, this is a first connection. `first_use` is called once with the
/// identity the server showed, before anything is offered to it, so the caller has it
/// even if the login or what follows then fails. Key files and the agent are then tried
/// as usual: they give the server nothing it could use elsewhere. The stored password is
/// not sent. If nothing else gets in and the password is all that is left to try (one
/// is stored and the server takes passwords), this fails with [`SshError::NewHost`]; the
/// password gets its turn once the owner has approved the identity and it comes back in
/// `login.known_key`. If the password would have had no turn anyway (the server takes
/// keys only, or closed the connection first), approving would change nothing, so the
/// failure is [`SshError::AuthFailed`] as on any other connection.
///
/// What `first_use` may do with the identity depends on [`Login::has_password`], because
/// an identity in `login.known_key` counts as approved by the owner, wherever it came
/// from, and the stored password is sent to it:
/// - No password stored: store it there and then. Nothing that could be used elsewhere
///   is ever sent to this machine, and a later change of identity is noticed.
/// - A password stored: do NOT store it from `first_use`. It would come back as
///   `known_key` on the next connection and the password would go to a machine nobody
///   has looked at. Store it only once the owner has approved the identity shown with
///   [`SshError::NewHost`], or once this function has returned a [`Session`] (a key got
///   in without the password; the identity is in [`Session::host_key`]).
pub async fn connect(
    login: &Login,
    first_use: &(dyn Fn(&HostKey) + Send + Sync),
) -> Result<Session, SshError> {
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
    let handler = Handler { expected: login.known_key.clone(), seen: seen.clone() };
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
    // Still nothing has been offered to the server, not even the user name.
    if login.known_key.is_none() {
        first_use(&host_key);
    }

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
        // The stored password would have been sent, and was held back only because nobody
        // has approved this identity. It may well be what gets in, so ask for the
        // approval instead of calling the login failed. Where the password had no turn
        // anyway (the server takes keys only, or hung up first) approving would change
        // nothing, so that is a failed login like any other, with its advice.
        let error = if auth.tried.contains(&Tried::PasswordWithheld) {
            SshError::NewHost(host_key)
        } else {
            let windows_server = server_version.contains("Windows");
            SshError::AuthFailed(auth_message(host, user, &auth.tried, login.has_password(), windows_server))
        };
        say_goodbye(&handle).await;
        return Err(error);
    }
    Ok(Session { handle, host: host.to_string(), host_key, server_version })
}

impl Session {
    /// The identity the server showed. On a first connection it is the one that was
    /// handed to `first_use`.
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
        let deadline = deadline_after(limit);
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
        // A server can keep answering keepalives and still never answer this request, so
        // it needs a limit of its own.
        match timeout(CHANNEL_LIMIT, self.handle.channel_open_session()).await {
            Ok(Ok(channel)) => Ok(channel),
            Ok(Err(e)) => Err(self.lost(&e)),
            Err(_) => Err(SshError::Other(format!(
                "{} did not open a new session in time. Close other sessions to it, or try again.",
                self.host
            ))),
        }
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

/// The moment `limit` from now. A limit too long to count (a caller's way of saying
/// "no limit") becomes a moment far in the future instead of a crash.
fn deadline_after(limit: Duration) -> Instant {
    const FAR: Duration = Duration::from_secs(365 * 24 * 60 * 60);
    let now = Instant::now();
    now.checked_add(limit).or_else(|| now.checked_add(FAR)).unwrap_or(now)
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
    // The system would read "192.168.1" as 192.168.0.1 and connect to a machine nobody
    // typed, so text like that never reaches the name lookup.
    if incomplete_address(host) {
        return Err(SshError::Unreachable(format!(
            "{host} is not a full address. An address needs four numbers, like 192.168.1.20: correct it \
             for this machine."
        )));
    }
    let deadline = Instant::now() + TCP_LIMIT;
    let addrs: Vec<SocketAddr> = match timeout_at(deadline, tokio::net::lookup_host((host, port))).await {
        Ok(Ok(addrs)) => addrs.collect(),
        Ok(Err(_)) => return Err(not_found()),
        Err(_) => return Err(silent()),
    };
    let stream = match first_to_answer(&addrs, deadline, TCP_EACH, TcpStream::connect).await {
        Ok(stream) => stream,
        Err(Some(e)) => return Err(tcp_error(host, port, &e)),
        Err(None) => return Err(not_found()),
    };
    // Terminals send single key presses; don't let the system hold them back to batch them.
    let _ = stream.set_nodelay(true);
    Ok(stream)
}

/// True for text made only of digits and dots that is not a full IPv4 address, such as
/// "192.168.1" or "10.5". Systems accept these old short forms and fill in the gaps.
fn incomplete_address(host: &str) -> bool {
    !host.is_empty()
        && host.chars().all(|c| c.is_ascii_digit() || c == '.')
        && host.parse::<Ipv4Addr>().is_err()
}

/// Try each address in turn until one connects, all of it over by `deadline`. An address
/// that stays silent is given up on after `each`, so a dead first address doesn't use up
/// the time a working second one needs; the last address may use whatever time is left.
/// The error is the last one seen, or None if there was no address to try.
async fn first_to_answer<T, F, Fut>(
    addrs: &[SocketAddr],
    deadline: Instant,
    each: Duration,
    connect: F,
) -> Result<T, Option<std::io::Error>>
where
    F: Fn(SocketAddr) -> Fut,
    Fut: Future<Output = std::io::Result<T>>,
{
    let mut last = None;
    let mut addrs = addrs.iter().peekable();
    while let Some(addr) = addrs.next() {
        let now = Instant::now();
        if last.is_some() && now >= deadline {
            break;
        }
        let until = match (addrs.peek(), now.checked_add(each)) {
            (Some(_), Some(soon)) => soon.min(deadline),
            _ => deadline,
        };
        match timeout_at(until, connect(*addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => last = Some(e),
            Err(_) => last = Some(std::io::ErrorKind::TimedOut.into()),
        }
    }
    Err(last)
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
    HostKeyChanged { expected: HostKey, got: HostKey },
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
    /// The identity the server must show. None until the first one is seen.
    expected: Option<HostKey>,
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
            // The fingerprint is worked out from the kind of key as well as the key, so
            // comparing it is enough.
            if expected.fingerprint != key.fingerprint {
                return Err(Fail::HostKeyChanged { expected: expected.clone(), got: key });
            }
        }
        // Keys are exchanged again on a long connection: the identity must stay the same.
        self.expected = Some(key.clone());
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
    /// The server accepts a key the agent holds, but the agent would not sign with it.
    AgentDeclined,
    KeyRefused(String),
    /// The key file needs a passphrase and none is stored.
    KeyLocked(String),
    KeyWrongPassphrase(String),
    KeyUnreadable(String),
    /// The key file was left out because the server allows only a few tries.
    KeyNotTried(String),
    PasswordRefused,
    /// A password is stored and the server would have been sent it, but its identity is
    /// new, so it was held back. The login then ends as [`SshError::NewHost`].
    PasswordWithheld,
    /// A password is stored but the server takes keys only.
    PasswordNotTaken,
    /// The server takes a password only.
    KeysNotTaken,
    /// The server closed the connection before everything had its turn.
    CutOff,
}

type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;

/// A key file read before the login starts, with the name to call it by in messages.
struct KeyFile {
    name: String,
    key: Result<PrivateKey, KeyProblem>,
}

/// One key as the server tells them apart: its fingerprint, and whether it was offered
/// as a certificate. A certificate and the plain key under it are two separate offers.
type KeyId = (String, bool);

/// True if the key files are offered before the agent's keys. They are where no agent
/// normally runs, and also when the owner chose a key file or stored a passphrase in
/// Settings: that is the key they mean, and offering it first keeps a desktop agent from
/// showing its unlock window on every background check.
fn files_go_first(agent_first: bool, login: &Login) -> bool {
    let chosen = login.key_path.as_deref().is_some_and(|p| !p.as_os_str().is_empty());
    let passphrase = login.key_passphrase.as_deref().is_some_and(|p| !p.is_empty());
    !agent_first || chosen || passphrase
}

/// True if an agent key may be offered: the tries used so far, plus one kept back for
/// each key file still waiting, must leave room. Agents can hold many keys, and without
/// this they would use up every try before the key files had their turn.
fn agent_may_try(key_tries: usize, files_waiting: usize) -> bool {
    key_tries.saturating_add(files_waiting) < MAX_KEY_TRIES
}

/// One login in progress: tries each way in turn on the same connection.
struct Auth<'a> {
    handle: &'a mut client::Handle<Handler>,
    user: &'a str,
    tried: Vec<Tried>,
    /// Keys the server has turned down so far.
    key_tries: usize,
    /// What the server said it still accepts, from its latest refusal.
    methods: Option<MethodSet>,
    /// The server stopped answering login attempts.
    gone: bool,
    /// Signature kind for RSA keys, asked for once and only if an RSA key turns up
    /// (asking can cost a second on old servers).
    rsa_hash: Option<Option<HashAlg>>,
    /// Keys the server turned down. A key that is both in the agent and on disk is not
    /// offered again, because every refusal counts towards the server's limit. A key the
    /// agent would not sign with is not in here: the server never turned it down, and
    /// the same key read from its file may well work.
    refused_keys: Vec<KeyId>,
    /// Key files that have not had their turn yet.
    files: Vec<KeyFile>,
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
            refused_keys: Vec::new(),
            files: Vec::new(),
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
        if self.may_try_key() {
            // Read ahead of the agent's turn, so it is known how many tries to keep for them.
            self.files = load_key_files(login).await;
        }
        let with_keys = if files_go_first(AGENT_FIRST, login) {
            self.key_files().await || self.agents().await
        } else {
            self.agents().await || self.key_files().await
        };
        with_keys || self.password(login).await
    }

    fn allows(&self, method: MethodKind) -> bool {
        !self.gone && self.methods.as_ref().is_none_or(|m| m.contains(&method))
    }

    fn may_try_key(&self) -> bool {
        self.key_tries < MAX_KEY_TRIES && self.allows(MethodKind::PublicKey)
    }

    /// Like [`Self::may_try_key`], but keeps a try back for each key file still waiting.
    fn may_try_agent_key(&self) -> bool {
        let waiting = self.files.iter().filter(|file| match &file.key {
            Ok(key) => !self.refused_keys.contains(&(fingerprint(key.public_key()), false)),
            Err(_) => false,
        });
        self.allows(MethodKind::PublicKey) && agent_may_try(self.key_tries, waiting.count())
    }

    fn key_refused(&mut self, key: KeyId) {
        self.key_tries += 1;
        self.refused_keys.push(key);
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
        let mut declined = false;
        for identity in identities {
            if !self.may_try_agent_key() {
                break;
            }
            let certificate = matches!(identity, AgentIdentity::Certificate { .. });
            let id = (fingerprint(&identity.public_key()), certificate);
            if self.refused_keys.contains(&id) {
                continue;
            }
            let hash = if identity.public_key().algorithm().is_rsa() { self.rsa_hash().await } else { None };
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
                    self.key_refused(id);
                }
                // The agent would not sign with this key (it is locked, or the owner said
                // no). The server asks for the signature only for a key it accepts and
                // does not count this as a failed try, so the connection is still good
                // and the same key on disk keeps its turn.
                Err(AgentAuthError::Key(_)) if !silent => declined = true,
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
        if declined {
            self.tried.push(Tried::AgentDeclined);
        }
        false
    }

    async fn key_files(&mut self) -> bool {
        for KeyFile { name, key } in std::mem::take(&mut self.files) {
            if !self.allows(MethodKind::PublicKey) {
                break;
            }
            let key = match key {
                Ok(key) => key,
                Err(KeyProblem::Locked) => {
                    self.tried.push(Tried::KeyLocked(name));
                    continue;
                }
                Err(KeyProblem::WrongPassphrase) => {
                    self.tried.push(Tried::KeyWrongPassphrase(name));
                    continue;
                }
                Err(KeyProblem::Unreadable) => {
                    self.tried.push(Tried::KeyUnreadable(name));
                    continue;
                }
            };
            let id = (fingerprint(key.public_key()), false);
            if self.refused_keys.contains(&id) {
                continue;
            }
            if !self.may_try_key() {
                self.tried.push(Tried::KeyNotTried(name));
                continue;
            }
            let hash = if key.algorithm().is_rsa() { self.rsa_hash().await } else { None };
            let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
            match self.handle.authenticate_publickey(self.user, key).await {
                Ok(result) => {
                    if self.accepted(result) {
                        return true;
                    }
                    self.key_refused(id);
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
        if self.gone {
            self.tried.push(Tried::CutOff);
            return false;
        }
        let stored = password_to_send(login);
        if stored == Password::NotStored {
            return false;
        }
        // Whether the server would be sent the password at all comes first, so that
        // "held back" is only ever said of a server that takes passwords.
        let plain = self.allows(MethodKind::Password);
        if !plain && !self.allows(MethodKind::KeyboardInteractive) {
            self.tried.push(Tried::PasswordNotTaken);
            return false;
        }
        let Password::Send(password) = stored else {
            self.tried.push(Tried::PasswordWithheld);
            return false;
        };
        let logged_in = if plain {
            match self.handle.authenticate_password(self.user, password).await {
                Ok(result) => self.accepted(result),
                Err(_) => {
                    self.tried.push(Tried::CutOff);
                    return false;
                }
            }
        } else {
            self.typed_password(password).await
        };
        if !logged_in {
            self.tried.push(Tried::PasswordRefused);
        }
        logged_in
    }

    /// Some servers take the password only as the typed answer to a question (their
    /// "keyboard-interactive" way). Answer the hidden-input question with it. Only ever
    /// given what [`password_to_send`] let through.
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

/// Whether there is a password for this login and whether it may leave this PC.
#[derive(Debug, PartialEq, Eq)]
enum Password<'a> {
    Send(&'a str),
    /// One is stored, but the server's identity has not been seen and approved before.
    /// Whoever receives a password can use it again, so it stays here.
    Withheld,
    NotStored,
}

/// The one place that decides whether the stored password is sent, for the plain
/// password login and for the typed answer alike. With an identity in `known_key` the
/// server has already shown exactly that one by the time anything is sent, or the
/// connection would have ended with [`SshError::HostKeyChanged`].
fn password_to_send(login: &Login) -> Password<'_> {
    match login.password.as_deref().filter(|p| !p.is_empty()) {
        None => Password::NotStored,
        Some(_) if login.known_key.is_none() => Password::Withheld,
        Some(password) => Password::Send(password),
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

fn fingerprint(key: &russh::keys::PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Read every key file that could be offered. Unlocking a key with a passphrase is slow
/// on purpose, so this is kept off the threads that serve the screens.
async fn load_key_files(login: &Login) -> Vec<KeyFile> {
    let mut files = Vec::new();
    for (path, name) in key_candidates(login.key_path.as_deref(), home_dir().as_deref()) {
        let passphrase = login.key_passphrase.clone();
        let loaded = tokio::task::spawn_blocking(move || load_key(&path, passphrase.as_deref())).await;
        files.push(KeyFile { name, key: loaded.unwrap_or(Err(KeyProblem::Unreadable)) });
    }
    files
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
    use std::io::Read;
    // Only an ordinary file with something in it can be a key. A pipe or a device (which
    // say they are empty) could be read for ever, and that would hold a thread for good.
    let ordinary = |m: std::fs::Metadata| m.is_file() && (1..=MAX_KEY_FILE).contains(&m.len());
    if !std::fs::metadata(path).is_ok_and(ordinary) {
        return Err(KeyProblem::Unreadable);
    }
    let file = std::fs::File::open(path).map_err(|_| KeyProblem::Unreadable)?;
    // The size was only what the file claimed a moment ago; never read past the limit.
    let mut text = String::new();
    let read = file.take(MAX_KEY_FILE + 1).read_to_string(&mut text).map_err(|_| KeyProblem::Unreadable)?;
    if read as u64 > MAX_KEY_FILE {
        return Err(KeyProblem::Unreadable);
    }
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
            Tried::KeyNotTried(name) => notes.push(format!(
                "The key file {name} was not tried, because that machine allows only a few attempts: choose \
                 it as the key file in Settings."
            )),
            // Not shown as things stand, because `connect` answers with `NewHost` instead.
            // Kept true, so the sentence is right if that ever changes.
            Tried::PasswordWithheld => notes.push(
                "The stored password was not sent, because this is the first connection to this machine."
                    .into(),
            ),
            Tried::PasswordNotTaken => notes.push("This machine does not take passwords, only keys.".into()),
            Tried::KeysNotTaken if !password_stored => {
                notes.push("This machine takes a password only, not keys: store a password for it.".into())
            }
            Tried::KeysNotTaken => {}
            Tried::AgentSilent => notes.push(
                "The SSH agent did not answer in time; it may be waiting for you to unlock a key.".into(),
            ),
            Tried::AgentDeclined => notes.push(
                "The SSH agent holds a key that machine accepts but would not use it; the key may be locked. \
                 Unlock it, or choose its key file in Settings."
                    .into(),
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

/// A small SSH server that runs inside the test process, so the rules about what is
/// offered to a machine, and when, are checked against what a server really received.
/// It listens on 127.0.0.1 only and writes down every login attempt it is sent.
#[cfg(test)]
pub(crate) mod test_server {
    use super::*;
    use russh::keys::ssh_key::LineEnding;
    use russh::keys::ssh_key::private::Ed25519Keypair;
    use russh::server::{self, Auth, PublicKeyOffer, Response, VerifiedPublicKey};

    /// The only user the server lets in.
    pub(crate) const USER: &str = "jim";
    /// The only password it accepts, sent plainly or typed as an answer.
    pub(crate) const PASSWORD: &str = "correct horse battery";

    /// A key made from fixed bytes, so no key text has to live in the source.
    fn key(seed: u8) -> PrivateKey {
        PrivateKey::from(Ed25519Keypair::from_seed(&[seed; 32]))
    }

    /// The private key the server accepts for [`USER`], as the text of a key file.
    pub(crate) fn client_key_file() -> String {
        key(2).to_openssh(LineEnding::LF).unwrap().to_string()
    }

    pub(crate) struct TestServer {
        pub(crate) port: u16,
        /// The identity it shows, as [`connect`] reports it.
        pub(crate) host_key: HostKey,
        attempts: Arc<Mutex<Vec<String>>>,
    }

    impl TestServer {
        /// Every login attempt received so far, oldest first: "none" (the question
        /// about what is accepted), "key offered", "key signed", "password <text>",
        /// "question asked" and "answer <text>".
        pub(crate) fn attempts(&self) -> Vec<String> {
            self.attempts.lock().unwrap().clone()
        }

        /// A login for this server: [`USER`], no stored identity, no password and a
        /// key file that does not exist, so that no real key on this PC is ever read.
        pub(crate) fn login(&self) -> Login {
            Login {
                host: "127.0.0.1".into(),
                port: self.port,
                user: USER.into(),
                known_key: None,
                key_path: Some("/nonexistent/home-cc/test-key".into()),
                key_passphrase: None,
                password: None,
            }
        }
    }

    /// Start a server that offers these ways of logging in. Keep `PublicKey` out of
    /// `methods` unless the login's key file is the accepted one: a refused key is
    /// followed by the keys of whatever SSH agent the person running the tests has.
    pub(crate) async fn start(methods: &[MethodKind]) -> TestServer {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server_key = key(1);
        let host_key = HostKey {
            algorithm: server_key.algorithm().to_string(),
            fingerprint: fingerprint(server_key.public_key()),
        };
        let methods = MethodSet::from(methods);
        let config = Arc::new(server::Config {
            keys: vec![server_key],
            methods: methods.clone(),
            // The real thing answers a refusal slowly on purpose; tests need not wait.
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        });
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let seen = attempts.clone();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let handler = ServerHandler {
                    attempts: seen.clone(),
                    methods: methods.clone(),
                    accepted_key: fingerprint(key(2).public_key()),
                };
                let config = config.clone();
                tokio::spawn(async move {
                    if let Ok(session) = server::run_stream(config, socket, handler).await {
                        let _ = session.await;
                    }
                });
            }
        });
        TestServer { port, host_key, attempts }
    }

    struct ServerHandler {
        attempts: Arc<Mutex<Vec<String>>>,
        methods: MethodSet,
        accepted_key: String,
    }

    impl ServerHandler {
        fn note(&self, attempt: String) {
            self.attempts.lock().unwrap().push(attempt);
        }

        fn answer(&self, accept: bool) -> Auth {
            if accept {
                Auth::Accept
            } else {
                Auth::Reject { proceed_with_methods: Some(self.methods.clone()), partial_success: false }
            }
        }
    }

    impl server::Handler for ServerHandler {
        type Error = russh::Error;

        async fn auth_none(&mut self, _user: &str) -> Result<Auth, russh::Error> {
            self.note("none".into());
            Ok(self.answer(false))
        }

        async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, russh::Error> {
            self.note(format!("password {password}"));
            Ok(self.answer(user == USER && password == PASSWORD))
        }

        async fn auth_publickey_offered(
            &mut self,
            user: &str,
            offer: PublicKeyOffer<'_>,
        ) -> Result<Auth, russh::Error> {
            self.note("key offered".into());
            Ok(self.answer(user == USER && fingerprint(offer.public_key) == self.accepted_key))
        }

        async fn auth_publickey(
            &mut self,
            user: &str,
            key: VerifiedPublicKey<'_>,
        ) -> Result<Auth, russh::Error> {
            self.note("key signed".into());
            Ok(self.answer(user == USER && fingerprint(key.public_key) == self.accepted_key))
        }

        async fn auth_keyboard_interactive<'a>(
            &'a mut self,
            user: &str,
            _submethods: &str,
            response: Option<Response<'a>>,
        ) -> Result<Auth, russh::Error> {
            let Some(mut answers) = response else {
                self.note("question asked".into());
                return Ok(Auth::Partial {
                    name: Cow::Borrowed(""),
                    instructions: Cow::Borrowed(""),
                    prompts: Cow::Owned(vec![(Cow::Borrowed("Password: "), false)]),
                });
            };
            let answer = answers.next().map(|a| String::from_utf8_lossy(&a).into_owned()).unwrap_or_default();
            self.note(format!("answer {answer}"));
            Ok(self.answer(user == USER && answer == PASSWORD))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::{self, PASSWORD};
    use super::*;

    /// For connections that must not get as far as a first connection.
    fn never(key: &HostKey) {
        panic!("first_use was called with {key:?}");
    }

    /// Writes down each identity handed to `first_use`, with how many login attempts
    /// the server had received by then.
    struct FirstUse<'a> {
        server: &'a test_server::TestServer,
        calls: Mutex<Vec<(HostKey, usize)>>,
    }

    impl<'a> FirstUse<'a> {
        fn new(server: &'a test_server::TestServer) -> Self {
            Self { server, calls: Mutex::new(Vec::new()) }
        }

        fn call(&self, key: &HostKey) {
            self.calls.lock().unwrap().push((key.clone(), self.server.attempts().len()));
        }

        /// Called exactly once, with the server's identity, before any login attempt.
        fn assert_called_once_before_the_login(&self) {
            let calls = self.calls.lock().unwrap();
            assert_eq!(*calls, [(self.server.host_key.clone(), 0)]);
        }
    }

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
    fn only_ordinary_files_of_a_sane_size_are_read_as_keys() {
        let dir = std::env::temp_dir().join(format!("home-cc-ssh-keys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load_key(&dir, None).err(), Some(KeyProblem::Unreadable), "a folder");

        let empty = dir.join("empty");
        std::fs::write(&empty, "").unwrap();
        assert_eq!(load_key(&empty, None).err(), Some(KeyProblem::Unreadable));

        // Marked as locked, so it would be reported as locked if it were read at all.
        let mut text = String::from("-----BEGIN ENCRYPTED PRIVATE KEY-----\n");
        let just_fits = dir.join("fits");
        text.push_str(&"A".repeat(MAX_KEY_FILE as usize - text.len()));
        std::fs::write(&just_fits, &text).unwrap();
        assert_eq!(load_key(&just_fits, None).err(), Some(KeyProblem::Locked));
        let too_big = dir.join("big");
        text.push('A');
        std::fs::write(&too_big, &text).unwrap();
        assert_eq!(load_key(&too_big, None).err(), Some(KeyProblem::Unreadable));

        // Devices never end; reading one must be refused, not attempted.
        #[cfg(unix)]
        for device in ["/dev/zero", "/dev/null"] {
            assert_eq!(load_key(Path::new(device), None).err(), Some(KeyProblem::Unreadable), "{device}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn key_set_up_in_settings_goes_before_the_agent() {
        let plain = login();
        assert!(!files_go_first(true, &plain), "the agent goes first where one normally runs");
        assert!(files_go_first(false, &plain), "and never first on Windows");

        let mut chosen = login();
        chosen.key_path = Some("/keys/homelab".into());
        assert!(files_go_first(true, &chosen));
        let mut unlocked = login();
        unlocked.key_passphrase = Some("secret".into());
        assert!(files_go_first(true, &unlocked));

        // Empty settings mean "not set".
        let mut blank = login();
        blank.key_path = Some(PathBuf::new());
        blank.key_passphrase = Some(String::new());
        assert!(!files_go_first(true, &blank));
    }

    #[test]
    fn agent_keys_leave_a_try_for_each_waiting_key_file() {
        assert!(agent_may_try(0, 0));
        assert!(agent_may_try(MAX_KEY_TRIES - 1, 0));
        assert!(!agent_may_try(MAX_KEY_TRIES, 0), "the password still needs its turn");
        // Three key files waiting: the agent gets two tries, however many keys it holds.
        assert!(agent_may_try(1, 3));
        assert!(!agent_may_try(2, 3));
        assert!(!agent_may_try(0, MAX_KEY_TRIES));
        assert!(!agent_may_try(usize::MAX, usize::MAX));
    }

    #[tokio::test]
    async fn silent_address_does_not_use_up_the_time_of_the_next() {
        use std::io::{Error, ErrorKind};
        let silent: SocketAddr = "[2001:db8::1]:22".parse().unwrap();
        let refusing: SocketAddr = "192.0.2.1:22".parse().unwrap();
        let working: SocketAddr = "192.0.2.2:22".parse().unwrap();
        let connect = move |addr: SocketAddr| async move {
            if addr == silent {
                std::future::pending::<()>().await;
            }
            if addr == refusing { Err(Error::from(ErrorKind::ConnectionRefused)) } else { Ok(addr) }
        };
        let each = Duration::from_millis(40);
        let soon = || Instant::now() + Duration::from_secs(30);

        let started = Instant::now();
        let reached = first_to_answer(&[silent, refusing, working], soon(), each, connect).await;
        assert_eq!(reached.ok(), Some(working));
        assert!(started.elapsed() < Duration::from_secs(10), "the silent address got {each:?}, not the lot");

        // The last error is the one reported.
        let kind = |r: Result<SocketAddr, Option<Error>>| r.err().flatten().map(|e| e.kind());
        let refused = first_to_answer(&[silent, refusing], soon(), each, connect).await;
        assert_eq!(kind(refused), Some(ErrorKind::ConnectionRefused));
        let shortly = Instant::now() + each * 3;
        let quiet = first_to_answer(&[refusing, silent, silent], shortly, each, connect).await;
        assert_eq!(kind(quiet), Some(ErrorKind::TimedOut));
        let nothing = first_to_answer(&[], soon(), each, connect).await;
        assert!(matches!(nothing, Err(None)));

        // A lone address may use all the time there is, as may the last of several.
        let started = Instant::now();
        let limit = Duration::from_millis(300);
        let alone = first_to_answer(&[silent], started + limit, Duration::from_millis(10), connect).await;
        assert_eq!(kind(alone), Some(ErrorKind::TimedOut));
        assert!(started.elapsed() >= limit, "gave up after {:?}", started.elapsed());
    }

    #[test]
    fn endless_time_limit_is_not_a_crash() {
        let now = Instant::now();
        assert!(deadline_after(Duration::MAX) > now + Duration::from_secs(24 * 60 * 60));
        let short = deadline_after(Duration::from_secs(5));
        assert!(short >= now + Duration::from_secs(5) && short < now + Duration::from_secs(60));
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
        let left = [Tried::AgentKeys(5), Tried::KeyNotTried("id_rsa".into())];
        let left = auth_message("nas", "jim", &left, false, false);
        assert!(left.contains("The key file id_rsa was not tried"), "{left}");
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
        let declined = auth_message("pc", "jim", &[Tried::AgentDeclined], false, false);
        assert_eq!(
            declined,
            "Could not log in to pc as jim. The SSH agent holds a key that machine accepts but would not use \
             it; the key may be locked. Unlock it, or choose its key file in Settings."
        );

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
        assert!(matches!(connect(&no_host, &never).await, Err(SshError::Unreachable(_))));
        let mut no_user = login();
        no_user.user = String::new();
        assert!(matches!(connect(&no_user, &never).await, Err(SshError::AuthFailed(_))));
    }

    #[tokio::test]
    async fn closed_port_and_unknown_name_are_unreachable() {
        // Bind a port, then free it: nothing listens there now.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut closed = login();
        closed.port = port;
        match connect(&closed, &never).await {
            Err(SshError::Unreachable(m)) => assert!(m.contains("refused the connection"), "{m}"),
            other => panic!("expected a refused connection, got {other:?}"),
        }

        // ".invalid" is reserved and never exists, and a part longer than 63 letters is
        // not a name at all, so this fails without asking anyone on the network.
        let mut unknown = login();
        unknown.host = format!("{}.invalid", "x".repeat(70));
        match connect(&unknown, &never).await {
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
        match connect(&web, &never).await {
            Err(SshError::Unreachable(m)) => assert!(m.contains("closed it before the login"), "{m}"),
            other => panic!("expected a failed handshake, got {other:?}"),
        }
        server.join().unwrap();
    }

    #[tokio::test]
    async fn half_typed_address_is_refused_without_connecting() {
        for text in ["192.168.1", "10.5", "22", "1.2.3.4.5", "256.1.1.1", "192.168.1.", "..", "192.168.010.5"]
        {
            assert!(incomplete_address(text), "{text}");
        }
        for text in ["192.168.1.20", "10.0.0.5", "nas", "nas.lan", "pc-2", "3com.lan", "2001:db8::1", ""] {
            assert!(!incomplete_address(text), "{text}");
        }
        // The system reads "127.1" as 127.0.0.1, this PC itself. Nothing may be sent there.
        let mut short = login();
        short.host = "127.1".into();
        match connect(&short, &never).await {
            Err(SshError::Unreachable(m)) => assert!(m.starts_with("127.1 is not a full address"), "{m}"),
            other => panic!("expected the address to be refused, got {other:?}"),
        }
    }

    #[test]
    fn stored_password_is_only_for_a_machine_whose_identity_is_stored() {
        let mut login = login();
        assert_eq!(password_to_send(&login), Password::NotStored);
        login.password = Some(String::new());
        assert_eq!(password_to_send(&login), Password::NotStored, "an empty password is no password");
        login.password = Some("secret".into());
        assert_eq!(password_to_send(&login), Password::Withheld, "first connection");

        login.known_key = Some(HostKey { algorithm: "ssh-ed25519".into(), fingerprint: "SHA256:abc".into() });
        assert_eq!(password_to_send(&login), Password::Send("secret"));
        login.password = Some(String::new());
        assert_eq!(password_to_send(&login), Password::NotStored);
    }

    #[test]
    fn an_empty_password_is_not_a_stored_password() {
        let mut login = login();
        assert!(!login.has_password());
        login.password = Some(String::new());
        assert!(!login.has_password());
        login.password = Some("secret".into());
        assert!(login.has_password());
    }

    #[test]
    fn identity_problems_read_as_plain_words() {
        let old = HostKey { algorithm: "ssh-ed25519".into(), fingerprint: "SHA256:old".into() };
        let new = HostKey { algorithm: "ssh-rsa".into(), fingerprint: "SHA256:new".into() };
        assert_eq!(
            SshError::HostKeyChanged { expected: old, got: new.clone() }.to_string(),
            "This machine's identity has changed since home-cc first reached it (was SHA256:old, now \
             SHA256:new)"
        );
        assert_eq!(
            SshError::NewHost(new).to_string(),
            "This is the first connection to this machine, so the stored password was not sent. Approve this \
             machine's identity (SHA256:new) first."
        );
    }

    #[test]
    fn login_failure_never_says_a_withheld_password_was_tried() {
        let tried = [Tried::KeyRefused("id_ed25519".into()), Tried::PasswordWithheld];
        let after_key = auth_message("nas", "jim", &tried, true, false);
        assert_eq!(
            after_key,
            "Could not log in to nas as jim. The key file id_ed25519 was not accepted. The stored password \
             was not sent, because this is the first connection to this machine. Check the user name, and \
             that this PC's public key has been added to that machine."
        );
        let alone = auth_message("nas", "jim", &[Tried::KeysNotTaken, Tried::PasswordWithheld], true, false);
        assert_eq!(
            alone,
            "Could not log in to nas as jim. The stored password was not sent, because this is the first \
             connection to this machine."
        );
        for message in [after_key, alone] {
            assert!(!message.contains("password was not accepted"), "{message}");
            assert!(!message.contains("and the password"), "{message}");
            assert!(!message.contains("no password is stored"), "{message}");
        }
    }

    #[tokio::test]
    async fn first_connection_hands_over_the_identity_before_the_login() {
        let server = test_server::start(&[MethodKind::PublicKey, MethodKind::Password]).await;
        let key_file = std::env::temp_dir().join(format!("home-cc-ssh-first-use-{}", std::process::id()));
        std::fs::write(&key_file, test_server::client_key_file()).unwrap();
        let mut login = server.login();
        login.key_path = Some(key_file.clone());
        // A password is stored as well. The key gets in, so it is neither sent nor asked about.
        login.password = Some(PASSWORD.into());

        let first_use = FirstUse::new(&server);
        let session = connect(&login, &|key: &HostKey| first_use.call(key)).await;
        std::fs::remove_file(&key_file).unwrap();
        let session = session.expect("the key is accepted");
        first_use.assert_called_once_before_the_login();
        assert_eq!(*session.host_key(), server.host_key);
        let attempts = server.attempts();
        assert_eq!(attempts.first().map(String::as_str), Some("none"), "{attempts:?}");
        assert_eq!(attempts.last().map(String::as_str), Some("key signed"), "{attempts:?}");
        assert!(attempts.iter().all(|a| a == "none" || a.starts_with("key ")), "{attempts:?}");
        session.close().await;
    }

    #[tokio::test]
    async fn first_connection_never_sends_the_stored_password() {
        // Neither plainly nor typed as the answer to the server's question.
        for method in [MethodKind::Password, MethodKind::KeyboardInteractive] {
            let server = test_server::start(&[method]).await;
            let mut login = server.login();
            login.password = Some(PASSWORD.into());
            let first_use = FirstUse::new(&server);
            match connect(&login, &|key: &HostKey| first_use.call(key)).await {
                Err(SshError::NewHost(key)) => assert_eq!(key, server.host_key),
                other => panic!("expected to be asked to approve the identity, got {other:?}"),
            }
            first_use.assert_called_once_before_the_login();
            // Only the question about what the server accepts ever reached it.
            assert_eq!(server.attempts(), ["none"], "{method:?}");
        }
    }

    #[tokio::test]
    async fn stored_identity_lets_the_password_through() {
        let ways = [
            (MethodKind::Password, format!("password {PASSWORD}")),
            (MethodKind::KeyboardInteractive, format!("answer {PASSWORD}")),
        ];
        for (method, sent) in ways {
            let server = test_server::start(&[method]).await;
            let mut login = server.login();
            login.password = Some(PASSWORD.into());
            login.known_key = Some(server.host_key.clone());
            let session = connect(&login, &never).await.expect("the password is accepted");
            assert_eq!(server.attempts().last(), Some(&sent));
            session.close().await;
        }

        // A wrong one is still a plain login failure.
        let server = test_server::start(&[MethodKind::Password]).await;
        let mut login = server.login();
        login.password = Some("not the password".into());
        login.known_key = Some(server.host_key.clone());
        match connect(&login, &never).await {
            Err(SshError::AuthFailed(m)) => assert_eq!(
                m,
                "Could not log in to 127.0.0.1 as jim. The stored password was not accepted. Check the user \
                 name and the password."
            ),
            other => panic!("expected a failed login, got {other:?}"),
        }
        assert_eq!(server.attempts(), ["none", "password not the password"]);
    }

    /// Approving the identity is asked for only where it would let the password
    /// through. A machine that would not be sent the password anyway gets the ordinary
    /// login failure, with its advice, on a first connection as on any other.
    #[tokio::test]
    async fn first_connection_asks_for_approval_only_if_the_password_would_be_sent() {
        // Takes no passwords in either form ("hostbased" stands in for keys here: with
        // keys on offer the SSH agent of whoever runs the tests would be asked).
        let keys_only = test_server::start(&[MethodKind::HostBased]).await;
        // Offers nothing to carry on with, as when a machine ends the login early.
        let hangs_up = test_server::start(&[]).await;
        let cases = [
            (&keys_only, "This machine does not take passwords, only keys."),
            (&hangs_up, "The machine closed the connection before everything could be tried."),
        ];
        for (server, advice) in cases {
            let mut login = server.login();
            login.password = Some(PASSWORD.into());
            let first_use = FirstUse::new(server);
            match connect(&login, &|key: &HostKey| first_use.call(key)).await {
                Err(SshError::AuthFailed(m)) => {
                    assert_eq!(m, format!("Could not log in to 127.0.0.1 as jim. {advice}"));
                }
                other => panic!("expected a failed login, got {other:?}"),
            }
            first_use.assert_called_once_before_the_login();
            assert_eq!(server.attempts(), ["none"]);

            // With the identity stored the outcome is the same: approving changes nothing.
            login.known_key = Some(server.host_key.clone());
            match connect(&login, &never).await {
                Err(SshError::AuthFailed(m)) => assert!(m.ends_with(advice), "{m}"),
                other => panic!("expected a failed login, got {other:?}"),
            }
            assert_eq!(server.attempts(), ["none", "none"]);
        }
    }

    /// The rule for callers in the notes on [`connect`], followed over several
    /// connections the way a background reading makes them: with a password stored the
    /// identity is not stored from `first_use`, so the password stays here until the
    /// owner approves, however many times the machine is tried.
    #[tokio::test]
    async fn storing_the_identity_by_the_rule_keeps_the_password_back_until_approved() {
        let server = test_server::start(&[MethodKind::Password]).await;
        let stored: Mutex<Option<HostKey>> = Mutex::new(None);
        let mut login = server.login();
        login.password = Some(PASSWORD.into());
        for _ in 0..3 {
            login.known_key = stored.lock().unwrap().clone();
            let first_use = |key: &HostKey| {
                if !login.has_password() {
                    *stored.lock().unwrap() = Some(key.clone());
                }
            };
            match connect(&login, &first_use).await {
                Err(SshError::NewHost(key)) => assert_eq!(key, server.host_key),
                other => panic!("expected to be asked to approve the identity, got {other:?}"),
            }
        }
        assert_eq!(*stored.lock().unwrap(), None);
        assert_eq!(server.attempts(), ["none", "none", "none"]);

        // The owner approves what was shown: only now does the password leave this PC.
        login.known_key = Some(server.host_key.clone());
        let session = connect(&login, &never).await.expect("the password is accepted");
        assert_eq!(server.attempts().last(), Some(&format!("password {PASSWORD}")));
        session.close().await;

        // With no password stored the same rule stores the identity on the spot, even
        // though the login then fails, and the next connection is checked against it.
        let server = test_server::start(&[MethodKind::Password]).await;
        let mut login = server.login();
        let first_use = |key: &HostKey| {
            if !login.has_password() {
                *stored.lock().unwrap() = Some(key.clone());
            }
        };
        assert!(matches!(connect(&login, &first_use).await, Err(SshError::AuthFailed(_))));
        assert_eq!(*stored.lock().unwrap(), Some(server.host_key.clone()));
        login.known_key = stored.lock().unwrap().clone();
        assert!(matches!(connect(&login, &never).await, Err(SshError::AuthFailed(_))));
    }

    #[tokio::test]
    async fn changed_identity_stops_before_anything_is_offered() {
        // Stored as the same kind of key, and as a kind the machine no longer shows.
        for algorithm in ["ssh-ed25519", "ssh-rsa"] {
            let server = test_server::start(&[
                MethodKind::PublicKey,
                MethodKind::Password,
                MethodKind::KeyboardInteractive,
            ])
            .await;
            let stored =
                HostKey { algorithm: algorithm.into(), fingerprint: "SHA256:what-was-stored".into() };
            let mut login = server.login();
            login.password = Some(PASSWORD.into());
            login.known_key = Some(stored.clone());
            match connect(&login, &never).await {
                Err(SshError::HostKeyChanged { expected, got }) => {
                    assert_eq!(expected, stored);
                    assert_eq!(got, server.host_key, "what it shows now, with its kind of key");
                }
                other => panic!("expected a changed identity, got {other:?}"),
            }
            // Not even the question about what the server accepts, which names the user.
            assert!(server.attempts().is_empty(), "{:?}", server.attempts());
        }
    }

    #[tokio::test]
    async fn first_connection_with_no_password_stored_is_a_plain_login_failure() {
        for password in [None, Some(String::new())] {
            let server = test_server::start(&[MethodKind::Password]).await;
            let mut login = server.login();
            login.password = password;
            let first_use = FirstUse::new(&server);
            match connect(&login, &|key: &HostKey| first_use.call(key)).await {
                Err(SshError::AuthFailed(m)) => assert_eq!(
                    m,
                    "Could not log in to 127.0.0.1 as jim. This machine takes a password only, not keys: \
                     store a password for it."
                ),
                other => panic!("expected a failed login, got {other:?}"),
            }
            first_use.assert_called_once_before_the_login();
            assert_eq!(server.attempts(), ["none"]);
        }
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
        sendable(&connect(&login, &never));
        fn exec_is_sendable(session: &Session) {
            sendable(&session.exec("true", None, Duration::from_secs(1)));
            sendable(&session.open_channel());
        }
        let _ = exec_is_sendable;
    }
}
