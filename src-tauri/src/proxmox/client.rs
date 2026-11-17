//! Calls to Proxmox VE's HTTPS API (normally port 8006), signed with an API
//! token. One connection per call: the screen asks every few seconds, which
//! is far too seldom for keeping connections open to be worth it.
//!
//! The certificate is checked against the one the owner approved (see
//! pin.rs) before anything is sent, so the token never goes to a server the
//! owner hasn't approved. Text from the server is treated as untrusted: it is
//! cut short and stripped of control characters before it is shown.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HOST};
use hyper::{Method, Request, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use serde_json::Value;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use super::pin::PinnedCert;

/// Where Proxmox VE's web interface and API listen unless changed.
pub const DEFAULT_PORT: u16 = 8006;
const CONNECT_TIME: Duration = Duration::from_secs(5);
/// For the secure handshake, and again for the whole answer.
const ANSWER_TIME: Duration = Duration::from_secs(20);
/// Far more than any answer home-cc asks for.
const MAX_BODY: usize = 16 * 1024 * 1024;
/// Longest text from the server that is passed on to a screen.
const MAX_TEXT: usize = 600;

/// An API token: its ID, such as "root@pam!homecc", and its secret.
#[derive(Clone)]
pub struct Token {
    pub id: String,
    pub secret: String,
}

// Written by hand so the secret can't end up in a log by accident.
impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token").field("id", &self.id).field("secret", &"(hidden)").finish()
    }
}

/// One Proxmox host, ready to be called.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    /// The certificate fingerprint the owner approved, if any.
    pub pinned: Option<String>,
    pub token: Token,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// The host showed a certificate, and none has been approved yet.
    /// Nothing was sent.
    NewCert { offered: String },
    /// The host showed a certificate other than the approved one. Nothing was sent.
    CertChanged { expected: String, offered: String },
    /// No connection at all, or no secure one.
    Unreachable(String),
    /// 401: the token wasn't accepted. Holds Proxmox's own reason.
    Auth(String),
    /// 403: the token lacks a permission. Holds Proxmox's own reason, such as
    /// "Permission check failed (/vms/100, VM.PowerMgmt)".
    Permission(String),
    Other(String),
}

/// Percent-encodes everything but letters, digits and `-._~`, for a part of
/// a path or a form value.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn form_body(form: &[(&str, &str)]) -> String {
    form.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

/// Untrusted text from the server, made safe to show: line breaks are kept,
/// other control characters become spaces, and long text is cut short.
pub fn clean(text: &str) -> String {
    let text: String = text
        .chars()
        .map(|c| if c == '\n' { '\n' } else if c.is_control() { ' ' } else { c })
        .collect();
    let text = text.trim();
    match text.char_indices().nth(MAX_TEXT) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

/// What a refused call says: Proxmox puts its message in the status line,
/// and details of a bad value in the body as {"errors": {"name": "why"}}.
fn refusal(status: StatusCode, reason: Option<&str>, body: &[u8]) -> String {
    let mut text = reason
        .filter(|r| !r.trim().is_empty())
        .map(str::to_string)
        .or_else(|| status.canonical_reason().map(str::to_string))
        .unwrap_or_else(|| "No reason given".into());
    if let Ok(json) = serde_json::from_slice::<Value>(body) {
        if let Some(errors) = json.get("errors").and_then(Value::as_object) {
            for (name, why) in errors {
                let why = why.as_str().map(str::to_string).unwrap_or_else(|| why.to_string());
                text.push_str(&format!("\n{name}: {}", why.trim()));
            }
        } else if let Some(message) = json.get("message").and_then(Value::as_str) {
            text.push_str(&format!("\n{}", message.trim()));
        }
    }
    clean(&format!("{text} (HTTP {})", status.as_u16()))
}

fn host_header(host: &str, port: u16) -> String {
    if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") }
}

/// Calls the API at `path` (after "/api2/json") and returns its "data".
/// `form` is sent as the body of a POST.
pub async fn call(ep: &Endpoint, method: Method, path: &str, form: &[(&str, &str)]) -> Result<Value, ApiError> {
    let place = format!("{} on port {}", ep.host, ep.port);
    let verifier = PinnedCert::new(ep.pinned.clone());
    let config = verifier
        .client_config()
        .map_err(|e| ApiError::Other(format!("Couldn't set up a secure connection: {e}")))?;
    let name = ServerName::try_from(ep.host.clone())
        .map_err(|_| ApiError::Other(format!("\"{}\" isn't an IP address or host name home-cc can use.", ep.host)))?;

    let tcp = match timeout(CONNECT_TIME, TcpStream::connect((ep.host.as_str(), ep.port))).await {
        Err(_) => {
            return Err(ApiError::Unreachable(format!(
                "Nothing answered at {place} within {} seconds. Check that the Proxmox PC is on and that the address and port are right.",
                CONNECT_TIME.as_secs()
            )));
        }
        Ok(Err(e)) => {
            let why = match e.kind() {
                std::io::ErrorKind::ConnectionRefused => {
                    "the connection was refused. The PC is there, but nothing is listening on that port: check the port (Proxmox uses 8006) and that this is the Proxmox PC.".to_string()
                }
                _ => format!("{e}. Check that the Proxmox PC is on and that the address is right."),
            };
            return Err(ApiError::Unreachable(format!("Couldn't connect to {place}: {why}")));
        }
        Ok(Ok(tcp)) => tcp,
    };
    tcp.set_nodelay(true).ok();

    let tls = match timeout(ANSWER_TIME, TlsConnector::from(Arc::new(config)).connect(name, tcp)).await {
        Err(_) => {
            return Err(ApiError::Unreachable(format!(
                "{place} took too long to set up a secure connection. Is this the Proxmox web interface's port?"
            )));
        }
        Ok(Err(e)) => {
            if verifier.refused() {
                let offered = verifier.seen().unwrap_or_default();
                return Err(match &ep.pinned {
                    None => ApiError::NewCert { offered },
                    Some(expected) => ApiError::CertChanged { expected: expected.clone(), offered },
                });
            }
            return Err(ApiError::Unreachable(format!(
                "Couldn't set up a secure connection with {place}: {e}. Check that this is the Proxmox PC and its web interface's port (normally 8006)."
            )));
        }
        Ok(Ok(tls)) => tls,
    };

    let mut request = Request::builder()
        .method(method)
        .uri(format!("/api2/json{path}"))
        .header(HOST, host_header(&ep.host, ep.port))
        .header(AUTHORIZATION, format!("PVEAPIToken={}={}", ep.token.id, ep.token.secret))
        .header(ACCEPT, "application/json");
    let body = if form.is_empty() {
        Bytes::new()
    } else {
        request = request.header(CONTENT_TYPE, "application/x-www-form-urlencoded");
        Bytes::from(form_body(form))
    };
    // The error doesn't include the header's value, so the secret stays out of it.
    let request = request.body(Full::new(body)).map_err(|_| {
        ApiError::Other(
            "The token ID or secret holds characters that can't be sent. Copy them again from Proxmox, without spaces or line breaks.".into(),
        )
    })?;

    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .map_err(|e| ApiError::Unreachable(format!("The connection to {place} failed: {e}")))?;
    let driver = tokio::spawn(async move {
        connection.await.ok();
    });
    let exchange = async {
        let response = sender.send_request(request).await?;
        let status = response.status();
        let reason = response
            .extensions()
            .get::<hyper::ext::ReasonPhrase>()
            .map(|r| String::from_utf8_lossy(r.as_bytes()).into_owned());
        let body = Limited::new(response.into_body(), MAX_BODY).collect().await?.to_bytes();
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((status, reason, body))
    };
    let answer = timeout(ANSWER_TIME, exchange).await;
    driver.abort();
    let (status, reason, body) = match answer {
        Err(_) => return Err(ApiError::Unreachable(format!("{place} didn't answer within {} seconds.", ANSWER_TIME.as_secs()))),
        Ok(Err(e)) => return Err(ApiError::Unreachable(format!("The connection to {place} failed: {e}"))),
        Ok(Ok(answer)) => answer,
    };

    match status {
        s if s.is_success() => {
            let json: Value = serde_json::from_slice(&body).map_err(|_| {
                ApiError::Other(format!("{place} answered with something that isn't Proxmox's API. Is this the Proxmox web interface's port?"))
            })?;
            Ok(json.get("data").cloned().unwrap_or(Value::Null))
        }
        StatusCode::UNAUTHORIZED => Err(ApiError::Auth(refusal(status, reason.as_deref(), &body))),
        StatusCode::FORBIDDEN => Err(ApiError::Permission(refusal(status, reason.as_deref(), &body))),
        _ => Err(ApiError::Other(refusal(status, reason.as_deref(), &body))),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::proxmox::pin::fingerprint;
    use rustls::ServerConfig;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use tokio_rustls::TlsAcceptor;

    pub const CERT_A: &[u8] = include_bytes!("testdata/cert_a.pem");
    pub const KEY_A: &[u8] = include_bytes!("testdata/key_a.pem");
    pub const CERT_B: &[u8] = include_bytes!("testdata/cert_b.pem");

    pub fn print_of(pem: &[u8]) -> String {
        fingerprint(CertificateDer::from_pem_slice(pem).unwrap().as_ref())
    }

    /// A one-call HTTPS server on 127.0.0.1 with certificate A. It answers
    /// with `response` and hands back the request it read, or None if the
    /// client gave up during the handshake (so it sent nothing).
    pub async fn serve_once(response: String) -> (u16, JoinHandle<Option<String>>) {
        let config = ServerConfig::builder_with_provider(super::super::pin::provider())
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from_pem_slice(CERT_A).unwrap()],
                PrivateKeyDer::from_pem_slice(KEY_A).unwrap(),
            )
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.ok()?;
            let mut tls = TlsAcceptor::from(Arc::new(config)).accept(tcp).await.ok()?;
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            // Headers, then as much body as they announce.
            loop {
                let n = tls.read(&mut buf).await.ok()?;
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                        .unwrap_or(0);
                    if got.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            tls.write_all(response.as_bytes()).await.ok()?;
            tls.shutdown().await.ok();
            Some(String::from_utf8_lossy(&got).into_owned())
        });
        (port, task)
    }

    pub fn reply(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json;charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    pub fn endpoint(port: u16, pinned: Option<String>) -> Endpoint {
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            pinned,
            token: Token { id: "root@pam!homecc".into(), secret: "5f2c-secret".into() },
        }
    }

    #[test]
    fn the_secret_stays_out_of_debug_output() {
        let text = format!("{:?}", endpoint(8006, None));
        assert!(text.contains("root@pam!homecc"));
        assert!(!text.contains("5f2c-secret"));
    }

    #[test]
    fn encodes_paths_and_forms() {
        assert_eq!(encode("pve-1.lan"), "pve-1.lan");
        assert_eq!(encode("UPID:pve:00:a b/c"), "UPID%3Apve%3A00%3Aa%20b%2Fc");
        assert_eq!(form_body(&[("snapname", "before_update"), ("description", "a&b=ü")]), "snapname=before_update&description=a%26b%3D%C3%BC");
    }

    #[test]
    fn server_text_is_cleaned() {
        assert_eq!(clean("  bad\u{1b}[31m thing\r\n"), "bad [31m thing");
        assert_eq!(clean(&"x".repeat(2000)).chars().count(), MAX_TEXT + 1);
    }

    #[tokio::test]
    async fn nothing_is_sent_before_the_certificate_is_approved() {
        let (port, server) = serve_once(reply("200 OK", r#"{"data":[]}"#)).await;
        let err = call(&endpoint(port, None), Method::GET, "/version", &[]).await.unwrap_err();
        assert_eq!(err, ApiError::NewCert { offered: print_of(CERT_A) });
        assert_eq!(server.await.unwrap(), None, "the token must not reach a server nobody approved");
    }

    #[tokio::test]
    async fn a_changed_certificate_stops_the_call() {
        let (port, server) = serve_once(reply("200 OK", r#"{"data":[]}"#)).await;
        let err = call(&endpoint(port, Some(print_of(CERT_B))), Method::GET, "/version", &[]).await.unwrap_err();
        assert_eq!(err, ApiError::CertChanged { expected: print_of(CERT_B), offered: print_of(CERT_A) });
        assert_eq!(server.await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_approved_certificate_gets_the_signed_request() {
        let (port, server) = serve_once(reply("200 OK", r#"{"data":{"version":"8.2.4","release":"8.2"}}"#)).await;
        let data = call(&endpoint(port, Some(print_of(CERT_A))), Method::GET, "/version", &[]).await.unwrap();
        assert_eq!(data["version"], "8.2.4");
        let request = server.await.unwrap().expect("the request arrives");
        assert!(request.starts_with("GET /api2/json/version HTTP/1.1\r\n"), "{request}");
        assert!(request.to_ascii_lowercase().contains("authorization: pveapitoken=root@pam!homecc=5f2c-secret\r\n"), "{request}");
    }

    #[tokio::test]
    async fn a_form_is_posted_url_encoded() {
        let (port, server) = serve_once(reply("200 OK", r#"{"data":"UPID:pve:1:2:3:qmsnapshot:100:root@pam!homecc:"}"#)).await;
        let data = call(
            &endpoint(port, Some(print_of(CERT_A))),
            Method::POST,
            "/nodes/pve/qemu/100/snapshot",
            &[("snapname", "before_update"), ("description", "new kernel")],
        )
        .await
        .unwrap();
        assert!(data.as_str().unwrap().starts_with("UPID:"));
        let request = server.await.unwrap().unwrap();
        assert!(request.starts_with("POST /api2/json/nodes/pve/qemu/100/snapshot HTTP/1.1\r\n"));
        assert!(request.to_ascii_lowercase().contains("content-type: application/x-www-form-urlencoded"));
        assert!(request.ends_with("\r\n\r\nsnapname=before_update&description=new%20kernel"), "{request}");
    }

    #[tokio::test]
    async fn refusals_carry_proxmoxs_reason() {
        let pinned = Some(print_of(CERT_A));
        let (port, _) = serve_once(reply("403 Permission check failed (/vms/100, VM.PowerMgmt)", r#"{"data":null}"#)).await;
        let err = call(&endpoint(port, pinned.clone()), Method::POST, "/nodes/pve/qemu/100/status/start", &[]).await.unwrap_err();
        assert_eq!(err, ApiError::Permission("Permission check failed (/vms/100, VM.PowerMgmt) (HTTP 403)".into()));

        let (port, _) = serve_once(reply("401 No ticket", r#"{"data":null}"#)).await;
        let err = call(&endpoint(port, pinned.clone()), Method::GET, "/cluster/resources", &[]).await.unwrap_err();
        assert_eq!(err, ApiError::Auth("No ticket (HTTP 401)".into()));

        let body = r#"{"errors":{"snapname":"invalid format - invalid configuration ID '1x'\n"},"data":null}"#;
        let (port, _) = serve_once(reply("400 Parameter verification failed.", body)).await;
        let err = call(&endpoint(port, pinned), Method::POST, "/nodes/pve/qemu/100/snapshot", &[("snapname", "1x")]).await.unwrap_err();
        assert_eq!(
            err,
            ApiError::Other("Parameter verification failed.\nsnapname: invalid format - invalid configuration ID '1x' (HTTP 400)".into())
        );
    }

    #[tokio::test]
    async fn nothing_listening_is_unreachable() {
        // Bind and drop, so the port is very likely free.
        let port = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let err = call(&endpoint(port, None), Method::GET, "/version", &[]).await.unwrap_err();
        assert!(matches!(&err, ApiError::Unreachable(m) if m.contains("refused")), "{err:?}");
    }
}
