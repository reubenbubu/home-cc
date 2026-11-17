//! Diagnostic for the SSH and machine-stats code: logs in to one machine the way the
//! app does and prints what the app would see. Nothing is stored.
//!
//! ```text
//! cargo run --example ssh_stats -- --host 192.168.68.20 --user jim [options]
//!
//!   --port N             SSH port (default 22)
//!   --os KIND            linux (default), windows, proxmox or other
//!   --key FILE           private key file; without it the usual ones in ~/.ssh are tried
//!   --known-alg A --known-fp F
//!                        the identity stored for the machine ("ssh-ed25519", "SHA256:...").
//!                        Without them this is a first connection: the identity the machine
//!                        shows is printed as `first use: ...` before anything is offered
//!                        to it, a key or the agent may log in, and the password is not
//!                        sent (the run ends with `error[newHost]` if only it could get in)
//!   --again SECS         read the stats a second time after SECS seconds, checking the
//!                        identity from the first reading (this one has a CPU percentage)
//!   --exec CMD           run CMD instead of reading stats and print what it printed
//!   --stdin-file FILE    with --exec: send this file to the command's input
//!   --timeout SECS       with --exec: give up after this long (default 10)
//! ```
//! The password and the key's passphrase are read from the environment, so they don't
//! show up in the process list: SSH_STATS_PASSWORD and SSH_STATS_PASSPHRASE.
//!
//! Exit code 0 if everything worked, 1 if not. Failures print as `error[kind]: message`,
//! where kind is newHost, hostKeyChanged, auth, unreachable or other, as on the screens.
//! newHost and hostKeyChanged also print the identity the machine showed (`offered:`).

use home_cc_lib::inventory::Os;
use home_cc_lib::ssh::{self, HostKey, Login, SshError};
use home_cc_lib::stats::{self, Collected};
use std::process::ExitCode;
use std::time::{Duration, Instant};

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn secret(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn fail(e: &SshError, took: Duration) -> ExitCode {
    let kind = match e {
        SshError::NewHost(_) => "newHost",
        SshError::HostKeyChanged { .. } => "hostKeyChanged",
        SshError::AuthFailed(_) => "auth",
        SshError::Unreachable(_) => "unreachable",
        SshError::Other(_) => "other",
    };
    println!("error[{kind}]: {e}");
    match e {
        SshError::NewHost(offered) => println!("  offered: {} {}", offered.algorithm, offered.fingerprint),
        SshError::HostKeyChanged { expected, got } => {
            println!("  stored: {} {}", expected.algorithm, expected.fingerprint);
            println!("  offered: {} {}", got.algorithm, got.fingerprint);
        }
        _ => {}
    }
    println!("  after {took:.1?}");
    ExitCode::FAILURE
}

/// What the app does on a first connection is store the identity. Here it is printed,
/// straight away, so the output shows it came before the login.
fn first_use(key: &HostKey) {
    println!("first use: {} {}", key.algorithm, key.fingerprint);
}

fn show(reading: &Collected, took: Duration) {
    println!("stats read in {took:.1?}");
    println!("  identity: {} {}", reading.host_key.algorithm, reading.host_key.fingerprint);
    println!("  cpu counters for next time: {:?}", reading.cpu_ticks);
    match serde_json::to_string_pretty(&reading.stats) {
        Ok(json) => println!("{json}"),
        Err(e) => println!("  (could not print the stats: {e})"),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let (Some(host), Some(user)) = (arg("--host"), arg("--user")) else {
        eprintln!(
            "usage: ssh_stats --host HOST --user USER [options]  (see the top of examples/ssh_stats.rs)"
        );
        return ExitCode::FAILURE;
    };
    let os = match arg("--os").as_deref() {
        None | Some("linux") => Os::Linux,
        Some("windows") => Os::Windows,
        Some("proxmox") => Os::Proxmox,
        Some("other") => Os::Other,
        Some(other) => {
            eprintln!("unknown --os {other}: use linux, windows, proxmox or other");
            return ExitCode::FAILURE;
        }
    };
    let known_key = match (arg("--known-alg"), arg("--known-fp")) {
        (Some(algorithm), Some(fingerprint)) => Some(HostKey { algorithm, fingerprint }),
        (None, None) => None,
        _ => {
            eprintln!("--known-alg and --known-fp go together");
            return ExitCode::FAILURE;
        }
    };
    let mut login = Login {
        host,
        port: arg("--port").and_then(|p| p.parse().ok()).unwrap_or(22),
        user,
        known_key,
        key_path: arg("--key").map(Into::into),
        key_passphrase: secret("SSH_STATS_PASSPHRASE"),
        password: secret("SSH_STATS_PASSWORD"),
    };

    if let Some(command) = arg("--exec") {
        let stdin = match arg("--stdin-file").map(std::fs::read) {
            Some(Ok(bytes)) => Some(bytes),
            Some(Err(e)) => {
                eprintln!("could not read the --stdin-file: {e}");
                return ExitCode::FAILURE;
            }
            None => None,
        };
        let limit = Duration::from_secs(arg("--timeout").and_then(|t| t.parse().ok()).unwrap_or(10));
        let started = Instant::now();
        let session = match ssh::connect(&login, &first_use).await {
            Ok(session) => session,
            Err(e) => return fail(&e, started.elapsed()),
        };
        println!("logged in after {:.1?}", started.elapsed());
        println!("  identity: {} {}", session.host_key().algorithm, session.host_key().fingerprint);
        println!("  server: {}", session.server_version());
        let started = Instant::now();
        let result = session.exec(&command, stdin.as_deref(), limit).await;
        let took = started.elapsed();
        // A second command shows the connection is still good after the first.
        let again = session.exec("echo still-connected", None, Duration::from_secs(5)).await;
        println!("  second command: {}", if again.is_ok() { "ran" } else { "failed" });
        session.close().await;
        return match result {
            Ok(output) => {
                println!("command ended after {took:.1?}: exit status {:?}", output.exit_status);
                if output.truncated {
                    println!("  (output was cut at the size limit)");
                }
                println!(
                    "--- stdout ({} bytes)\n{}",
                    output.stdout.len(),
                    String::from_utf8_lossy(&output.stdout)
                );
                println!(
                    "--- stderr ({} bytes)\n{}",
                    output.stderr.len(),
                    String::from_utf8_lossy(&output.stderr)
                );
                ExitCode::SUCCESS
            }
            Err(e) => fail(&e, took),
        };
    }

    let started = Instant::now();
    let first = match stats::collect(&login, os, None, &first_use).await {
        Ok(reading) => reading,
        Err(e) => return fail(&e, started.elapsed()),
    };
    show(&first, started.elapsed());

    if let Some(wait) = arg("--again").and_then(|s| s.parse().ok()) {
        tokio::time::sleep(Duration::from_secs(wait)).await;
        // What the app does: the identity from the first reading is now the stored one.
        login.known_key = Some(first.host_key.clone());
        let started = Instant::now();
        match stats::collect(&login, os, first.cpu_ticks, &first_use).await {
            Ok(reading) => show(&reading, started.elapsed()),
            Err(e) => return fail(&e, started.elapsed()),
        }
    }
    ExitCode::SUCCESS
}
