//! SSH connections to the owner's machines. Step 3 uses it to run one command
//! and read the answer; terminals (step 4) will build on the same code.

use std::path::PathBuf;

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
