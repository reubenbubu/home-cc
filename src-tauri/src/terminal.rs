//! Terminal tabs: a full terminal on a machine, over SSH.
//!
//! Each tab is its own SSH connection, made with the same login rules as
//! everything else (see ssh.rs and `monitor::Prepared::connect`: the stored password
//! only ever goes to an identity the owner approved). On it the app asks for a
//! terminal (a PTY, "xterm-256color", at the size of the tab), then starts the
//! login shell, or a given command such as `docker exec -it web sh`.
//!
//! What the machine prints is sent to the screens in the "terminal-output"
//! event, in small batches, as base64: the bytes go through untouched, so a
//! character split across two packets is put back together by the terminal on
//! screen. The screens confirm what they have drawn (`terminal_ack`); while
//! too much is waiting to be drawn, the app stops reading, so a program that
//! prints without end (`cat` of a huge file) slows down instead of filling the
//! memory, and Ctrl+C still gets through quickly.
//!
//! When the tab is closed, the program ends or the connection drops, the
//! connection is closed and "terminal-exit" says why. All terminals are
//! closed when the app exits.

use crate::error::{Error, Result};
use crate::inventory::Inventory;
use crate::monitor::{self, Monitor};
use crate::ssh::{HostKey, Session, SshError};
use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Pty, Sig};
use serde::Serialize;
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Notify, mpsc};
use tokio::time::{Instant, sleep_until, timeout};

pub const EVENT_OUTPUT: &str = "terminal-output";
pub const EVENT_EXIT: &str = "terminal-exit";

/// What the app tells the machine its terminal is. xterm.js behaves like xterm, and
/// every Linux machine knows this name, so colours, mouse and full-screen programs work.
const TERM: &str = "xterm-256color";
/// Settings for the machine's side of the terminal. Its defaults are right for nearly
/// everything; these make Backspace (which the screen sends as DEL) rub out a character,
/// and tell the line editor that one character can be several bytes (UTF-8).
const MODES: &[(Pty, u32)] = &[(Pty::VERASE, 0x7f), (Pty::IUTF8, 1)];
/// How long the machine gets to answer each request (a terminal, then the shell).
const REPLY_LIMIT: Duration = Duration::from_secs(10);
/// Output is collected for this long before it is sent to the screen, so a program
/// printing a character at a time doesn't become an event per character.
const BATCH_WAIT: Duration = Duration::from_millis(5);
/// A batch is sent at once when it reaches this size.
const MAX_BATCH: usize = 64 * 1024;
/// Reading stops while the screen has this much output not yet drawn, and starts again
/// once it has caught up.
const MAX_UNDRAWN: u64 = 512 * 1024;
/// Text typed or pasted in one go. A paste larger than this is surely a mistake.
const MAX_INPUT: usize = 4 * 1024 * 1024;
/// A command to run instead of the shell. Real ones are a line long.
const MAX_COMMAND: usize = 4096;
/// Terminal sizes outside this are not real windows; they are clamped.
const MAX_COLS: u16 = 1000;
const MAX_ROWS: u16 = 500;
/// How long closing gets when the app exits, for all terminals together.
const EXIT_LIMIT: Duration = Duration::from_secs(2);

/// The "terminal-output" event: bytes from the machine, base64-encoded.
#[derive(Debug, Clone, Serialize)]
pub struct OutputEvent {
    pub id: u32,
    pub data: String,
}

/// The "terminal-exit" event. The terminal is gone; anything sent to it now is refused.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitEvent {
    pub id: u32,
    /// Why it ended, for the owner. None for a normal end: the shell was left with
    /// `exit`, the program finished without an error, or the tab was closed.
    pub reason: Option<String>,
    /// The program's exit code, when the machine sent one.
    pub exit_status: Option<u32>,
}

/// What a running terminal reports. The last one is always `Exit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    Output(Vec<u8>),
    Exit { reason: Option<String>, exit_status: Option<u32> },
}

enum Input {
    Data(Vec<u8>),
    Resize(u32, u32),
}

/// How much output has gone to the screen, and how much of it the screen has drawn.
#[derive(Default)]
struct Flow {
    sent: AtomicU64,
    drawn: AtomicU64,
    caught_up: Notify,
}

impl Flow {
    fn undrawn(&self) -> u64 {
        self.sent.load(Ordering::SeqCst).saturating_sub(self.drawn.load(Ordering::SeqCst))
    }
}

/// A terminal whose program is running. Dropping it does not stop it: call `close`.
pub(crate) struct Running {
    input: mpsc::UnboundedSender<Input>,
    stop: Arc<Notify>,
    flow: Arc<Flow>,
    /// Set by the terminal itself once it has finished closing.
    over: Arc<Notify>,
}

impl Running {
    /// Send what was typed or pasted. False if the terminal has ended.
    fn write(&self, data: Vec<u8>) -> bool {
        data.is_empty() || self.input.send(Input::Data(data)).is_ok()
    }

    fn resize(&self, cols: u16, rows: u16) -> bool {
        let (cols, rows) = clamp_size(cols, rows);
        self.input.send(Input::Resize(cols, rows)).is_ok()
    }

    /// The screen has drawn this many more bytes of output.
    fn drawn(&self, bytes: u64) {
        self.flow.drawn.fetch_add(bytes, Ordering::SeqCst);
        self.flow.caught_up.notify_one();
    }

    /// Close the program's channel and the connection. "terminal-exit" follows.
    fn close(&self) {
        // notify_one keeps the wake-up for later if the terminal isn't waiting right now.
        self.stop.notify_one();
    }
}

fn clamp_size(cols: u16, rows: u16) -> (u32, u32) {
    (u32::from(cols.clamp(2, MAX_COLS)), u32::from(rows.clamp(1, MAX_ROWS)))
}

/// Ask `session` for a terminal of the given size and start the login shell, or `command`
/// in its place. Returns the terminal, and the work that carries it, which the caller
/// must spawn: it sends `Event`s to `emit` until the terminal ends, then closes the
/// connection. `host` names the machine in messages.
pub(crate) async fn start(
    session: Session,
    host: &str,
    command: Option<&str>,
    cols: u16,
    rows: u16,
    emit: impl FnMut(Event) + Send + 'static,
) -> std::result::Result<(Running, impl Future<Output = ()> + Send + 'static), SshError> {
    let channel = session.open_channel().await?;
    let (mut read, write) = channel.split();
    let (cols, rows) = clamp_size(cols, rows);
    let lost = || SshError::Other(format!("The connection to {host} was lost while the terminal was starting."));
    let mut early = Vec::new();

    write.request_pty(true, TERM, cols, rows, 0, 0, MODES).await.map_err(|_| lost())?;
    if !reply(&mut read, &mut early, host).await? {
        say_goodbye(read, &write, session).await;
        return Err(SshError::Other(format!(
            "{host} would not open a terminal. Check that its SSH server allows them (PermitTTY)."
        )));
    }
    match command {
        Some(command) => write.exec(true, command).await.map_err(|_| lost())?,
        None => write.request_shell(true).await.map_err(|_| lost())?,
    }
    if !reply(&mut read, &mut early, host).await? {
        say_goodbye(read, &write, session).await;
        return Err(SshError::Other(match command {
            Some(_) => format!("{host} would not run the command."),
            None => format!("{host} would not start a shell."),
        }));
    }

    let (input, inbox) = mpsc::unbounded_channel();
    let running = Running {
        input,
        stop: Arc::new(Notify::new()),
        flow: Arc::new(Flow::default()),
        over: Arc::new(Notify::new()),
    };
    let carry = Carry {
        session,
        host: host.to_string(),
        read,
        write: Arc::new(write),
        inbox,
        stop: running.stop.clone(),
        flow: running.flow.clone(),
        over: running.over.clone(),
        early,
    };
    Ok((running, carry.run(emit)))
}

/// Wait for the machine's yes or no to the last request. Output that comes first is kept.
async fn reply(read: &mut ChannelReadHalf, early: &mut Vec<u8>, host: &str) -> std::result::Result<bool, SshError> {
    let wait = async {
        loop {
            match read.wait().await {
                Some(ChannelMsg::Success) => return Ok(true),
                Some(ChannelMsg::Failure) => return Ok(false),
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    early.extend_from_slice(&data);
                }
                Some(ChannelMsg::Close) | None => {
                    return Err(SshError::Other(format!("{host} ended the session before the terminal started.")));
                }
                Some(_) => {}
            }
        }
    };
    timeout(REPLY_LIMIT, wait).await.unwrap_or_else(|_| {
        Err(SshError::Other(format!("{host} did not answer the request for a terminal in time. Try again.")))
    })
}

/// Close the channel and the connection without waiting long for either. The reading
/// side goes first: while nobody reads it, the connection can't take the close request.
async fn say_goodbye(read: ChannelReadHalf, write: &ChannelWriteHalf<Msg>, session: Session) {
    drop(read);
    let _ = timeout(Duration::from_secs(1), write.close()).await;
    session.close().await;
}

/// Everything a running terminal holds.
struct Carry {
    session: Session,
    host: String,
    read: ChannelReadHalf,
    write: Arc<ChannelWriteHalf<Msg>>,
    inbox: mpsc::UnboundedReceiver<Input>,
    stop: Arc<Notify>,
    flow: Arc<Flow>,
    over: Arc<Notify>,
    early: Vec<u8>,
}

/// How the terminal ended.
enum End {
    /// Closed from this side (the tab, or the app).
    Closed,
    /// The machine closed it: the program ended.
    Ended,
    Lost,
}

impl Carry {
    async fn run(self, mut emit: impl FnMut(Event) + Send + 'static) {
        let Carry { session, host, mut read, write, mut inbox, stop, flow, over, early } = self;

        // Typing goes out in order, on its own, so a machine slow to take it never holds
        // up its output.
        let writer = {
            let write = write.clone();
            tokio::spawn(async move {
                while let Some(input) = inbox.recv().await {
                    let sent = match input {
                        Input::Data(data) => write.data(&data[..]).await,
                        Input::Resize(cols, rows) => write.window_change(cols, rows, 0, 0).await,
                    };
                    if sent.is_err() {
                        break;
                    }
                }
            })
        };

        let mut batch = early;
        let mut send_at = (!batch.is_empty()).then(Instant::now);
        let flush = |batch: &mut Vec<u8>, emit: &mut dyn FnMut(Event)| {
            if !batch.is_empty() {
                flow.sent.fetch_add(batch.len() as u64, Ordering::SeqCst);
                emit(Event::Output(std::mem::take(batch)));
            }
        };
        let mut exit_status = None;
        let mut signal = None;
        let end = loop {
            let behind = flow.undrawn() >= MAX_UNDRAWN;
            tokio::select! {
                biased;
                _ = stop.notified() => break End::Closed,
                _ = sleep_until(send_at.unwrap_or_else(Instant::now)), if send_at.is_some() => {
                    flush(&mut batch, &mut emit);
                    send_at = None;
                }
                _ = flow.caught_up.notified(), if behind => {}
                msg = read.wait(), if !behind => match msg {
                    Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                        batch.extend_from_slice(&data);
                        if batch.len() >= MAX_BATCH {
                            flush(&mut batch, &mut emit);
                            send_at = None;
                        } else if send_at.is_none() {
                            send_at = Some(Instant::now() + BATCH_WAIT);
                        }
                    }
                    Some(ChannelMsg::ExitStatus { exit_status: status }) => exit_status = Some(status),
                    Some(ChannelMsg::ExitSignal { signal_name, error_message, .. }) => {
                        signal = Some((signal_name, error_message));
                    }
                    Some(ChannelMsg::Close) => break End::Ended,
                    // The connection is gone (the machine went away, or its SSH server stopped).
                    None => break End::Lost,
                    // Eof: the program closed its output, but the exit code is still to come.
                    Some(_) => {}
                },
            }
        };
        flush(&mut batch, &mut emit);
        writer.abort();
        say_goodbye(read, &write, session).await;

        let reason = match end {
            End::Closed => None,
            End::Lost => Some(format!("The connection to {host} was lost.")),
            End::Ended => ended_reason(exit_status, signal),
        };
        emit(Event::Exit { reason, exit_status });
        over.notify_one();
    }
}

/// Why the program ended, in plain words. None for a normal end.
fn ended_reason(exit_status: Option<u32>, signal: Option<(Sig, String)>) -> Option<String> {
    if let Some((signal, message)) = signal {
        let name = match signal {
            Sig::Custom(name) => name,
            other => format!("{other:?}"),
        };
        let name: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).take(20).collect();
        let message: String = message.chars().filter(|c| !c.is_control()).take(200).collect();
        let message = message.trim();
        return Some(if message.is_empty() {
            format!("The program was stopped by signal {name}.")
        } else {
            format!("The program was stopped by signal {name}: {message}")
        });
    }
    match exit_status {
        None | Some(0) => None,
        Some(code) => Some(format!("The program ended with exit code {code}.")),
    }
}

// --- The terminals the screens have open ---

/// Every open terminal, by the id the screens know it by.
#[derive(Default)]
pub struct Terminals {
    next_id: AtomicU32,
    open: Mutex<HashMap<u32, Running>>,
}

impl Terminals {
    fn get<T>(&self, id: u32, act: impl FnOnce(&Running) -> T) -> Result<T> {
        let open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        let running = open.get(&id).ok_or_else(|| Error::Invalid("This terminal has already closed.".into()))?;
        Ok(act(running))
    }

    /// Connect to a machine and start a terminal on it. Resolves once the shell (or
    /// `command`) is running, with the terminal's id. Its output may arrive before this
    /// returns.
    pub async fn open(
        &self,
        app: &AppHandle,
        device_id: i64,
        command: Option<String>,
        cols: u16,
        rows: u16,
    ) -> Result<u32> {
        let command = command.map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
        if let Some(c) = &command
            && (c.len() > MAX_COMMAND || c.contains('\0'))
        {
            return Err(Error::Invalid("That command is too long, or has characters a command can't hold.".into()));
        }
        let device = app.state::<Inventory>().get(device_id)?;
        let name = device.name.clone();
        if device.ssh_user.as_deref().is_none_or(|u| u.trim().is_empty()) {
            return Err(Error::Invalid(format!(
                "{name} has no SSH user, so home-cc can't log in to it. Add one with Edit on the Machines screen."
            )));
        }
        let prepared = monitor::login_for(app, &device).await?;
        let (host, port) = (prepared.login.host.clone(), prepared.login.port);
        // A machine reached for the first time has its identity remembered (not approved)
        // before anything is offered to it, as the stats readings do.
        let first_app = app.clone();
        let first_host = host.clone();
        let first_use = move |key: &HostKey| monitor::remember(&first_app, device_id, &first_host, port, key);
        let session = match prepared.connect(&first_use).await {
            Ok(session) => session,
            Err(e) => {
                if matches!(e, SshError::NewHost(_) | SshError::HostKeyChanged { .. }) {
                    // Let the Machines screen show this machine's identity for approval.
                    app.state::<Monitor>().retry_login(device_id);
                }
                return Err(Error::Invalid(open_error(&name, &e)));
            }
        };

        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let events = app.clone();
        let emit = move |event: Event| match event {
            Event::Output(bytes) => {
                let data = data_encoding::BASE64.encode(&bytes);
                events.emit(EVENT_OUTPUT, OutputEvent { id, data }).ok();
            }
            Event::Exit { reason, exit_status } => {
                // Gone from the list first, so nothing more is sent to it.
                let terminals = events.state::<Terminals>();
                terminals.open.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                events.emit(EVENT_EXIT, ExitEvent { id, reason, exit_status }).ok();
            }
        };
        let (running, carry) = start(session, &host, command.as_deref(), cols, rows, emit)
            .await
            .map_err(|e| Error::Invalid(e.to_string()))?;
        // Listed before it runs, so its end always finds it to remove.
        self.open.lock().unwrap_or_else(|e| e.into_inner()).insert(id, running);
        tauri::async_runtime::spawn(carry);
        Ok(id)
    }

    pub fn write(&self, id: u32, data: String) -> Result<()> {
        if data.len() > MAX_INPUT {
            return Err(Error::Invalid("That is too much text to paste into a terminal at once.".into()));
        }
        self.send(id, data.into_bytes())
    }

    /// Bytes the terminal on screen reports by itself, one character per byte: xterm.js
    /// sends mouse clicks in the oldest mouse format this way. Sent in the same queue as
    /// typing, so the two stay in order.
    pub fn write_binary(&self, id: u32, data: &str) -> Result<()> {
        if data.len() > MAX_INPUT {
            return Err(Error::Invalid("That is too much for a terminal at once.".into()));
        }
        self.send(id, binary_bytes(data)?)
    }

    fn send(&self, id: u32, bytes: Vec<u8>) -> Result<()> {
        if self.get(id, |t| t.write(bytes))? {
            Ok(())
        } else {
            Err(Error::Invalid("This terminal has already closed.".into()))
        }
    }

    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> Result<()> {
        self.get(id, |t| t.resize(cols, rows)).map(|_| ())
    }

    /// The screen has drawn `bytes` more of this terminal's output. A terminal that has
    /// closed meanwhile is not an error.
    pub fn ack(&self, id: u32, bytes: u64) {
        let _ = self.get(id, |t| t.drawn(bytes));
    }

    /// Close one terminal. Closing one that has already closed is not an error.
    pub fn close(&self, id: u32) {
        let _ = self.get(id, Running::close);
    }

    /// Close every terminal and wait (a little) until they have said goodbye.
    pub async fn close_all(&self) {
        let waiting: Vec<Arc<Notify>> = {
            let open = self.open.lock().unwrap_or_else(|e| e.into_inner());
            open.values()
                .map(|t| {
                    t.close();
                    t.over.clone()
                })
                .collect()
        };
        let all = async {
            for over in waiting {
                over.notified().await;
            }
        };
        let _ = timeout(EXIT_LIMIT, all).await;
    }
}

/// The bytes in xterm.js's binary input: each character stands for the byte of the same
/// number. A mouse click past column 95 is a byte of 128 or more, which as text would
/// become two bytes of UTF-8 and reach the program as a different click.
fn binary_bytes(data: &str) -> Result<Vec<u8>> {
    data.chars()
        .map(|c| u8::try_from(u32::from(c)).ok())
        .collect::<Option<Vec<u8>>>()
        .ok_or_else(|| Error::Invalid("The terminal on screen sent something that isn't bytes; it was not passed on.".into()))
}

/// Why a terminal could not be opened, saying what to do about it.
fn open_error(name: &str, e: &SshError) -> String {
    match e {
        SshError::NewHost(_) => format!(
            "Logging in to {name} needs its stored password, and home-cc only sends that once you have approved \
             this machine's identity. Approve it on the Machines screen, then open the terminal again."
        ),
        SshError::HostKeyChanged { .. } => format!(
            "{name}'s identity has changed since home-cc first reached it, so nothing was sent to it. Check the new \
             identity on the Machines screen and approve it there if you expected the change, then open the \
             terminal again."
        ),
        other => other.to_string(),
    }
}

// --- Copy and paste ---

/// The one connection to the system clipboard, kept open. On Linux the text the app
/// copied is handed out by this connection, so it has to outlive the copy.
static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);

fn with_clipboard<T>(act: impl FnOnce(&mut arboard::Clipboard) -> std::result::Result<T, arboard::Error>) -> Result<T> {
    let failed = |e: arboard::Error| Error::Invalid(format!("The clipboard could not be used ({e})."));
    let mut held = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    let clipboard = match held.as_mut() {
        Some(clipboard) => clipboard,
        None => held.insert(arboard::Clipboard::new().map_err(failed)?),
    };
    act(clipboard).map_err(|e| {
        // Start afresh next time, in case the connection itself broke.
        if !matches!(e, arboard::Error::ContentNotAvailable) {
            *held = None;
        }
        failed(e)
    })
}

/// The text on the clipboard. Empty if it holds none (an image, say, or nothing).
pub fn read_clipboard() -> Result<String> {
    with_clipboard(|c| match c.get_text() {
        Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
        other => other,
    })
}

pub fn write_clipboard(text: &str) -> Result<()> {
    with_clipboard(|c| c.set_text(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::ssh_key::private::Ed25519Keypair;
    use russh::keys::{HashAlg, PrivateKey};
    use russh::server::{self, Auth, ChannelOpenHandle};
    use russh::{Channel, ChannelId, MethodKind, MethodSet};
    use crate::ssh::{self, Login};

    const USER: &str = "jim";
    const PASSWORD: &str = "correct horse battery";

    /// A small SSH server that offers a pretend shell: it writes down what it is asked
    /// for, echoes what is typed, and answers a few words the way a real program would.
    struct ShellServer {
        port: u16,
        host_key: HostKey,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl ShellServer {
        async fn start() -> ShellServer {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let key = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]));
            let host_key = HostKey {
                algorithm: key.algorithm().to_string(),
                fingerprint: key.public_key().fingerprint(HashAlg::Sha256).to_string(),
            };
            let config = Arc::new(server::Config {
                keys: vec![key],
                methods: MethodSet::from(&[MethodKind::Password][..]),
                auth_rejection_time: Duration::ZERO,
                auth_rejection_time_initial: Some(Duration::ZERO),
                ..Default::default()
            });
            let log = Arc::new(Mutex::new(Vec::new()));
            let seen = log.clone();
            tokio::spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    let handler = Shell { log: seen.clone() };
                    let config = config.clone();
                    tokio::spawn(async move {
                        if let Ok(session) = server::run_stream(config, socket, handler).await {
                            let _ = session.await;
                        }
                    });
                }
            });
            ShellServer { port, host_key, log }
        }

        /// A login the server takes: its identity is stored and approved, so the password goes.
        fn login(&self) -> Login {
            Login {
                host: "127.0.0.1".into(),
                port: self.port,
                user: USER.into(),
                known_key: Some(self.host_key.clone()),
                key_path: Some("/nonexistent/home-cc/test-key".into()),
                key_passphrase: None,
                password: Some(PASSWORD.into()),
            }
        }

        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }

        async fn wait_for_log(&self, line: &str) {
            let found = timeout(Duration::from_secs(5), async {
                while !self.log().iter().any(|l| l == line) {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await;
            assert!(found.is_ok(), "the server never logged {line:?}; it logged {:?}", self.log());
        }
    }

    struct Shell {
        log: Arc<Mutex<Vec<String>>>,
    }

    impl Shell {
        fn note(&self, line: String) {
            self.log.lock().unwrap().push(line);
        }
    }

    impl Drop for Shell {
        fn drop(&mut self) {
            self.note("disconnected".into());
        }
    }

    impl server::Handler for Shell {
        type Error = russh::Error;

        async fn auth_none(&mut self, _user: &str) -> std::result::Result<Auth, russh::Error> {
            Ok(Auth::Reject { proceed_with_methods: Some(MethodSet::from(&[MethodKind::Password][..])), partial_success: false })
        }

        async fn auth_password(&mut self, user: &str, password: &str) -> std::result::Result<Auth, russh::Error> {
            Ok(if user == USER && password == PASSWORD {
                Auth::Accept
            } else {
                Auth::Reject { proceed_with_methods: None, partial_success: false }
            })
        }

        async fn channel_open_session(
            &mut self,
            _channel: Channel<server::Msg>,
            reply: ChannelOpenHandle,
            _session: &mut server::Session,
        ) -> std::result::Result<(), russh::Error> {
            reply.accept().await;
            Ok(())
        }

        async fn pty_request(
            &mut self,
            channel: ChannelId,
            term: &str,
            cols: u32,
            rows: u32,
            _pix_width: u32,
            _pix_height: u32,
            modes: &[(Pty, u32)],
            session: &mut server::Session,
        ) -> std::result::Result<(), russh::Error> {
            let erase = modes.iter().find(|(m, _)| *m == Pty::VERASE).map(|(_, v)| *v);
            self.note(format!("pty {term} {cols}x{rows} erase={erase:?}"));
            session.channel_success(channel)
        }

        async fn shell_request(
            &mut self,
            channel: ChannelId,
            session: &mut server::Session,
        ) -> std::result::Result<(), russh::Error> {
            self.note("shell".into());
            session.channel_success(channel)?;
            session.data(channel, &b"$ "[..])
        }

        async fn exec_request(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut server::Session,
        ) -> std::result::Result<(), russh::Error> {
            let command = String::from_utf8_lossy(data).into_owned();
            self.note(format!("exec {command}"));
            if command == "refuse" {
                return session.channel_failure(channel);
            }
            session.channel_success(channel)?;
            session.data(channel, format!("ran {command}\r\n").into_bytes())?;
            session.exit_status_request(channel, 7)?;
            session.eof(channel)?;
            session.close(channel)
        }

        async fn window_change_request(
            &mut self,
            channel: ChannelId,
            cols: u32,
            rows: u32,
            _pix_width: u32,
            _pix_height: u32,
            session: &mut server::Session,
        ) -> std::result::Result<(), russh::Error> {
            self.note(format!("size {cols}x{rows}"));
            session.data(channel, format!("[size {cols}x{rows}]").into_bytes())
        }

        async fn data(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut server::Session,
        ) -> std::result::Result<(), russh::Error> {
            self.note(format!("typed {}", String::from_utf8_lossy(data)));
            match data {
                b"exit\r" => {
                    session.exit_status_request(channel, 0)?;
                    session.eof(channel)?;
                    session.close(channel)
                }
                b"false\r" => {
                    session.exit_status_request(channel, 1)?;
                    session.close(channel)
                }
                // "é" in two packets: the first byte alone is not a character.
                b"accent\r" => {
                    session.data(channel, &[0xc3][..])?;
                    session.data(channel, &[0xa9][..])
                }
                b"crash\r" => Err(russh::Error::Disconnect),
                b"flood\r" => {
                    for _ in 0..64 {
                        session.data(channel, vec![b'x'; 32 * 1024])?;
                    }
                    session.data(channel, &b"END"[..])
                }
                other => session.data(channel, other.to_vec()),
            }
        }

        async fn channel_close(
            &mut self,
            _channel: ChannelId,
            _session: &mut server::Session,
        ) -> std::result::Result<(), russh::Error> {
            self.note("channel closed".into());
            Ok(())
        }
    }

    /// The terminal's events, as they arrive.
    struct Seen {
        events: mpsc::UnboundedReceiver<Event>,
        output: Vec<u8>,
        exit: Option<(Option<String>, Option<u32>)>,
    }

    impl Seen {
        /// Read events until the output holds `text`, the terminal ends, or time runs out.
        async fn until(&mut self, text: &[u8]) {
            let wait = async {
                while !contains(&self.output, text) && self.exit.is_none() {
                    match self.events.recv().await {
                        Some(Event::Output(bytes)) => self.output.extend(bytes),
                        Some(Event::Exit { reason, exit_status }) => self.exit = Some((reason, exit_status)),
                        None => break,
                    }
                }
            };
            let _ = timeout(Duration::from_secs(5), wait).await;
            assert!(
                contains(&self.output, text),
                "never saw {:?}; saw {:?}",
                String::from_utf8_lossy(text),
                String::from_utf8_lossy(&self.output)
            );
        }

        async fn until_exit(&mut self) -> (Option<String>, Option<u32>) {
            let wait = async {
                while self.exit.is_none() {
                    match self.events.recv().await {
                        Some(Event::Output(bytes)) => self.output.extend(bytes),
                        Some(Event::Exit { reason, exit_status }) => self.exit = Some((reason, exit_status)),
                        None => break,
                    }
                }
            };
            timeout(Duration::from_secs(5), wait).await.expect("the terminal never ended");
            self.exit.clone().expect("the terminal never ended")
        }
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        needle.is_empty() || haystack.windows(needle.len()).any(|w| w == needle)
    }

    async fn open(server: &ShellServer, command: Option<&str>) -> std::result::Result<(Running, Seen), SshError> {
        let session = ssh::connect(&server.login(), &|_: &HostKey| {}).await?;
        let (tx, events) = mpsc::unbounded_channel();
        let emit = move |e: Event| {
            let _ = tx.send(e);
        };
        let (running, carry) = start(session, "test machine", command, 100, 30, emit).await?;
        tokio::spawn(carry);
        Ok((running, Seen { events, output: Vec::new(), exit: None }))
    }

    #[tokio::test]
    async fn shell_gets_a_terminal_and_typing_goes_both_ways() {
        let server = ShellServer::start().await;
        let (running, mut seen) = open(&server, None).await.unwrap();
        assert_eq!(server.log()[..2], ["pty xterm-256color 100x30 erase=Some(127)", "shell"]);
        seen.until(b"$ ").await;

        assert!(running.write(b"ls -l\r".to_vec()));
        seen.until(b"ls -l\r").await;
        // Escape, arrows and control keys are plain bytes and go through untouched.
        assert!(running.write(b"\x1b[A\x1b:q\x03".to_vec()));
        seen.until(b"\x1b[A\x1b:q\x03").await;

        assert!(running.resize(120, 40));
        server.wait_for_log("size 120x40").await;
        seen.until(b"[size 120x40]").await;

        assert!(running.write(b"exit\r".to_vec()));
        assert_eq!(seen.until_exit().await, (None, Some(0)));
        server.wait_for_log("disconnected").await;
        assert!(!running.write(b"more".to_vec()), "typing into an ended terminal is refused");
    }

    #[tokio::test]
    async fn mouse_reports_from_the_screen_arrive_as_the_bytes_they_are() {
        let server = ShellServer::start().await;
        let (running, mut seen) = open(&server, None).await.unwrap();
        seen.until(b"$ ").await;
        let terminals = Terminals::default();
        terminals.open.lock().unwrap().insert(1, running);

        // A click at column 120, row 10 in the oldest mouse format: ESC [ M, then the button and the
        // place, each plus 32. 152 is one byte, not the two bytes of UTF-8 that text would make of it.
        terminals.write_binary(1, "\x1b[M\u{20}\u{98}\u{2a}").unwrap();
        seen.until(&[0x1b, b'[', b'M', 0x20, 0x98, 0x2a]).await;
        // Text and bytes share one queue, so they stay in order.
        terminals.write(1, "é".into()).unwrap();
        terminals.write_binary(1, "\u{ff}").unwrap();
        seen.until(&[0xc3, 0xa9, 0xff]).await;

        assert!(terminals.write_binary(1, "\u{100}").is_err(), "a character past 255 is not a byte");
        assert!(terminals.write_binary(2, "x").is_err(), "an unknown terminal is refused");
        terminals.close(1);
        assert_eq!(seen.until_exit().await, (None, None));
    }

    #[test]
    fn binary_input_is_one_byte_per_character() {
        assert_eq!(binary_bytes("").unwrap(), Vec::<u8>::new());
        assert_eq!(binary_bytes("\u{0}a\u{7f}\u{80}\u{ff}").unwrap(), vec![0, b'a', 0x7f, 0x80, 0xff]);
        assert!(binary_bytes("€").is_err());
    }

    #[tokio::test]
    async fn a_character_split_across_packets_arrives_whole() {
        let server = ShellServer::start().await;
        let (running, mut seen) = open(&server, None).await.unwrap();
        running.write(b"accent\r".to_vec());
        seen.until("é".as_bytes()).await;
    }

    #[tokio::test]
    async fn closing_the_tab_closes_the_channel_and_the_connection() {
        let server = ShellServer::start().await;
        let (running, mut seen) = open(&server, None).await.unwrap();
        seen.until(b"$ ").await;
        running.close();
        assert_eq!(seen.until_exit().await, (None, None));
        server.wait_for_log("channel closed").await;
        server.wait_for_log("disconnected").await;
    }

    #[tokio::test]
    async fn a_command_runs_in_place_of_the_shell_and_its_exit_code_is_reported() {
        let server = ShellServer::start().await;
        let (_running, mut seen) = open(&server, Some("docker exec -it web sh")).await.unwrap();
        seen.until(b"ran docker exec -it web sh").await;
        let (reason, status) = seen.until_exit().await;
        assert_eq!(status, Some(7));
        assert_eq!(reason.as_deref(), Some("The program ended with exit code 7."));
        assert!(server.log().contains(&"exec docker exec -it web sh".to_string()));
        assert!(!server.log().contains(&"shell".to_string()));
        server.wait_for_log("disconnected").await;
    }

    #[tokio::test]
    async fn a_refused_command_is_an_error_and_the_connection_is_closed() {
        let server = ShellServer::start().await;
        let error = open(&server, Some("refuse")).await.err().expect("a refused command can't start");
        assert_eq!(error.to_string(), "test machine would not run the command.");
        server.wait_for_log("disconnected").await;
    }

    #[tokio::test]
    async fn output_waits_while_the_screen_is_behind() {
        let server = ShellServer::start().await;
        let (running, mut seen) = open(&server, None).await.unwrap();
        seen.until(b"$ ").await;
        running.write(b"flood\r".to_vec());
        // Nothing has been drawn yet: reading stops at the limit, give or take a batch.
        tokio::time::sleep(Duration::from_millis(300)).await;
        while let Ok(Event::Output(bytes)) = seen.events.try_recv() {
            seen.output.extend(bytes);
        }
        let first = seen.output.len() as u64;
        assert!(first >= MAX_UNDRAWN, "only {first} bytes arrived");
        assert!(first < MAX_UNDRAWN + MAX_BATCH as u64 + 64 * 1024, "{first} bytes arrived unread");
        assert!(!contains(&seen.output, b"END"));
        // Once the screen catches up, the rest follows.
        let drawer = |bytes: u64| running.drawn(bytes);
        drawer(first);
        let wait = async {
            while !contains(&seen.output, b"END") {
                match seen.events.recv().await {
                    Some(Event::Output(bytes)) => {
                        drawer(bytes.len() as u64);
                        seen.output.extend(bytes);
                    }
                    _ => break,
                }
            }
        };
        timeout(Duration::from_secs(5), wait).await.expect("the rest of the output never came");
        assert_eq!(seen.output.iter().filter(|b| **b == b'x').count(), 64 * 32 * 1024);
    }

    #[tokio::test]
    async fn a_dropped_connection_says_so() {
        let server = ShellServer::start().await;
        let (running, mut seen) = open(&server, None).await.unwrap();
        seen.until(b"$ ").await;
        // The server goes away without closing anything, as a machine that crashes would.
        running.write(b"crash\r".to_vec());
        let (reason, status) = seen.until_exit().await;
        assert_eq!(reason.as_deref(), Some("The connection to test machine was lost."));
        assert_eq!(status, None);
    }

    #[test]
    fn the_end_of_a_program_is_told_in_plain_words() {
        assert_eq!(ended_reason(None, None), None);
        assert_eq!(ended_reason(Some(0), None), None);
        assert_eq!(ended_reason(Some(2), None).as_deref(), Some("The program ended with exit code 2."));
        assert_eq!(
            ended_reason(Some(137), Some((Sig::KILL, String::new()))).as_deref(),
            Some("The program was stopped by signal KILL.")
        );
        // What the machine sends is cleaned before it is shown.
        assert_eq!(
            ended_reason(None, Some((Sig::Custom("WINCH\u{1b}[2J".into()), "bye\nnow".into()))).as_deref(),
            Some("The program was stopped by signal WINCH2J: byenow")
        );
    }

    /// Against a real OpenSSH server with vim, tmux and htop installed, which the tests can't
    /// count on. Run it with
    /// `HOME_CC_TEST_SSH=host:port:user:password cargo test real_server -- --ignored`.
    /// It checks what "done" means for terminal tabs: the terminal has the size asked for and
    /// follows a resize, and full-screen programs start (switch to the second screen), take
    /// keys (Escape, Ctrl) and give the screen back when they end.
    #[tokio::test]
    #[ignore = "needs a real SSH server: set HOME_CC_TEST_SSH=host:port:user:password"]
    async fn real_server_runs_full_screen_programs() {
        let spec = std::env::var("HOME_CC_TEST_SSH").expect("set HOME_CC_TEST_SSH=host:port:user:password");
        let mut parts = spec.splitn(4, ':');
        let (host, port, user, password) =
            (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
        let mut login = Login {
            host: host.into(),
            port: port.parse().unwrap(),
            user: user.into(),
            known_key: None,
            key_path: Some("/nonexistent/home-cc/test-key".into()),
            key_passphrase: None,
            password: Some(password.into()),
        };
        // First contact: the password is held back until the identity is approved, as in the app.
        let shown = Mutex::new(None);
        let first = ssh::connect(&login, &|key: &HostKey| *shown.lock().unwrap() = Some(key.clone())).await;
        assert!(matches!(first, Err(SshError::NewHost(_))), "{first:?}");
        login.known_key = shown.lock().unwrap().clone();

        let session = ssh::connect(&login, &|_: &HostKey| {}).await.unwrap();
        let (tx, events) = mpsc::unbounded_channel();
        let (running, carry) = start(session, host, None, 100, 30, move |e| {
            let _ = tx.send(e);
        })
        .await
        .unwrap();
        tokio::spawn(carry);
        let mut seen = Seen { events, output: Vec::new(), exit: None };
        let step = async |seen: &mut Seen, input: &[u8], expect: &[u8]| {
            seen.output.clear();
            running.write(input.to_vec());
            seen.until(expect).await;
        };

        step(&mut seen, b"echo $TERM; stty size\r", b"xterm-256color\r\n30 100").await;
        running.resize(120, 40);
        step(&mut seen, b"stty size\r", b"40 120").await;
        // UTF-8 both ways.
        step(&mut seen, "echo 'naïve ünïcödé'\r".as_bytes(), "\rnaïve ünïcödé\r\n".as_bytes()).await;
        // Ctrl+C stops a running program.
        step(&mut seen, b"sleep 30\r", b"sleep 30").await;
        step(&mut seen, b"\x03", b"^C").await;

        step(&mut seen, b"vim -u NONE\r", b"\x1b[?1049h").await;
        // Insert mode, some text, Escape, then quit without saving.
        step(&mut seen, b"ihello", b"hello").await;
        step(&mut seen, b"\x1b:q!\r", b"\x1b[?1049l").await;

        step(&mut seen, b"tmux -f /dev/null new -s homecc\r", b"\x1b[?1049h").await;
        // Ctrl+B then d: detach.
        step(&mut seen, b"\x02d", b"[detached").await;
        step(&mut seen, b"tmux kill-server; echo killed\r", b"\rkilled\r\n").await;

        step(&mut seen, b"htop\r", b"\x1b[?1049h").await;
        step(&mut seen, b"q", b"\x1b[?1049l").await;

        running.write(b"exit\r".to_vec());
        assert_eq!(seen.until_exit().await, (None, Some(0)));
    }

    #[test]
    fn sizes_are_kept_to_real_windows() {
        assert_eq!(clamp_size(0, 0), (2, 1));
        assert_eq!(clamp_size(80, 24), (80, 24));
        assert_eq!(clamp_size(u16::MAX, u16::MAX), (1000, 500));
    }

    #[test]
    fn opening_failures_say_where_to_approve() {
        let key = HostKey { algorithm: "ssh-ed25519".into(), fingerprint: "SHA256:abc".into() };
        let text = open_error("NAS", &SshError::NewHost(key.clone()));
        assert!(text.contains("Approve it on the Machines screen"), "{text}");
        // A failed login is only that: where approving would help, the login says NewHost.
        let text = open_error("NAS", &SshError::AuthFailed("Nothing worked.".into()));
        assert_eq!(text, "Nothing worked.");
        let changed = SshError::HostKeyChanged { expected: key.clone(), got: key };
        assert!(open_error("NAS", &changed).contains("identity has changed"));
    }

    #[test]
    fn terminals_and_their_work_can_cross_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<Terminals>();
    }

    /// Never run: it only has to compile.
    #[allow(dead_code)]
    fn starting_can_cross_threads(session: Session) {
        fn sendable<T: Send>(_: &T) {}
        sendable(&start(session, "x", None, 80, 24, |_| {}));
    }
}
