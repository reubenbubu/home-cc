//! Logging in to Pi-hole v6 and keeping the session.
//!
//! POST /api/auth with the password (the web interface password, or better an
//! app password) gives a session ID ("sid"), sent with every later call in the
//! X-FTL-SID header. Pi-hole only allows a few sessions at once (16 unless
//! changed), and every login takes one, so home-cc keeps one session per
//! Pi-hole and uses it for as long as Pi-hole accepts it: a session lasts a
//! while without use (30 minutes unless changed) and each use extends it.
//! When a call is turned away with 401 the session is dropped, home-cc logs
//! in once more and tries the call again. DELETE /api/auth logs out, which
//! home-cc does when the settings change, the Pi-hole is removed, or the app
//! closes.
//!
//! Written against a `Transport` so the whole flow can be tried without a
//! network (see the tests); client.rs is the real one.

use std::fmt;
use std::future::Future;
use std::time::{Duration, Instant};

use hyper::Method;
use serde_json::{Value, json};

use super::client::{self, ApiError, Endpoint, Reply};
use super::model::{self, ApiFault};

/// A Pi-hole's password. Never printed.
#[derive(Clone)]
pub struct Password(pub String);

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(hidden)")
    }
}

/// Something that sends one request to one Pi-hole.
pub trait Transport {
    /// Where it goes: a session made at one place is never sent to another.
    fn key(&self) -> String;
    fn send(&self, method: Method, path: &str, sid: Option<&str>, body: Option<&Value>) -> impl Future<Output = Result<Reply, ApiError>> + Send;
}

impl Transport for Endpoint {
    fn key(&self) -> String {
        Endpoint::key(self)
    }

    async fn send(&self, method: Method, path: &str, sid: Option<&str>, body: Option<&Value>) -> Result<Reply, ApiError> {
        client::send(self, method, path, sid, body).await
    }
}

/// A session Pi-hole gave home-cc.
#[derive(Clone, PartialEq)]
pub struct Session {
    /// Transport::key of where it was made.
    pub place: String,
    /// None when the Pi-hole has no password, so calls need no session.
    pub sid: Option<String>,
    /// How long Pi-hole keeps it without use.
    pub lifetime: Duration,
    /// When home-cc stops using it and logs in again, a little before Pi-hole would drop it.
    pub until: Instant,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session").field("place", &self.place).field("has_sid", &self.sid.is_some()).finish_non_exhaustive()
    }
}

/// Used when Pi-hole doesn't say how long a session lasts. Its own default is 1800 seconds.
const DEFAULT_LIFETIME: Duration = Duration::from_secs(1800);
/// A session is given up this long before Pi-hole would end it, so a call never races its end.
const MARGIN: Duration = Duration::from_secs(60);

impl Session {
    fn fresh(&self, place: &str) -> bool {
        self.place == place && Instant::now() < self.until
    }

    fn used(&mut self) {
        self.until = Instant::now() + self.lifetime.saturating_sub(MARGIN);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PiholeErrorKind {
    /// No password stored.
    NoPassword,
    /// First contact: the owner must approve the certificate. Nothing was sent.
    NewCert,
    /// The certificate isn't the approved one. Nothing was sent.
    CertChanged,
    /// The password wasn't accepted.
    Auth,
    /// Every API session the Pi-hole allows is in use.
    TooManySessions,
    /// Pi-hole is holding back logins after too many tries.
    RateLimited,
    /// An older Pi-hole (v5), whose API home-cc doesn't speak.
    V5,
    Unreachable,
    Other,
}

/// Why a call failed, worded for the owner.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub kind: PiholeErrorKind,
    pub message: String,
    /// With NewCert and CertChanged: the certificate the Pi-hole showed.
    pub offered: Option<String>,
    /// With CertChanged: the one approved before.
    pub expected: Option<String>,
}

impl Failure {
    pub fn new(kind: PiholeErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), offered: None, expected: None }
    }
}

const APP_PASSWORD_HOWTO: &str = "In the Pi-hole's web interface open Settings › Web interface / API, switch the page to Expert (top right), \
     and use \"Configure app password\". Copy the app password it shows (it is shown only once), press \"Enable new app password\", and paste it here.";

/// Turns a failed connection into words that say what to do.
pub fn explain(e: ApiError) -> Failure {
    use PiholeErrorKind as K;
    match e {
        ApiError::NewCert { offered } => Failure {
            kind: K::NewCert,
            message: "First connection to this Pi-hole over HTTPS. Check its certificate before home-cc sends it the password.".into(),
            offered: Some(offered),
            expected: None,
        },
        ApiError::CertChanged { expected, offered } => Failure {
            kind: K::CertChanged,
            message: "This Pi-hole shows a different certificate from the one you approved, so home-cc sent it nothing. Check the new one before you approve it.".into(),
            offered: Some(offered),
            expected: Some(expected),
        },
        ApiError::Unreachable(m) => Failure::new(K::Unreachable, m),
        ApiError::Other(m) => Failure::new(K::Other, m),
    }
}

/// What a refused call says, from Pi-hole's error object when there is one.
pub fn refusal(reply: &Reply) -> String {
    let text = match model::parse_fault(&reply.body) {
        Some(ApiFault { message, hint: Some(hint), .. }) if !message.is_empty() => format!("{message} ({hint})"),
        Some(ApiFault { message, .. }) if !message.is_empty() => message,
        _ => match reply.status {
            404 => "Not found".into(),
            500..=599 => "Pi-hole had an internal error".into(),
            _ => "No reason given".into(),
        },
    };
    client::clean(&format!("{text} (HTTP {})", reply.status))
}

/// Logs in. Nothing about the password ends up in a message.
pub async fn login<T: Transport>(t: &T, password: &Password) -> Result<Session, Failure> {
    use PiholeErrorKind as K;
    let body = json!({ "password": password.0 });
    let reply = t.send(Method::POST, "/api/auth", None, Some(&body)).await.map_err(explain)?;
    let session = model::parse_session(&reply.body);
    match reply.status {
        200..=299 => {
            let Some(s) = session.filter(|s| s.valid) else {
                return Err(Failure::new(K::Other, "Pi-hole answered the login, but not with a session. Is this Pi-hole v6?"));
            };
            let lifetime = u64::try_from(s.validity).ok().filter(|v| *v > 0).map(Duration::from_secs).unwrap_or(DEFAULT_LIFETIME);
            let mut made = Session { place: t.key(), sid: s.sid, lifetime, until: Instant::now() };
            made.used();
            Ok(made)
        }
        401 => {
            let fault = model::parse_fault(&reply.body);
            if let Some(f) = fault.filter(|f| f.message.contains("2FA")) {
                return Err(Failure::new(
                    K::Auth,
                    format!("Pi-hole asks for a two-factor code ({}). home-cc can't give one: use an app password instead. {APP_PASSWORD_HOWTO}", f.message),
                ));
            }
            let totp = session.map(|s| s.totp).unwrap_or(false);
            let mut message = "Pi-hole didn't accept the password. Check it, or better, use an app password. ".to_string();
            message.push_str(APP_PASSWORD_HOWTO);
            if totp {
                message.push_str(" Two-factor login is on for this Pi-hole, so only an app password works here.");
            }
            Err(Failure::new(K::Auth, message))
        }
        400 => {
            let why = refusal(&reply);
            if why.contains("2FA") {
                return Err(Failure::new(
                    K::Auth,
                    format!("Pi-hole wants a two-factor code with this password ({why}). home-cc can't give one: use an app password instead. {APP_PASSWORD_HOWTO}"),
                ));
            }
            Err(Failure::new(K::Other, format!("Pi-hole refused the login: {why}")))
        }
        429 => {
            let fault = model::parse_fault(&reply.body);
            if fault.as_ref().is_some_and(|f| f.key == "api_seats_exceeded") {
                Err(Failure::new(
                    K::TooManySessions,
                    format!(
                        "Pi-hole has no room for another login: every API session it allows is in use ({}). Sessions end by \
                         themselves after a while without use (30 minutes unless changed). To free them now, open the \
                         Pi-hole's web interface, Settings › Web interface / API, and delete the old sessions in the list there; \
                         or allow more with webserver.api.max_sessions. home-cc keeps one session per Pi-hole and logs out \
                         when it closes.",
                        refusal(&reply)
                    ),
                ))
            } else {
                Err(Failure::new(
                    K::RateLimited,
                    format!(
                        "Pi-hole is holding back logins for a while after too many tries ({}). Wait a minute, check the password, then read it again.",
                        refusal(&reply)
                    ),
                ))
            }
        }
        404 => Err(not_v6(t).await),
        _ => Err(Failure::new(K::Other, format!("Pi-hole refused the login: {}", refusal(&reply)))),
    }
}

/// /api/auth isn't there: tell an older Pi-hole (v5, whose API is /admin/api.php) from something else.
/// Nothing secret is sent.
async fn not_v6<T: Transport>(t: &T) -> Failure {
    if let Ok(r) = t.send(Method::GET, "/admin/api.php?version", None, None).await {
        if r.ok() && r.body.get("version").is_some() {
            return Failure::new(
                PiholeErrorKind::V5,
                "This looks like Pi-hole v5. home-cc only works with Pi-hole v6 (from February 2025), whose API is \
                 different; v5 isn't supported yet. Updating with \"pihole -up\" on the Pi-hole brings it to v6.",
            );
        }
    }
    Failure::new(
        PiholeErrorKind::Other,
        "Something answers at this address and port, but not with Pi-hole's API (/api/auth wasn't found). Check that this is the \
         Pi-hole and its web interface's port, and that it runs Pi-hole v6.",
    )
}

/// Logs out, if there is a session to end. Failures don't matter: the session ends by itself anyway.
pub async fn logout<T: Transport>(t: &T, session: &Session) {
    if session.place != t.key() {
        return;
    }
    if let Some(sid) = &session.sid {
        t.send(Method::DELETE, "/api/auth", Some(sid), None).await.ok();
    }
}

/// Makes one call with the session in `slot`, logging in first when there is none (or it is old, or from
/// another place). A call turned away with 401 drops the session and is tried once more after a new login.
/// Gives the body of a successful answer.
pub async fn call<T: Transport>(
    t: &T,
    slot: &mut Option<Session>,
    password: &Password,
    method: Method,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, Failure> {
    let place = t.key();
    for attempt in 0..2 {
        let session = match slot.take() {
            Some(s) if s.fresh(&place) => s,
            _ => login(t, password).await?,
        };
        let reply = match t.send(method.clone(), path, session.sid.as_deref(), body).await {
            Ok(r) => r,
            Err(e) => {
                // The request didn't get through (timeout, dropped connection), so Pi-hole still holds the
                // session: keep it for the next call rather than logging in again and taking another of
                // Pi-hole's few API seats. A certificate that changed is different: the session isn't sent
                // anywhere until the owner has looked, and an approval may mean another machine answers.
                if !matches!(e, ApiError::NewCert { .. } | ApiError::CertChanged { .. }) {
                    *slot = Some(session);
                }
                return Err(explain(e));
            }
        };
        if reply.ok() {
            let mut session = session;
            session.used();
            *slot = Some(session);
            if !reply.json {
                return Err(Failure::new(
                    PiholeErrorKind::Other,
                    format!("Pi-hole answered {path} with something that isn't JSON. Is this the Pi-hole's web interface port?"),
                ));
            }
            return Ok(reply.body);
        }
        if reply.status == 401 {
            // Pi-hole ended the session (restarted, timed out, or someone deleted it): log in again once.
            if attempt == 0 {
                continue;
            }
            return Err(Failure::new(
                PiholeErrorKind::Auth,
                format!("Pi-hole turned home-cc away right after logging in ({}). Check the password, then read it again.", refusal(&reply)),
            ));
        }
        // The session is still good for other calls.
        *slot = Some(session);
        return Err(Failure::new(PiholeErrorKind::Other, format!("Pi-hole refused {path}: {}", refusal(&reply))));
    }
    unreachable!("the loop returns on its second round")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// A Pi-hole that answers from a script and remembers what it was asked.
    pub struct Fake {
        pub place: String,
        pub replies: Mutex<VecDeque<Result<Reply, ApiError>>>,
        pub asked: Mutex<Vec<(Method, String, Option<String>, Option<Value>)>>,
    }

    pub fn ok(body: Value) -> Result<Reply, ApiError> {
        Ok(Reply { status: 200, body, json: true, location: None })
    }

    pub fn status(code: u16, body: Value) -> Result<Reply, ApiError> {
        Ok(Reply { status: code, body, json: true, location: None })
    }

    pub fn logged_in(sid: &str) -> Result<Reply, ApiError> {
        ok(json!({"session": {"valid": true, "totp": false, "sid": sid, "csrf": "c", "validity": 1800, "message": "password correct"}, "took": 0.01}))
    }

    pub fn unauthorized() -> Result<Reply, ApiError> {
        status(401, json!({"error": {"key": "unauthorized", "message": "Unauthorized", "hint": null}, "took": 0}))
    }

    impl Fake {
        pub fn new(replies: Vec<Result<Reply, ApiError>>) -> Self {
            Self { place: "https://192.168.68.31:443".into(), replies: Mutex::new(replies.into()), asked: Mutex::new(Vec::new()) }
        }

        /// (method, path, sid) of each request, in order.
        pub fn asked(&self) -> Vec<(String, String, Option<String>)> {
            self.asked.lock().unwrap().iter().map(|(m, p, s, _)| (m.to_string(), p.clone(), s.clone())).collect()
        }
    }

    impl Transport for Fake {
        fn key(&self) -> String {
            self.place.clone()
        }

        async fn send(&self, method: Method, path: &str, sid: Option<&str>, body: Option<&Value>) -> Result<Reply, ApiError> {
            self.asked.lock().unwrap().push((method, path.to_string(), sid.map(str::to_string), body.cloned()));
            self.replies.lock().unwrap().pop_front().expect("the test gave an answer for every request")
        }
    }

    fn pw() -> Password {
        Password("correct horse".into())
    }

    fn summary() -> Value {
        json!({"queries": {"total": 10, "blocked": 2}})
    }

    fn get(s: &str) -> (String, String, Option<String>) {
        ("GET".into(), s.into(), Some("sid-1".into()))
    }

    #[tokio::test]
    async fn one_login_serves_many_calls() {
        let fake = Fake::new(vec![logged_in("sid-1"), ok(summary()), ok(json!({"blocking": "enabled", "timer": null}))]);
        let mut slot = None;
        call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap();
        let b = call(&fake, &mut slot, &pw(), Method::GET, "/api/dns/blocking", None).await.unwrap();
        assert_eq!(b["blocking"], "enabled");
        assert_eq!(
            fake.asked(),
            vec![("POST".into(), "/api/auth".into(), None), get("/api/stats/summary"), get("/api/dns/blocking")]
        );
        // The password went in the login's body, and only there.
        let asked = fake.asked.lock().unwrap();
        assert_eq!(asked[0].3, Some(json!({"password": "correct horse"})));
        assert!(asked[1..].iter().all(|a| a.3.is_none()));
        assert_eq!(slot.as_ref().and_then(|s| s.sid.as_deref()), Some("sid-1"));
    }

    #[tokio::test]
    async fn a_session_pi_hole_dropped_is_replaced_once() {
        let fake = Fake::new(vec![unauthorized(), logged_in("sid-2"), ok(summary())]);
        let mut slot = Some(Session {
            place: fake.place.clone(),
            sid: Some("sid-1".into()),
            lifetime: DEFAULT_LIFETIME,
            until: Instant::now() + Duration::from_secs(600),
        });
        call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap();
        assert_eq!(
            fake.asked(),
            vec![
                get("/api/stats/summary"),
                ("POST".into(), "/api/auth".into(), None),
                ("GET".into(), "/api/stats/summary".into(), Some("sid-2".into())),
            ]
        );
        assert_eq!(slot.unwrap().sid.as_deref(), Some("sid-2"));

        // Turned away again straight after a new login: it stops there rather than logging in over and over.
        let fake = Fake::new(vec![logged_in("sid-1"), unauthorized(), logged_in("sid-2"), unauthorized()]);
        let mut slot = None;
        let err = call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap_err();
        assert_eq!(err.kind, PiholeErrorKind::Auth);
        assert_eq!(fake.asked().len(), 4);
        assert!(slot.is_none());
    }

    #[tokio::test]
    async fn an_old_session_or_one_from_elsewhere_is_not_used() {
        let fake = Fake::new(vec![logged_in("sid-new"), ok(summary())]);
        let mut slot = Some(Session { place: fake.place.clone(), sid: Some("sid-old".into()), lifetime: DEFAULT_LIFETIME, until: Instant::now() });
        call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap();
        assert_eq!(fake.asked()[0].1, "/api/auth", "an expired session isn't sent");

        let fake = Fake::new(vec![logged_in("sid-new"), ok(summary())]);
        let mut slot = Some(Session {
            place: "https://192.168.68.99:443".into(),
            sid: Some("sid-elsewhere".into()),
            lifetime: DEFAULT_LIFETIME,
            until: Instant::now() + Duration::from_secs(600),
        });
        call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap();
        assert!(fake.asked().iter().all(|(_, _, sid)| sid.as_deref() != Some("sid-elsewhere")), "a session goes only where it was made");
    }

    #[tokio::test]
    async fn a_pi_hole_without_a_password_needs_no_session() {
        let fake = Fake::new(vec![
            ok(json!({"session": {"valid": true, "totp": false, "sid": null, "validity": -1, "message": "no password set"}})),
            ok(summary()),
            ok(summary()),
        ]);
        let mut slot = None;
        call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap();
        call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap();
        let asked = fake.asked();
        assert_eq!(asked.len(), 3, "logged in once");
        assert_eq!(asked[1].2, None);
    }

    #[tokio::test]
    async fn login_failures_say_what_to_do() {
        let wrong = Fake::new(vec![status(401, json!({"session": {"valid": false, "totp": false, "sid": null, "validity": -1, "message": "password incorrect"}}))]);
        let err = login(&wrong, &pw()).await.unwrap_err();
        assert_eq!(err.kind, PiholeErrorKind::Auth);
        assert!(err.message.contains("didn't accept the password") && err.message.contains("app password"), "{}", err.message);
        assert!(!err.message.contains("correct horse"));

        let two_factor = Fake::new(vec![status(401, json!({"session": {"valid": false, "totp": true, "sid": null, "validity": -1}}))]);
        assert!(login(&two_factor, &pw()).await.unwrap_err().message.contains("Two-factor login is on"));

        let code = Fake::new(vec![status(400, json!({"error": {"key": "bad_request", "message": "No 2FA token found in JSON payload", "hint": null}}))]);
        let err = login(&code, &pw()).await.unwrap_err();
        assert_eq!(err.kind, PiholeErrorKind::Auth);
        assert!(err.message.contains("two-factor"));

        let full = Fake::new(vec![status(
            429,
            json!({"error": {"key": "api_seats_exceeded", "message": "API seats exceeded", "hint": "increase webserver.api.max_sessions"}}),
        )]);
        let err = login(&full, &pw()).await.unwrap_err();
        assert_eq!(err.kind, PiholeErrorKind::TooManySessions);
        assert!(err.message.contains("API seats exceeded (increase webserver.api.max_sessions) (HTTP 429)"), "{}", err.message);

        let slow = Fake::new(vec![status(429, json!({"error": {"key": "rate_limiting", "message": "Rate-limiting login attempts", "hint": null}}))]);
        assert_eq!(login(&slow, &pw()).await.unwrap_err().kind, PiholeErrorKind::RateLimited);

        let down = Fake::new(vec![Err(ApiError::Unreachable("Couldn't connect to 192.168.68.31 on port 443: refused".into()))]);
        let err = login(&down, &pw()).await.unwrap_err();
        assert_eq!((err.kind, err.message.as_str()), (PiholeErrorKind::Unreachable, "Couldn't connect to 192.168.68.31 on port 443: refused"));

        let changed = Fake::new(vec![Err(ApiError::CertChanged { expected: "AA".into(), offered: "BB".into() })]);
        let err = login(&changed, &pw()).await.unwrap_err();
        assert_eq!((err.kind, err.offered.as_deref(), err.expected.as_deref()), (PiholeErrorKind::CertChanged, Some("BB"), Some("AA")));
        assert!(err.message.contains("sent it nothing"));

        let new = Fake::new(vec![Err(ApiError::NewCert { offered: "CC".into() })]);
        let err = login(&new, &pw()).await.unwrap_err();
        assert_eq!((err.kind, err.offered.as_deref()), (PiholeErrorKind::NewCert, Some("CC")));
    }

    #[tokio::test]
    async fn an_older_pi_hole_is_recognised() {
        let v5 = Fake::new(vec![
            Ok(Reply { status: 404, body: Value::Null, json: false, location: None }),
            ok(json!({"version": 3})),
        ]);
        let err = login(&v5, &pw()).await.unwrap_err();
        assert_eq!(err.kind, PiholeErrorKind::V5);
        assert_eq!(v5.asked()[1], ("GET".into(), "/admin/api.php?version".into(), None));

        let other = Fake::new(vec![
            Ok(Reply { status: 404, body: Value::Null, json: false, location: None }),
            Ok(Reply { status: 404, body: Value::Null, json: false, location: None }),
        ]);
        let err = login(&other, &pw()).await.unwrap_err();
        assert_eq!(err.kind, PiholeErrorKind::Other);
        assert!(err.message.contains("not with Pi-hole's API"));
    }

    #[tokio::test]
    async fn other_refusals_keep_the_session() {
        let fake = Fake::new(vec![
            logged_in("sid-1"),
            status(400, json!({"error": {"key": "body_error", "message": "No \"blocking\" boolean in body data", "hint": null}})),
        ]);
        let mut slot = None;
        let err = call(&fake, &mut slot, &pw(), Method::POST, "/api/dns/blocking", Some(&json!({}))).await.unwrap_err();
        assert_eq!(err.kind, PiholeErrorKind::Other);
        assert!(err.message.contains("No \"blocking\" boolean in body data (HTTP 400)"), "{}", err.message);
        assert!(slot.is_some());
    }

    #[tokio::test]
    async fn a_request_that_does_not_get_through_keeps_the_session() {
        // Logged in, then the next request times out: the session Pi-hole still holds is used again
        // instead of a second login taking another API seat.
        let fake = Fake::new(vec![
            logged_in("sid-1"),
            ok(summary()),
            Err(ApiError::Unreachable("No answer from 192.168.68.31 within 15 seconds".into())),
            ok(summary()),
        ]);
        let mut slot = None;
        call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap();
        let err = call(&fake, &mut slot, &pw(), Method::GET, "/api/dns/blocking", None).await.unwrap_err();
        assert_eq!(err.kind, PiholeErrorKind::Unreachable);
        assert_eq!(slot.as_ref().and_then(|s| s.sid.as_deref()), Some("sid-1"));
        call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap();
        assert_eq!(
            fake.asked(),
            vec![("POST".into(), "/api/auth".into(), None), get("/api/stats/summary"), get("/api/dns/blocking"), get("/api/stats/summary")]
        );

        // Straight after a login too: the new session isn't lost.
        let fake = Fake::new(vec![logged_in("sid-1"), Err(ApiError::Other("Connection reset".into())), ok(summary())]);
        let mut slot = None;
        assert_eq!(call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap_err().kind, PiholeErrorKind::Other);
        call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap();
        assert_eq!(fake.asked().iter().filter(|(_, p, _)| p == "/api/auth").count(), 1);

        // A certificate that changed mid-way: the session isn't kept for whoever answers after an approval.
        let fake = Fake::new(vec![logged_in("sid-1"), Err(ApiError::CertChanged { expected: "AA".into(), offered: "BB".into() })]);
        let mut slot = None;
        assert_eq!(call(&fake, &mut slot, &pw(), Method::GET, "/api/stats/summary", None).await.unwrap_err().kind, PiholeErrorKind::CertChanged);
        assert!(slot.is_none());
    }

    #[tokio::test]
    async fn logging_out_sends_the_session_to_where_it_was_made_only() {
        let fake = Fake::new(vec![status(204, Value::Null)]);
        let s = Session { place: fake.place.clone(), sid: Some("sid-1".into()), lifetime: DEFAULT_LIFETIME, until: Instant::now() };
        logout(&fake, &s).await;
        assert_eq!(fake.asked(), vec![("DELETE".into(), "/api/auth".into(), Some("sid-1".into()))]);

        let elsewhere = Fake::new(vec![]);
        let s = Session { place: "http://192.168.68.99:80".into(), ..s };
        logout(&elsewhere, &s).await;
        assert!(elsewhere.asked().is_empty());
    }

    #[test]
    fn the_password_stays_out_of_debug_output() {
        assert_eq!(format!("{:?}", pw()), "Password(hidden)");
        let s = Session { place: "p".into(), sid: Some("sid-secret".into()), lifetime: DEFAULT_LIFETIME, until: Instant::now() };
        assert!(!format!("{s:?}").contains("sid-secret"));
    }
}
