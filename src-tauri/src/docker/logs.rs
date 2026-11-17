//! Following a container's log over an SSH channel. `docker logs --follow` runs with a
//! terminal (PTY) on the channel, so that closing the channel hangs it up and it
//! stops on the machine, even on servers that ignore a request to stop it. The
//! terminal is set not to touch the output, and lines are handed on in batches a
//! few times a second, so a chatty container can't flood the screen.

use russh::client::Msg;
use russh::{Channel, ChannelMsg, Pty};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

/// How often lines that have arrived are handed on.
const FLUSH_EVERY: Duration = Duration::from_millis(200);
/// Handed on straight away once this many lines are waiting.
const BATCH: usize = 1000;
/// A line longer than this is split. Some programs log a whole JSON document per line.
const MAX_LINE: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct LogLines {
    pub id: u32,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEnd {
    pub id: u32,
    /// Why the log ended, unless the owner closed it.
    pub reason: Option<String>,
}

/// Splits the stream into lines. Bytes of a line that hasn't ended yet wait for the
/// rest, so a character split across two packets comes out whole.
#[derive(Default)]
pub struct Lines {
    partial: Vec<u8>,
}

impl Lines {
    pub fn push(&mut self, data: &[u8], out: &mut Vec<String>) {
        for &b in data {
            if b == b'\n' {
                out.push(self.take());
            } else {
                self.partial.push(b);
                if self.partial.len() >= MAX_LINE {
                    out.push(self.take());
                }
            }
        }
    }

    /// What is left when the stream ends.
    pub fn finish(&mut self) -> Option<String> {
        (!self.partial.is_empty()).then(|| self.take())
    }

    fn take(&mut self) -> String {
        let mut bytes = std::mem::take(&mut self.partial);
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// Why a log ended, from how docker ended and the last thing it said.
fn end_reason(exit: Option<u32>, last: Option<&str>, host: &str) -> String {
    let said = last.map(str::trim).filter(|s| !s.is_empty());
    match (exit, said) {
        (Some(0), _) => "The container stopped, so its log ends here.".into(),
        (Some(_), Some(s)) if s.contains("No such container") => "This container is no longer there.".into(),
        (Some(_), Some(s)) => format!("Docker stopped following the log: {}", super::parse::text(s)),
        (Some(code), None) => format!("Docker stopped following the log (it ended with {code})."),
        (None, _) => format!("The connection to {host} was lost."),
    }
}

/// Run `command` on the channel and hand on its output line by line until it ends or
/// `stop` is notified. Returns why it ended, or None if it was stopped.
pub async fn follow(
    channel: Channel<Msg>,
    command: &str,
    stop: Arc<Notify>,
    mut emit: impl FnMut(Vec<String>) + Send,
    host: &str,
) -> Option<String> {
    // Wide, so nothing is tempted to wrap; output passed through as it is (no CR added).
    let modes = [(Pty::OPOST, 0), (Pty::ECHO, 0)];
    if let Err(e) = channel.request_pty(false, "dumb", 1000, 50, 0, 0, &modes).await {
        return Some(format!("Couldn't start following the log on {host} ({e})."));
    }
    if let Err(e) = channel.exec(true, command).await {
        return Some(format!("Couldn't start following the log on {host} ({e})."));
    }
    let mut channel = channel;
    let mut lines = Lines::default();
    let mut waiting: Vec<String> = Vec::new();
    let mut last_line: Option<String> = None;
    let mut exit: Option<u32> = None;
    let mut tick = tokio::time::interval(FLUSH_EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut flush = |waiting: &mut Vec<String>, last_line: &mut Option<String>| {
        if let Some(l) = waiting.iter().rev().find(|l| !l.trim().is_empty()) {
            *last_line = Some(l.clone());
        }
        if !waiting.is_empty() {
            emit(std::mem::take(waiting));
        }
    };

    let reason = loop {
        tokio::select! {
            biased;
            _ = stop.notified() => {
                flush(&mut waiting, &mut last_line);
                // Closing the channel hangs up the terminal, which ends docker logs there.
                let _ = tokio::time::timeout(Duration::from_secs(2), channel.close()).await;
                break None;
            }
            msg = channel.wait() => match msg {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    lines.push(&data, &mut waiting);
                    if waiting.len() >= BATCH {
                        flush(&mut waiting, &mut last_line);
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) => exit = Some(exit_status),
                Some(ChannelMsg::Failure) => {
                    break Some(format!("{host} refused to run docker logs."));
                }
                Some(ChannelMsg::Close) | None => {
                    waiting.extend(lines.finish());
                    flush(&mut waiting, &mut last_line);
                    break Some(end_reason(exit, last_line.as_deref(), host));
                }
                Some(_) => {}
            },
            _ = tick.tick() => flush(&mut waiting, &mut last_line),
        }
    };
    reason
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh::{self, Login};
    use russh::keys::PrivateKey;
    use russh::keys::ssh_key::private::Ed25519Keypair;
    use russh::server::{self, Auth, ChannelOpenHandle, Msg as ServerMsg, PublicKeyOffer, Session as ServerSession, VerifiedPublicKey};
    use russh::{ChannelId, MethodKind, MethodSet};
    use std::sync::Mutex;

    #[test]
    fn lines_are_split_and_joined_across_packets() {
        let mut lines = Lines::default();
        let mut out = Vec::new();
        lines.push(b"2026-11-17T10:00:00Z first\r\n2026-11-17T10:00:01Z sec", &mut out);
        assert_eq!(out, ["2026-11-17T10:00:00Z first"]);
        // "é" split in two between packets.
        lines.push(b"ond caf\xc3", &mut out);
        lines.push(b"\xa9\n", &mut out);
        assert_eq!(out[1], "2026-11-17T10:00:01Z second café");
        lines.push(b"no end", &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(lines.finish().as_deref(), Some("no end"));
        assert_eq!(lines.finish(), None);

        let mut long = Lines::default();
        let mut out = Vec::new();
        long.push(&vec![b'x'; MAX_LINE * 2 + 5], &mut out);
        assert_eq!(out.len(), 2, "a very long line comes out in pieces");
        assert_eq!(long.finish().map(|s| s.len()), Some(5));
    }

    #[test]
    fn end_reasons_read_plainly() {
        assert!(end_reason(Some(0), None, "nas").contains("stopped"));
        assert_eq!(end_reason(Some(1), Some("Error: No such container: abc"), "nas"), "This container is no longer there.");
        assert!(end_reason(Some(1), Some("something odd"), "nas").ends_with("something odd"));
        assert_eq!(end_reason(None, None, "nas"), "The connection to nas was lost.");
    }

    /// A server that lets anyone with a key in and, for each command, writes what the
    /// test asked for. It notes whether a terminal was asked for and when the channel
    /// was closed by the client.
    #[derive(Clone)]
    struct LogServer {
        /// What to send, and then whether to end the command with this exit status.
        script: Arc<(Vec<&'static [u8]>, Option<u32>)>,
        seen: Arc<Mutex<Vec<String>>>,
    }

    impl server::Handler for LogServer {
        type Error = russh::Error;

        async fn auth_publickey_offered(&mut self, _: &str, _: PublicKeyOffer<'_>) -> Result<Auth, Self::Error> {
            Ok(Auth::Accept)
        }

        async fn auth_publickey(&mut self, _: &str, _: VerifiedPublicKey<'_>) -> Result<Auth, Self::Error> {
            Ok(Auth::Accept)
        }

        async fn channel_open_session(
            &mut self,
            _: Channel<ServerMsg>,
            reply: ChannelOpenHandle,
            _: &mut ServerSession,
        ) -> Result<(), Self::Error> {
            reply.accept().await;
            Ok(())
        }

        #[allow(clippy::too_many_arguments)]
        async fn pty_request(
            &mut self,
            _: ChannelId,
            term: &str,
            _: u32,
            _: u32,
            _: u32,
            _: u32,
            modes: &[(Pty, u32)],
            _: &mut ServerSession,
        ) -> Result<(), Self::Error> {
            let opost = modes.iter().find(|(m, _)| *m == Pty::OPOST).map(|(_, v)| *v);
            self.seen.lock().unwrap().push(format!("pty {term} opost={opost:?}"));
            Ok(())
        }

        async fn exec_request(
            &mut self,
            channel: ChannelId,
            data: &[u8],
            session: &mut ServerSession,
        ) -> Result<(), Self::Error> {
            self.seen.lock().unwrap().push(format!("exec {}", String::from_utf8_lossy(data)));
            session.channel_success(channel)?;
            let handle = session.handle();
            let script = self.script.clone();
            tokio::spawn(async move {
                for chunk in &script.0 {
                    let _ = handle.data(channel, chunk.to_vec()).await;
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
                if let Some(code) = script.1 {
                    let _ = handle.exit_status_request(channel, code).await;
                    let _ = handle.eof(channel).await;
                    let _ = handle.close(channel).await;
                }
            });
            Ok(())
        }

        async fn channel_close(&mut self, _: ChannelId, _: &mut ServerSession) -> Result<(), Self::Error> {
            self.seen.lock().unwrap().push("closed by client".into());
            Ok(())
        }
    }

    async fn start(script: (Vec<&'static [u8]>, Option<u32>)) -> (u16, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = Arc::new(server::Config {
            keys: vec![PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]))],
            methods: MethodSet::from(&[MethodKind::PublicKey][..]),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        });
        let seen = Arc::new(Mutex::new(Vec::new()));
        let handler = LogServer { script: Arc::new(script), seen: seen.clone() };
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let (config, handler) = (config.clone(), handler.clone());
                tokio::spawn(async move {
                    if let Ok(session) = server::run_stream(config, socket, handler).await {
                        let _ = session.await;
                    }
                });
            }
        });
        (port, seen)
    }

    async fn session(port: u16) -> ssh::Session {
        let dir = std::env::temp_dir().join(format!("home-cc-docker-logs-{}-{port}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("key");
        std::fs::write(&key, ssh::test_server::client_key_file()).unwrap();
        let login = Login {
            host: "127.0.0.1".into(),
            port,
            user: "jim".into(),
            known_key: None,
            key_path: Some(key),
            key_passphrase: None,
            password: None,
        };
        ssh::connect(&login, &|_| {}).await.unwrap()
    }

    #[tokio::test]
    async fn a_log_is_followed_until_the_container_stops() {
        let (port, seen) = start((vec![b"one\ntw", b"o\nthree\n"], Some(0))).await;
        let session = session(port).await;
        let channel = session.open_channel().await.unwrap();
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let reason = follow(channel, "docker logs --follow abc", Arc::new(Notify::new()), move |l| sink.lock().unwrap().extend(l), "vm").await;
        assert_eq!(*got.lock().unwrap(), ["one", "two", "three"]);
        assert_eq!(reason.as_deref(), Some("The container stopped, so its log ends here."));
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen[0], "pty dumb opost=Some(0)", "a terminal that leaves the output alone");
        assert_eq!(seen[1], "exec docker logs --follow abc");
    }

    #[tokio::test]
    async fn closing_the_view_closes_the_channel() {
        // The server never ends this one by itself, like a quiet container.
        let (port, seen) = start((vec![b"hello\n"], None)).await;
        let session = session(port).await;
        let channel = session.open_channel().await.unwrap();
        let stop = Arc::new(Notify::new());
        let got = Arc::new(Mutex::new(Vec::new()));
        let sink = got.clone();
        let follower = tokio::spawn({
            let stop = stop.clone();
            async move { follow(channel, "docker logs --follow abc", stop, move |l| sink.lock().unwrap().extend(l), "vm").await }
        });
        // Wait for the line, then close.
        for _ in 0..100 {
            if !got.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(*got.lock().unwrap(), ["hello"]);
        stop.notify_one();
        let reason = tokio::time::timeout(Duration::from_secs(5), follower).await.unwrap().unwrap();
        assert_eq!(reason, None, "stopped by the owner, not ended");
        for _ in 0..100 {
            if seen.lock().unwrap().iter().any(|s| s == "closed by client") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the server never saw the channel close: {:?}", seen.lock().unwrap());
    }
}
