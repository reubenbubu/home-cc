//! A second way to ping on Windows: one plain blocking call, no callbacks and
//! no third-party code. It is only used when HOME_CC_PING=plain is set (see
//! `ping.rs`). It compiles for Windows but has never been run on a real PC.

use super::ping::PingOutcome;
use std::io;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::GetLastError;
use windows::Win32::NetworkManagement::IpHelper::{
    ICMP_ECHO_REPLY, IP_DEST_HOST_UNREACHABLE, IP_DEST_NET_UNREACHABLE, IP_REQ_TIMED_OUT, IP_SUCCESS,
    IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho,
};

/// Blocks for up to `timeout`: call it through `spawn_blocking`.
pub fn ping_blocking(ip: Ipv4Addr, timeout: Duration) -> io::Result<PingOutcome> {
    // SAFETY: no arguments; the handle is closed below.
    let handle = unsafe { IcmpCreateFile() }?;
    let data = [0u8; 32];
    // Room for one reply, the 32 data bytes and 8 bytes of error detail. u64
    // elements keep it aligned for the reply structure.
    let mut buffer = [0u64; 16];
    let started = Instant::now();
    // SAFETY: both buffers live until the call returns, and the sizes given
    // are their real sizes. The address is wanted in network byte order.
    let replies = unsafe {
        IcmpSendEcho(
            handle,
            u32::from(ip).to_be(),
            data.as_ptr().cast(),
            data.len() as u16,
            None,
            buffer.as_mut_ptr().cast(),
            std::mem::size_of_val(&buffer) as u32,
            u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX),
        )
    };
    let elapsed = started.elapsed();
    // SAFETY: reads the calling thread's last error, set by the call above.
    let error = unsafe { GetLastError() };
    // SAFETY: the handle came from IcmpCreateFile and is not used again.
    let _ = unsafe { IcmpCloseHandle(handle) };
    let status = if replies == 0 {
        error.0
    } else {
        // SAFETY: the call wrote at least one reply at the start of the buffer.
        unsafe { (*buffer.as_ptr().cast::<ICMP_ECHO_REPLY>()).Status }
    };
    Ok(match status {
        // Our own stopwatch is finer than the whole milliseconds in the reply.
        IP_SUCCESS if replies > 0 => PingOutcome::Up(elapsed),
        IP_REQ_TIMED_OUT => PingOutcome::Timeout,
        IP_DEST_HOST_UNREACHABLE | IP_DEST_NET_UNREACHABLE => PingOutcome::Unreachable,
        _ => PingOutcome::Unknown,
    })
}
