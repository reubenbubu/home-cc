//! One call to a Pi-hole's web server: over HTTPS with the certificate the
//! owner approved (the same pinning as Proxmox, see proxmox/pin.rs), or over
//! plain HTTP when the owner chose that. One connection per call: the screen
//! asks every 30 seconds, far too seldom for keeping connections open to be
//! worth it.
//!
//! This layer knows nothing about logging in: it sends the session ID it is
//! given and hands back the status and the body. mod.rs does the rest. Text
//! from the server is treated as untrusted (cut short, control characters
//! removed) before it is shown.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::header::{ACCEPT, CONTENT_TYPE, HOST, LOCATION};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

pub use crate::proxmox::client::clean;
use crate::proxmox::pin::PinnedCert;

/// Pi-hole v6 serves its web interface and API on both by default.
pub const HTTPS_PORT: u16 = 443;
pub const HTTP_PORT: u16 = 80;
const CONNECT_TIME: Duration = Duration::from_secs(5);
/// For the secure handshake, and again for the whole answer.
const ANSWER_TIME: Duration = Duration::from_secs(15);
/// Far more than any answer home-cc asks for.
const MAX_BODY: usize = 8 * 1024 * 1024;
/// The header Pi-hole reads the session ID from (one of the four ways its API description allows).
pub const SID_HEADER: &str = "X-FTL-SID";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Https,
    Http,
}

/// One Pi-hole's web server, ready to be called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub scheme: Scheme,
    /// With HTTPS: the certificate fingerprint the owner approved, if any.
    pub pinned: Option<String>,
}

impl Endpoint {
    /// "192.168.68.31 on port 443", for messages.
    pub fn place(&self) -> String {
        format!("{} on port {}", self.host, self.port)
    }

    /// What a session belongs to: a session made at one place is never sent to another.
    pub fn key(&self) -> String {
        let scheme = match self.scheme {
            Scheme::Https => "https",
            Scheme::Http => "http",
        };
        format!("{scheme}://{}", host_header(&self.host, self.port))
    }
}

/// The value of the Host header (and the middle of a URL): an IPv6 address goes in brackets.
pub fn host_header(host: &str, port: u16) -> String {
    if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") }
}

/// What came back.
#[derive(Clone, PartialEq)]
pub struct Reply {
    pub status: u16,
    /// The body read as JSON; Null when it isn't.
    pub body: Value,
    /// The body was JSON.
    pub json: bool,
    /// Where a redirect pointed, cleaned for showing.
    pub location: Option<String>,
}

// Written by hand: a login answer holds a session ID, which must not end up in a log.
impl fmt::Debug for Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reply").field("status", &self.status).field("json", &self.json).finish_non_exhaustive()
    }
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// The Pi-hole showed a certificate, and none has been approved yet. Nothing was sent.
    NewCert { offered: String },
    /// It showed a certificate other than the approved one. Nothing was sent.
    CertChanged { expected: String, offered: String },
    /// No connection at all, or no secure one.
    Unreachable(String),
    Other(String),
}

/// Sends one request: `path` is the whole path, such as "/api/stats/summary". `sid` goes in the X-FTL-SID
/// header, `body` as JSON.
pub async fn send(ep: &Endpoint, method: Method, path: &str, sid: Option<&str>, body: Option<&Value>) -> Result<Reply, ApiError> {
    let place = ep.place();
    let tcp = match timeout(CONNECT_TIME, TcpStream::connect((ep.host.as_str(), ep.port))).await {
        Err(_) => {
            return Err(ApiError::Unreachable(format!(
                "Nothing answered at {place} within {} seconds. Check that the Pi-hole is on and that the address and port are right.",
                CONNECT_TIME.as_secs()
            )));
        }
        Ok(Err(e)) => {
            let why = match e.kind() {
                std::io::ErrorKind::ConnectionRefused => format!(
                    "the connection was refused. The machine is there, but nothing is listening on that port: Pi-hole uses {HTTPS_PORT} for HTTPS and {HTTP_PORT} for HTTP."
                ),
                _ => format!("{e}. Check that the Pi-hole is on and that the address is right."),
            };
            return Err(ApiError::Unreachable(format!("Couldn't connect to {place}: {why}")));
        }
        Ok(Ok(tcp)) => tcp,
    };
    tcp.set_nodelay(true).ok();
    let request = build(ep, method, path, sid, body)?;
    match ep.scheme {
        Scheme::Http => exchange(tcp, request, &place).await,
        Scheme::Https => {
            let verifier = PinnedCert::new(ep.pinned.clone());
            let config = verifier
                .client_config()
                .map_err(|e| ApiError::Other(format!("Couldn't set up a secure connection: {e}")))?;
            let name = ServerName::try_from(ep.host.clone())
                .map_err(|_| ApiError::Other(format!("\"{}\" isn't an IP address or host name home-cc can use.", ep.host)))?;
            let tls = match timeout(ANSWER_TIME, TlsConnector::from(Arc::new(config)).connect(name, tcp)).await {
                Err(_) => {
                    return Err(ApiError::Unreachable(format!(
                        "{place} took too long to set up a secure connection. If this Pi-hole only offers plain HTTP, choose HTTP in its connection settings."
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
                        "Couldn't set up a secure connection with {place}: {e}. Pi-hole v6 offers HTTPS on port {HTTPS_PORT}; if this one only offers plain HTTP, choose HTTP in its connection settings."
                    )));
                }
                Ok(Ok(tls)) => tls,
            };
            exchange(tls, request, &place).await
        }
    }
}

fn build(ep: &Endpoint, method: Method, path: &str, sid: Option<&str>, body: Option<&Value>) -> Result<Request<Full<Bytes>>, ApiError> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(HOST, host_header(&ep.host, ep.port))
        .header(ACCEPT, "application/json");
    if let Some(sid) = sid {
        request = request.header(SID_HEADER, sid);
    }
    let bytes = match body {
        Some(b) => {
            request = request.header(CONTENT_TYPE, "application/json");
            Bytes::from(b.to_string())
        }
        None => Bytes::new(),
    };
    // The error doesn't include any header's value, so the session ID stays out of it.
    request
        .body(Full::new(bytes))
        .map_err(|_| ApiError::Other("Pi-hole gave a session ID that can't be sent back. Try again; if it keeps happening, restart Pi-hole's FTL service.".into()))
}

async fn exchange<S>(io: S, request: Request<Full<Bytes>>, place: &str) -> Result<Reply, ApiError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
        .await
        .map_err(|e| ApiError::Unreachable(format!("The connection to {place} failed: {e}")))?;
    let driver = tokio::spawn(async move {
        connection.await.ok();
    });
    let talk = async {
        let response = sender.send_request(request).await?;
        let status = response.status().as_u16();
        let location = response.headers().get(LOCATION).and_then(|v| v.to_str().ok()).map(clean);
        let body = Limited::new(response.into_body(), MAX_BODY).collect().await?.to_bytes();
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>((status, location, body))
    };
    let answer = timeout(ANSWER_TIME, talk).await;
    driver.abort();
    let (status, location, body) = match answer {
        Err(_) => return Err(ApiError::Unreachable(format!("{place} didn't answer within {} seconds.", ANSWER_TIME.as_secs()))),
        Ok(Err(e)) => return Err(ApiError::Unreachable(format!("The connection to {place} failed: {e}"))),
        Ok(Ok(answer)) => answer,
    };
    let parsed = serde_json::from_slice::<Value>(&body).ok();
    Ok(Reply { status, json: parsed.is_some(), body: parsed.unwrap_or(Value::Null), location })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxmox::client::tests::{CERT_A, CERT_B, print_of, reply, serve_once};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn https(port: u16, pinned: Option<String>) -> Endpoint {
        Endpoint { host: "127.0.0.1".into(), port, scheme: Scheme::Https, pinned }
    }

    #[test]
    fn places_and_keys() {
        let ep = Endpoint { host: "192.168.68.31".into(), port: 443, scheme: Scheme::Https, pinned: None };
        assert_eq!(ep.place(), "192.168.68.31 on port 443");
        assert_eq!(ep.key(), "https://192.168.68.31:443");
        let ep = Endpoint { host: "fd00::31".into(), port: 80, scheme: Scheme::Http, pinned: None };
        assert_eq!(ep.key(), "http://[fd00::31]:80");
        assert_eq!(host_header("pihole1.lan", 8443), "pihole1.lan:8443");
    }

    #[test]
    fn a_reply_never_shows_its_body_in_debug_output() {
        let r = Reply { status: 200, body: serde_json::json!({"session": {"sid": "secret-sid"}}), json: true, location: None };
        assert!(!format!("{r:?}").contains("secret-sid"));
    }

    #[tokio::test]
    async fn nothing_is_sent_before_the_certificate_is_approved() {
        let (port, server) = serve_once(reply("200 OK", "{}")).await;
        let body = serde_json::json!({"password": "hunter2"});
        let err = send(&https(port, None), Method::POST, "/api/auth", None, Some(&body)).await.unwrap_err();
        assert_eq!(err, ApiError::NewCert { offered: print_of(CERT_A) });
        assert_eq!(server.await.unwrap(), None, "the password must not reach a server nobody approved");
    }

    #[tokio::test]
    async fn a_changed_certificate_stops_the_call() {
        let (port, server) = serve_once(reply("200 OK", "{}")).await;
        let err = send(&https(port, Some(print_of(CERT_B))), Method::GET, "/api/stats/summary", Some("sid1"), None).await.unwrap_err();
        assert_eq!(err, ApiError::CertChanged { expected: print_of(CERT_B), offered: print_of(CERT_A) });
        assert_eq!(server.await.unwrap(), None);
    }

    #[tokio::test]
    async fn the_approved_certificate_gets_the_session_and_the_body() {
        let (port, server) = serve_once(reply("200 OK", r#"{"blocking":"disabled","timer":300,"took":0.001}"#)).await;
        let body = serde_json::json!({"blocking": false, "timer": 300});
        let got = send(&https(port, Some(print_of(CERT_A))), Method::POST, "/api/dns/blocking", Some("abc+/="), Some(&body))
            .await
            .unwrap();
        assert!(got.ok() && got.json);
        assert_eq!(got.body["blocking"], "disabled");
        let request = server.await.unwrap().expect("the request arrives");
        let lower = request.to_ascii_lowercase();
        assert!(request.starts_with("POST /api/dns/blocking HTTP/1.1\r\n"), "{request}");
        assert!(lower.contains("x-ftl-sid: abc+/=\r\n"), "{request}");
        assert!(lower.contains("content-type: application/json\r\n"), "{request}");
        assert!(request.ends_with(r#"{"blocking":false,"timer":300}"#), "{request}");
    }

    /// A one-call plain HTTP server: answers with `response`, hands back the request.
    async fn serve_plain_once(response: String) -> (u16, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut tcp, _) = listener.accept().await.unwrap();
            let mut got = Vec::new();
            let mut buf = [0u8; 4096];
            while !String::from_utf8_lossy(&got).contains("\r\n\r\n") {
                let n = tcp.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
            tcp.write_all(response.as_bytes()).await.unwrap();
            tcp.shutdown().await.ok();
            String::from_utf8_lossy(&got).into_owned()
        });
        (port, task)
    }

    #[tokio::test]
    async fn plain_http_when_chosen() {
        let (port, server) = serve_plain_once(reply("401 Unauthorized", r#"{"error":{"key":"unauthorized","message":"Unauthorized","hint":null},"took":0}"#)).await;
        let ep = Endpoint { host: "127.0.0.1".into(), port, scheme: Scheme::Http, pinned: None };
        let got = send(&ep, Method::GET, "/api/stats/summary", Some("old"), None).await.unwrap();
        assert_eq!((got.status, got.json), (401, true));
        assert_eq!(got.body["error"]["key"], "unauthorized");
        let request = server.await.unwrap();
        assert!(request.starts_with("GET /api/stats/summary HTTP/1.1\r\n"), "{request}");
        assert!(request.to_ascii_lowercase().contains(&format!("host: 127.0.0.1:{port}\r\n")), "{request}");
    }

    #[tokio::test]
    async fn not_json_is_said_so() {
        let (port, _) = serve_plain_once(
            "HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot found".into(),
        )
        .await;
        let ep = Endpoint { host: "127.0.0.1".into(), port, scheme: Scheme::Http, pinned: None };
        let got = send(&ep, Method::GET, "/api/auth", None, None).await.unwrap();
        assert_eq!((got.status, got.json, got.body.clone()), (404, false, Value::Null));
    }

    #[tokio::test]
    async fn nothing_listening_is_unreachable() {
        let port = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
        let err = send(&https(port, None), Method::GET, "/api/auth", None, None).await.unwrap_err();
        assert!(matches!(&err, ApiError::Unreachable(m) if m.contains("refused")), "{err:?}");
    }
}
