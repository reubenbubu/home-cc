//! Guessing a machine's system from what answers on the network, without
//! logging in and without administrator rights: a short TCP connection to a
//! few ports, and the greeting its SSH server sends before any login. It is the
//! weakest way home-cc tells Linux from Windows (see `inventory::may_replace`),
//! used only while nothing better is known. The ping TTL would be another hint
//! (64 for Linux, 128 for Windows), but ping-async doesn't give it.

use crate::inventory::Os;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::task::JoinSet;
use tokio::time::{Instant, timeout};

/// Windows' own services: its RPC endpoint (135), file sharing (445) and Remote
/// Desktop (3389).
const WINDOWS_PORTS: [u16; 3] = [135, 445, 3389];
/// Proxmox's web interface.
const PROXMOX_PORT: u16 = 8006;
/// How long a port gets to accept the connection. Machines on the home network
/// answer in a few milliseconds; a port that says nothing is closed or filtered.
const CONNECT_LIMIT: Duration = Duration::from_millis(800);
/// How long an SSH server gets to send its first line, once connected.
const GREETING_LIMIT: Duration = Duration::from_millis(1200);
/// How long a host name gets to resolve.
const NAME_LIMIT: Duration = Duration::from_secs(2);
/// An SSH greeting is at most 255 characters (RFC 4253).
const GREETING_MAX: usize = 255;
/// The most read from an SSH port: the greeting, and a few lines a server may
/// send before it.
const SENT_MAX: usize = 1024;

/// What answered on a machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answered {
    /// The ports tried, other than SSH, that took the connection.
    pub open: Vec<u16>,
    /// The first line from its SSH port, such as "SSH-2.0-OpenSSH_9.2p1 Debian-2+deb12u3".
    pub ssh_greeting: Option<String>,
}

/// The system that what answered points to, if it points to one:
/// - Windows' own SSH server ("OpenSSH_for_Windows") is Windows.
/// - Another OpenSSH greeting is Linux, also with file sharing (445, Samba),
///   and Proxmox with Proxmox's web interface (8006) as well.
/// - Windows' RPC endpoint or Remote Desktop, or file sharing without Linux's
///   SSH, is Windows.
/// - Both kinds together (xrdp on Linux, or WSL's SSH on Windows), or nothing
///   telling, is no guess.
pub fn guess(a: &Answered) -> Option<Os> {
    let open = |port: u16| a.open.contains(&port);
    let greeting = a.ssh_greeting.as_deref().unwrap_or("");
    if !greeting.starts_with("SSH-") {
        return windows_only(a);
    }
    if greeting.contains("OpenSSH_for_Windows") {
        return Some(Os::Windows);
    }
    if !greeting.contains("OpenSSH") {
        // Dropbear on a router, or some other server: nothing telling.
        return windows_only(a);
    }
    match (open(135) || open(3389), open(PROXMOX_PORT)) {
        (true, _) => None,
        (false, true) => Some(Os::Proxmox),
        (false, false) => Some(Os::Linux),
    }
}

/// Windows, when one of its own services answers.
fn windows_only(a: &Answered) -> Option<Os> {
    WINDOWS_PORTS.iter().any(|p| a.open.contains(p)).then_some(Os::Windows)
}

/// Look at what answers at `host` (an address or a name) and guess its system
/// from that. `ssh_port` is where its SSH server is expected. Never takes much
/// more than about 3 seconds, and never sends anything but the connections.
pub async fn probe(host: &str, ssh_port: u16) -> Option<Os> {
    let ip = address_of(host).await?;
    let mut ports = WINDOWS_PORTS.to_vec();
    ports.push(PROXMOX_PORT);
    ports.retain(|p| *p != ssh_port);
    guess(&answered(ip, ssh_port, &ports).await)
}

/// The address to try for `host`: as written, or what the name resolves to
/// (an IPv4 address first).
async fn address_of(host: &str) -> Option<IpAddr> {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Some(ip);
    }
    let found: Vec<SocketAddr> = timeout(NAME_LIMIT, tokio::net::lookup_host((host, 0))).await.ok()?.ok()?.collect();
    found.iter().find(|a| a.is_ipv4()).or(found.first()).map(SocketAddr::ip)
}

/// Every port in `ports` that takes a connection, and the greeting on `ssh_port`,
/// all tried at once.
pub(crate) async fn answered(ip: IpAddr, ssh_port: u16, ports: &[u16]) -> Answered {
    let mut tries = JoinSet::new();
    for &port in ports {
        tries.spawn(async move { connects(SocketAddr::new(ip, port)).await.then_some(port) });
    }
    let ssh_greeting = greeting(SocketAddr::new(ip, ssh_port)).await;
    let mut open = Vec::new();
    while let Some(done) = tries.join_next().await {
        open.extend(done.ok().flatten());
    }
    open.sort_unstable();
    Answered { open, ssh_greeting }
}

/// Whether the port takes a connection. It is closed straight away.
async fn connects(addr: SocketAddr) -> bool {
    matches!(timeout(CONNECT_LIMIT, TcpStream::connect(addr)).await, Ok(Ok(_)))
}

/// The first line an SSH server sends, cleaned for showing. None if nothing
/// answers there or nothing is said in time.
async fn greeting(addr: SocketAddr) -> Option<String> {
    let stream = timeout(CONNECT_LIMIT, TcpStream::connect(addr)).await.ok()?.ok()?;
    let deadline = Instant::now() + GREETING_LIMIT;
    let mut got = Vec::new();
    let mut buf = [0u8; GREETING_MAX + 2];
    while !has_greeting(&got) && got.len() < SENT_MAX {
        timeout(deadline.saturating_duration_since(Instant::now()), stream.readable()).await.ok()?.ok()?;
        match stream.try_read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
            Err(_) => break,
        }
    }
    first_line(&got)
}

/// Whether what a server sent so far holds a whole line starting "SSH-" (the
/// greeting, which may come after other lines).
fn has_greeting(sent: &[u8]) -> bool {
    let lines: Vec<&[u8]> = sent.split(|b| *b == b'\n').collect();
    // The last piece has no line ending yet.
    lines[..lines.len() - 1].iter().any(|l| l.starts_with(b"SSH-"))
}

/// The SSH greeting in what a server sent: its first line starting "SSH-"
/// (servers may send other lines first), without control characters.
pub(crate) fn first_line(sent: &[u8]) -> Option<String> {
    String::from_utf8_lossy(sent)
        .lines()
        .map(|l| l.chars().filter(|c| !c.is_control()).take(GREETING_MAX).collect::<String>())
        .find(|l| l.starts_with("SSH-"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use tokio::net::TcpListener;

    fn seen(open: &[u16], greeting: Option<&str>) -> Option<Os> {
        guess(&Answered { open: open.to_vec(), ssh_greeting: greeting.map(Into::into) })
    }

    const DEBIAN: &str = "SSH-2.0-OpenSSH_9.2p1 Debian-2+deb12u3";
    const WINDOWS: &str = "SSH-2.0-OpenSSH_for_Windows_9.5";

    #[test]
    fn ports_point_to_a_system() {
        // Linux: only its SSH server.
        assert_eq!(seen(&[], Some(DEBIAN)), Some(Os::Linux));
        assert_eq!(seen(&[], Some("SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13.5")), Some(Os::Linux));
        // With Samba's file sharing it is still Linux.
        assert_eq!(seen(&[445], Some(DEBIAN)), Some(Os::Linux));
        // Proxmox: its web interface next to Debian's SSH.
        assert_eq!(seen(&[8006], Some(DEBIAN)), Some(Os::Proxmox));
        // Windows' own services, with no SSH or with Windows' own SSH server.
        assert_eq!(seen(&[135, 445], None), Some(Os::Windows));
        assert_eq!(seen(&[3389], None), Some(Os::Windows));
        assert_eq!(seen(&[445], None), Some(Os::Windows));
        assert_eq!(seen(&[], Some(WINDOWS)), Some(Os::Windows));
        assert_eq!(seen(&[135, 445, 3389], Some(WINDOWS)), Some(Os::Windows));
        // Windows with some other SSH server.
        assert_eq!(seen(&[135], Some("SSH-2.0-9.99 FlowSsh: Bitvise SSH Server (WinSSHD)")), Some(Os::Windows));
    }

    #[test]
    fn mixed_or_quiet_answers_are_no_guess() {
        assert_eq!(seen(&[], None), None, "nothing answered");
        // Remote Desktop next to Linux's SSH: xrdp, or WSL's SSH on Windows.
        assert_eq!(seen(&[3389], Some(DEBIAN)), None);
        assert_eq!(seen(&[135, 445], Some(DEBIAN)), None);
        // A router's Dropbear, or something on port 22 that isn't SSH at all.
        assert_eq!(seen(&[], Some("SSH-2.0-dropbear_2022.83")), None);
        assert_eq!(seen(&[], Some("HTTP/1.1 400 Bad Request")), None);
        // Proxmox's port alone says nothing about the system.
        assert_eq!(seen(&[8006], None), None);
    }

    #[test]
    fn the_greeting_is_found_and_cleaned() {
        assert_eq!(first_line(b"SSH-2.0-OpenSSH_9.2p1 Debian-2+deb12u3\r\n").as_deref(), Some(DEBIAN));
        // Lines before it are allowed; control characters are dropped.
        assert_eq!(first_line(b"Welcome\r\nSSH-2.0-Open\x1b[2JSSH_9.5\r\n").as_deref(), Some("SSH-2.0-Open[2JSSH_9.5"));
        assert_eq!(first_line(b"no greeting here\n"), None);
        assert_eq!(first_line(b""), None);
        let long = format!("SSH-2.0-{}\r\n", "x".repeat(1000));
        assert_eq!(first_line(long.as_bytes()).unwrap().chars().count(), GREETING_MAX);
    }

    #[tokio::test]
    async fn what_answers_on_a_real_socket() {
        use tokio::io::AsyncWriteExt;
        let ssh = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let ssh_port = ssh.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = ssh.accept().await {
                s.write_all(b"SSH-2.0-OpenSSH_9.2p1 Debian-2+deb12u3\r\n").await.ok();
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
        let other = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let open_port = other.local_addr().unwrap().port();
        // A port that was open a moment ago and is closed now.
        let closed_port = {
            let gone = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
            gone.local_addr().unwrap().port()
        };
        let started = Instant::now();
        let a = answered(IpAddr::V4(Ipv4Addr::LOCALHOST), ssh_port, &[open_port, closed_port]).await;
        assert_eq!(a, Answered { open: vec![open_port], ssh_greeting: Some(DEBIAN.into()) });
        assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());

        // A server that never says anything is given up on in time.
        let quiet = answered(IpAddr::V4(Ipv4Addr::LOCALHOST), open_port, &[]).await;
        assert_eq!(quiet, Answered::default());
        drop(other);

        // One that sends another line first, and the greeting a moment later.
        let chatty = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let chatty_port = chatty.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = chatty.accept().await {
                s.write_all(b"Authorised users only\r\n").await.ok();
                tokio::time::sleep(Duration::from_millis(100)).await;
                s.write_all(b"SSH-2.0-OpenSSH_9.2p1 Debian-2+deb12u3\r\n").await.ok();
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
        let a = answered(IpAddr::V4(Ipv4Addr::LOCALHOST), chatty_port, &[]).await;
        assert_eq!(a.ssh_greeting.as_deref(), Some(DEBIAN));
    }

    #[test]
    fn the_greeting_is_waited_for_until_its_line_is_whole() {
        assert!(!has_greeting(b""));
        assert!(!has_greeting(b"Welcome\r\n"));
        assert!(!has_greeting(b"Welcome\r\nSSH-2.0-Open"), "not finished yet");
        assert!(has_greeting(b"Welcome\r\nSSH-2.0-OpenSSH_9.5\r\n"));
        assert!(has_greeting(b"SSH-2.0-OpenSSH_9.5\n"));
    }
}
