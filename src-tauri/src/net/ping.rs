//! One ping to one address, without administrator rights: an ICMP datagram
//! socket on Linux, the call `ping.exe` itself uses on Windows.

use ping_async::{IcmpEchoRequestor, IcmpEchoStatus};
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PingOutcome {
    /// The device answered, after this long.
    Up(Duration),
    /// No answer in time.
    Timeout,
    /// The system or a router said the address can't be reached.
    Unreachable,
    /// Anything else Windows reports.
    Unknown,
}

/// Send one ping and wait for the answer. Must run inside the tokio runtime.
///
/// Don't drop this future while it is waiting (so no `select!` or timeout
/// around it): on Windows that closes the system handle with a request still
/// pending, which is untested ground. Let it finish; it never takes much
/// longer than `timeout`.
pub async fn ping_once(ip: Ipv4Addr, timeout: Duration) -> io::Result<PingOutcome> {
    #[cfg(windows)]
    if use_plain_windows_ping() {
        return tokio::task::spawn_blocking(move || super::winping::ping_blocking(ip, timeout))
            .await
            .map_err(io::Error::other)?;
    }
    let requestor = IcmpEchoRequestor::new(IpAddr::V4(ip), None, None, Some(timeout))?;
    let started = Instant::now();
    // The crate applies the timeout itself.
    let reply = requestor.send().await?;
    Ok(match reply.status() {
        IcmpEchoStatus::Success => {
            // Windows only reports whole milliseconds, so a reply from the
            // same network often reads 0. Use our own stopwatch then.
            let rtt = reply.round_trip_time();
            PingOutcome::Up(if rtt.is_zero() { started.elapsed().min(Duration::from_millis(1)) } else { rtt })
        }
        IcmpEchoStatus::TimedOut => PingOutcome::Timeout,
        IcmpEchoStatus::Unreachable => PingOutcome::Unreachable,
        IcmpEchoStatus::Unknown => PingOutcome::Unknown,
    })
}

/// Setting the environment variable HOME_CC_PING=plain before starting the
/// app switches Windows to the simpler ping call in `winping.rs`. It is there
/// in case the usual one misbehaves on some PC.
#[cfg(windows)]
fn use_plain_windows_ping() -> bool {
    static PLAIN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PLAIN.get_or_init(|| std::env::var("HOME_CC_PING").is_ok_and(|v| v.eq_ignore_ascii_case("plain")))
}

/// Check that this PC lets the app ping at all, without sending anything.
/// Only Linux can refuse. Must run inside the tokio runtime.
pub fn allowed() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    drop(IcmpEchoRequestor::new(IpAddr::V4(Ipv4Addr::LOCALHOST), None, None, None)?);
    Ok(())
}

/// What to tell the owner when `allowed` fails, for a scan and a status
/// check alike: what is wrong, and the one thing to do about it. The setting
/// goes into a file so that it is still there after a restart; `sysctl -w`
/// alone would be lost.
pub fn not_allowed_message(error: &io::Error) -> String {
    if error.kind() == io::ErrorKind::PermissionDenied {
        "This computer does not let apps send pings, so home-cc can't tell which devices are on. To allow \
         it, now and after every restart, run this once in a terminal: \
         echo \"net.ipv4.ping_group_range = 0 2147483647\" | sudo tee /etc/sysctl.d/99-ping.conf \
         && sudo sysctl -p /etc/sysctl.d/99-ping.conf"
            .into()
    } else {
        format!("Pings can't be sent from this computer ({error}).")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_says_what_to_do_and_how_to_make_it_last() {
        let message = not_allowed_message(&io::Error::from(io::ErrorKind::PermissionDenied));
        assert!(message.starts_with("This computer does not let apps send pings"), "{message}");
        assert!(message.contains("net.ipv4.ping_group_range = 0 2147483647"), "{message}");
        // In a file, so a restart doesn't undo it.
        assert!(message.contains("/etc/sysctl.d/"), "{message}");
        // The same text serves the status check, where "scan again" would be wrong.
        assert!(!message.to_lowercase().contains("scan"), "{message}");
        // Nothing a non-specialist would have to look up.
        assert!(!message.contains("ICMP") && !message.contains("errno"), "{message}");

        let other = not_allowed_message(&io::Error::other("no buffer space"));
        assert_eq!(other, "Pings can't be sent from this computer (no buffer space).");
    }
}
