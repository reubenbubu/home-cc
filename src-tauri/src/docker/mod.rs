//! Docker (spec 5.5): containers and compose stacks on the owner's Docker hosts,
//! reached over the same SSH logins as the machine stats. Nothing is opened on the
//! network for Docker itself: every step is the docker command line, run on the host.
//!
//! - Which machines are Docker hosts: every machine with an SSH user (not Windows,
//!   not ignored) is looked at once for a docker command, and the answer is stored
//!   (store.rs). A machine without Docker is looked at again after a day, in case it
//!   was installed since. The owner can also mark a machine as a Docker host (shown
//!   even when it can't be reached) or as not one (never looked at again).
//! - One SSH connection per host is kept open while the screen uses it, and closed
//!   after a few quiet minutes. Each command runs on its own channel of it.
//! - parse.rs reads docker's output, scripts.rs builds what is run, logs.rs follows
//!   a container's log.

pub mod commands;
mod logs;
pub mod parse;
pub mod scripts;
mod store;

use crate::error::{Error, Result};
use crate::inventory::{Device, DeviceKind, Inventory, Os, now};
use crate::monitor::{self, Monitor, StatsErrorKind};
use crate::ssh::{self, ExecOutput, HostKey, Session, SshError};
use parse::{CleanupPreview, Compose, Container, ContainerStats, Probe, Refusal};
use scripts::{ContainerAction, StackAction, StackPlace};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;

pub use store::{HostMode, init};

pub const EVENT_LOGS: &str = "docker-logs";
pub const EVENT_LOGS_END: &str = "docker-logs-end";

/// A machine where Docker wasn't found is looked at again after this long.
const RECHECK_MISSING: i64 = 24 * 3600;
/// A connection nobody has used for this long is closed.
const IDLE: Duration = Duration::from_secs(5 * 60);
/// Quick commands (listing, starting, stopping).
const QUICK: Duration = Duration::from_secs(60);
/// Stopping can take a while: Docker waits 10 seconds per container before it kills.
const SLOW: Duration = Duration::from_secs(5 * 60);
/// Pulling images and building can take many minutes on a slow line.
const LONG: Duration = Duration::from_secs(30 * 60);
/// Logs followed at once on one host. OpenSSH allows 10 channels per connection.
const MAX_LOGS_PER_HOST: usize = 3;
/// What an action's answer keeps of docker's output.
const OUTPUT_LINES: usize = 60;

/// What stops Docker from being used on a machine, with what to do about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub kind: ProblemKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProblemKind {
    /// There is no docker command on the machine.
    NotInstalled,
    /// The SSH user isn't allowed to use Docker (not in the docker group).
    Permission,
    /// Docker is installed but not running.
    Daemon,
    /// No docker compose command, so stacks can't be managed.
    NoCompose,
    /// Logging in needs the stored password, held back until the owner approves this
    /// machine's identity (on the Machines screen).
    NewHost,
    HostKeyChanged,
    Auth,
    Unreachable,
    Other,
}

impl Problem {
    fn new(kind: ProblemKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }

    /// Problems that come from logging in. The app doesn't retry those by itself:
    /// repeated failed logins get this PC locked out by tools like fail2ban.
    fn is_login(&self) -> bool {
        matches!(self.kind, ProblemKind::NewHost | ProblemKind::HostKeyChanged | ProblemKind::Auth)
    }
}

impl From<Problem> for Error {
    fn from(p: Problem) -> Self {
        Error::Network(p.message)
    }
}

/// One machine that is, or could be, a Docker host.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerHost {
    pub device_id: i64,
    pub mode: HostMode,
    /// True if Docker was found, false if not. None: not looked at yet.
    pub found: Option<bool>,
    /// The Docker engine's version, when it answered.
    pub version: Option<String>,
    pub compose: Compose,
    /// Unix seconds of the last look.
    pub checked_at: Option<i64>,
    /// What went wrong the last time, if anything.
    pub problem: Option<Problem>,
    /// True when the screen should look for Docker on it now.
    pub needs_check: bool,
}

/// A compose stack and its containers. A stack with none is down, remembered from
/// an earlier look.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stack {
    pub name: String,
    pub working_dir: Option<String>,
    pub config_files: Vec<String>,
    pub containers: Vec<Container>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub device_id: i64,
    /// Unix seconds.
    pub read_at: i64,
    pub stacks: Vec<Stack>,
    /// Containers that aren't part of a compose stack.
    pub others: Vec<Container>,
}

/// A host's state, with its containers when they could be read.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    pub host: DockerHost,
    pub overview: Option<Overview>,
}

/// What docker said while doing something, for the screen to show if wanted.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionOutput {
    pub output: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedFile {
    /// Where the copy of the old file was put.
    pub backup: String,
    /// What compose says is wrong with the stack's files now, if anything.
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupResult {
    pub removed: u32,
    /// Worked out from the preview's sizes of what was removed.
    pub freed_bytes: u64,
    /// One line per thing that was not removed, with docker's reason.
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupPick {
    #[serde(default)]
    pub containers: Vec<String>,
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default)]
    pub volumes: Vec<String>,
    #[serde(default)]
    pub build_cache: bool,
}

/// An open connection to one host, shared by everything the screen asks of it.
struct Conn {
    session: Session,
    /// The device's address, port and user when it was opened.
    place: String,
    last_used: Mutex<Instant>,
}

impl Conn {
    fn touch(&self) {
        *self.last_used.lock().unwrap() = Instant::now();
    }
}

struct LogStream {
    device_id: i64,
    stop: Arc<Notify>,
}

#[derive(Default)]
pub struct Docker {
    conns: Mutex<HashMap<i64, Arc<Conn>>>,
    /// One login to a host at a time, so two requests don't open two connections.
    dialing: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
    /// How the last try on each host went: a problem, or None when it worked. A host
    /// not tried yet this session has no entry.
    problems: Mutex<HashMap<i64, Option<Problem>>>,
    logs: Mutex<HashMap<u32, LogStream>>,
    next_log: AtomicU32,
}

/// Machines that may be Docker hosts: tracked, with an SSH user, and not Windows (the
/// scripts here are for `sh`).
fn may_host(d: &Device) -> bool {
    d.kind != DeviceKind::Ignore && d.os != Os::Windows && d.ssh_user.as_deref().is_some_and(|u| !u.trim().is_empty())
}

/// Whether the screen should look for Docker on a machine now.
fn needs_check(row: &store::HostRow, problem: Option<&Problem>, now: i64) -> bool {
    if row.mode == HostMode::Off || problem.is_some_and(Problem::is_login) {
        return false;
    }
    match row.found {
        None => true,
        Some(true) => false,
        Some(false) => row.checked_at.is_none_or(|at| now - at >= RECHECK_MISSING),
    }
}

fn ssh_problem(e: SshError) -> Problem {
    match e {
        SshError::NewHost(_) => Problem::new(
            ProblemKind::NewHost,
            "The stored password has not been sent: this machine's identity needs your approval first. Approve \
             it on the Machines screen.",
        ),
        SshError::HostKeyChanged { .. } => Problem::new(
            ProblemKind::HostKeyChanged,
            format!("{e} Nothing is sent to it until you have checked and approved it on the Machines screen."),
        ),
        SshError::AuthFailed(m) => Problem::new(ProblemKind::Auth, m),
        SshError::Unreachable(m) => Problem::new(ProblemKind::Unreachable, m),
        SshError::Other(m) => Problem::new(ProblemKind::Other, m),
    }
}

/// The first line of docker's complaint, as a sentence.
fn said(out: &ExecOutput) -> String {
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
    let line = parse::strip_ansi(&text)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(parse::text)
        .unwrap_or_default();
    line
}

/// Why a docker command didn't work, with advice for the usual setup problems.
fn docker_problem(d: &Device, out: &ExecOutput) -> Problem {
    let all = format!("{}\n{}", String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
    let user = d.ssh_user.as_deref().unwrap_or("").trim();
    match parse::refusal(&all, out.exit_status) {
        Some(Refusal::Permission) => Problem::new(
            ProblemKind::Permission,
            format!(
                "Docker is installed on {name}, but the user {user} isn't allowed to use it. On {name}, run: sudo \
                 usermod -aG docker {user} — then press Try again here (home-cc logs in afresh, which picks up the \
                 new group).",
                name = d.name
            ),
        ),
        Some(Refusal::Daemon) => Problem::new(
            ProblemKind::Daemon,
            format!(
                "Docker is installed on {name} but isn't running. Start it there with: sudo systemctl start docker \
                 (and sudo systemctl enable docker to have it start with the machine).",
                name = d.name
            ),
        ),
        Some(Refusal::Missing) => {
            Problem::new(ProblemKind::NotInstalled, format!("There is no docker command on {}.", d.name))
        }
        None => {
            let said = said(out);
            let message = if said.is_empty() {
                format!("Docker on {} gave no answer (it ended with {:?}).", d.name, out.exit_status)
            } else {
                said
            };
            Problem::new(ProblemKind::Other, message)
        }
    }
}

/// The last lines of what a command printed, cleaned up for the screen.
fn tail(out: &ExecOutput) -> String {
    let text = parse::strip_ansi(&format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ));
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(OUTPUT_LINES)..].join("\n")
}

impl Docker {
    /// Start the timer that closes connections nobody is using.
    pub fn start(app: &AppHandle) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                app.state::<Docker>().close_idle().await;
            }
        });
    }

    async fn close_idle(&self) {
        let idle: Vec<Arc<Conn>> = {
            let mut conns = self.conns.lock().unwrap();
            let quiet: Vec<i64> = conns
                .iter()
                // A connection a log is still following is in use however long it is quiet.
                .filter(|(_, c)| Arc::strong_count(c) == 1 && c.last_used.lock().unwrap().elapsed() >= IDLE)
                .map(|(id, _)| *id)
                .collect();
            quiet.into_iter().filter_map(|id| conns.remove(&id)).collect()
        };
        for conn in idle {
            if let Ok(conn) = Arc::try_unwrap(conn) {
                conn.session.close().await;
            }
        }
    }

    fn device(app: &AppHandle, device_id: i64) -> Result<Device> {
        let d = app.state::<Inventory>().get(device_id)?;
        if d.os == Os::Windows {
            return Err(Error::Invalid(format!(
                "{} is a Windows machine. home-cc manages Docker on Linux machines only.",
                d.name
            )));
        }
        if !may_host(&d) {
            return Err(Error::Invalid(format!("Add an SSH user to {} first, on the Machines screen.", d.name)));
        }
        Ok(d)
    }

    fn note(&self, device_id: i64, problem: Option<&Problem>) {
        self.problems.lock().unwrap().insert(device_id, problem.cloned());
    }

    /// A login problem the machine stats already ran into, so the Docker screen
    /// doesn't knock on the same door again by itself.
    fn stats_login_problem(app: &AppHandle, device_id: i64) -> Option<Problem> {
        let entries = app.state::<Monitor>().stats_entries(&app.state::<Inventory>()).ok()?;
        let entry = entries.into_iter().find(|e| e.device_id == device_id && e.paused)?;
        let kind = match entry.error_kind? {
            StatsErrorKind::NewHost => ProblemKind::NewHost,
            StatsErrorKind::HostKeyChanged => ProblemKind::HostKeyChanged,
            StatsErrorKind::Auth => ProblemKind::Auth,
            _ => return None,
        };
        Some(Problem::new(
            kind,
            format!("{} See the Machines screen.", entry.error.unwrap_or_else(|| "home-cc can't log in.".into())),
        ))
    }

    fn host(&self, app: &AppHandle, d: &Device) -> Result<DockerHost> {
        let row = app.state::<Inventory>().docker_host(d)?;
        // Until Docker itself has been tried here, a login the stats were turned away
        // with counts too, so the screen doesn't knock on the same door by itself.
        let tried = self.problems.lock().unwrap().get(&d.id).cloned();
        let problem = match tried {
            Some(outcome) => outcome,
            None => Self::stats_login_problem(app, d.id),
        };
        Ok(DockerHost {
            device_id: d.id,
            needs_check: needs_check(&row, problem.as_ref(), now()),
            mode: row.mode,
            found: row.found,
            version: row.version,
            compose: row.compose,
            checked_at: row.checked_at,
            problem,
        })
    }

    /// Every machine that is or could be a Docker host.
    pub fn hosts(&self, app: &AppHandle) -> Result<Vec<DockerHost>> {
        let devices = app.state::<Inventory>().list()?;
        devices.iter().filter(|d| may_host(d)).map(|d| self.host(app, d)).collect()
    }

    pub fn set_mode(&self, app: &AppHandle, device_id: i64, mode: HostMode) -> Result<DockerHost> {
        let d = Self::device(app, device_id)?;
        app.state::<Inventory>().set_docker_mode(&d, mode)?;
        if mode == HostMode::Off {
            self.drop_conn(device_id);
        }
        self.host(app, &d)
    }

    fn drop_conn(&self, device_id: i64) {
        self.conns.lock().unwrap().remove(&device_id);
    }

    /// The open connection to a machine, or a new one.
    async fn conn(&self, app: &AppHandle, d: &Device) -> std::result::Result<(Arc<Conn>, bool), Problem> {
        let turn = self.dialing.lock().unwrap().entry(d.id).or_default().clone();
        let _one_at_a_time = turn.lock().await;
        let here = store::place(d);
        if let Some(conn) = self.conns.lock().unwrap().get(&d.id).cloned() {
            if conn.place == here && !conn.session.is_closed() {
                conn.touch();
                return Ok((conn, true));
            }
        }
        self.drop_conn(d.id);

        let (login, password_held_back) =
            monitor::prepared_login(app, d).await.map_err(|e| Problem::new(ProblemKind::Other, e.to_string()))?;
        // As for the stats: a first identity is remembered straight away, not approved;
        // the password stays back until the owner has approved it.
        let shown: Arc<Mutex<Option<HostKey>>> = Arc::default();
        let (seen, first_app, device_id) = (shown.clone(), app.clone(), d.id);
        let (host, port) = (login.host.clone(), login.port);
        let first_use = move |key: &HostKey| {
            *seen.lock().unwrap() = Some(key.clone());
            if let Err(e) = first_app.state::<Inventory>().remember_host_key(device_id, &host, port, key) {
                crate::diag::note(format!("could not store the identity of device {device_id}: {e}"));
            }
        };
        let session = match ssh::connect(&login, &first_use).await {
            Ok(session) => session,
            Err(SshError::AuthFailed(_)) if password_held_back => {
                return Err(ssh_problem(match login.known_key.clone().or_else(|| shown.lock().unwrap().clone()) {
                    Some(key) => SshError::NewHost(key),
                    None => SshError::AuthFailed("Could not log in, and the machine did not show an identity to approve.".into()),
                }));
            }
            Err(e) => return Err(ssh_problem(e)),
        };
        let conn = Arc::new(Conn { session, place: here, last_used: Mutex::new(Instant::now()) });
        self.conns.lock().unwrap().insert(d.id, conn.clone());
        Ok((conn, false))
    }

    /// Run a script with `sh -s` on the machine. A connection that turns out to have
    /// died while it sat open is replaced once.
    async fn run(&self, app: &AppHandle, d: &Device, script: &str, limit: Duration) -> std::result::Result<ExecOutput, Problem> {
        let mut retried = false;
        loop {
            let (conn, reused) = match self.conn(app, d).await {
                Ok(c) => c,
                Err(p) => {
                    self.note(d.id, Some(&p));
                    return Err(p);
                }
            };
            match conn.session.exec("sh -s", Some(script.as_bytes()), limit).await {
                Ok(out) => {
                    conn.touch();
                    return Ok(out);
                }
                Err(e) => {
                    if reused && !retried && conn.session.is_closed() {
                        retried = true;
                        self.drop_conn(d.id);
                        continue;
                    }
                    if conn.session.is_closed() {
                        self.drop_conn(d.id);
                    }
                    let p = ssh_problem(e);
                    self.note(d.id, Some(&p));
                    return Err(p);
                }
            }
        }
    }

    /// Run a docker script and insist that it worked.
    async fn run_docker(&self, app: &AppHandle, d: &Device, script: &str, limit: Duration) -> std::result::Result<ExecOutput, Problem> {
        let out = self.run(app, d, script, limit).await?;
        if out.exit_status == Some(0) {
            return Ok(out);
        }
        let p = docker_problem(d, &out);
        if p.kind == ProblemKind::Permission {
            // Group changes only apply to a new login.
            self.drop_conn(d.id);
        }
        if matches!(p.kind, ProblemKind::Permission | ProblemKind::Daemon | ProblemKind::NotInstalled) {
            self.note(d.id, Some(&p));
        }
        Err(p)
    }

    /// Look for Docker on a machine and store what was found.
    pub async fn probe(&self, app: &AppHandle, device_id: i64) -> Result<DockerHost> {
        let d = Self::device(app, device_id)?;
        let outcome = self.probe_device(app, &d).await;
        self.note(d.id, outcome.as_ref().err());
        self.host(app, &d)
    }

    async fn probe_device(&self, app: &AppHandle, d: &Device) -> std::result::Result<Compose, Problem> {
        let out = self.run(app, d, &scripts::probe(), QUICK).await?;
        let inv = app.state::<Inventory>();
        let store = |e: crate::error::Error| Problem::new(ProblemKind::Other, e.to_string());
        match parse::probe(&String::from_utf8_lossy(&out.stdout)) {
            Some(Probe::Missing) => {
                inv.record_docker_probe(d, false, None, Compose::None, now()).map_err(store)?;
                Err(Problem::new(ProblemKind::NotInstalled, format!("There is no docker command on {}.", d.name)))
            }
            Some(Probe::Present { version, compose, error }) => {
                inv.record_docker_probe(d, true, version.as_deref(), compose, now()).map_err(store)?;
                match error {
                    None => Ok(compose),
                    Some(said) => {
                        let fake = ExecOutput { stderr: said.into_bytes(), exit_status: Some(1), ..ExecOutput::default() };
                        let p = docker_problem(d, &fake);
                        if p.kind == ProblemKind::Permission {
                            self.drop_conn(d.id);
                        }
                        Err(p)
                    }
                }
            }
            None => Err(Problem::new(
                ProblemKind::Other,
                format!("{} did not answer the question about Docker. {}", d.name, said(&out)),
            )),
        }
    }

    /// Containers and stacks on one host. `auto` is set for the screen's own timer: a
    /// host that turned the login away is then not tried again until the owner asks.
    pub async fn overview(&self, app: &AppHandle, device_id: i64, auto: bool) -> Result<HostView> {
        let d = Self::device(app, device_id)?;
        if auto {
            let host = self.host(app, &d)?;
            if host.problem.as_ref().is_some_and(Problem::is_login) {
                return Ok(HostView { host, overview: None });
            }
        }
        let outcome = self.read_overview(app, &d).await;
        let (overview, problem) = match outcome {
            Ok(o) => (Some(o), None),
            Err(p) => (None, Some(p)),
        };
        self.note(d.id, problem.as_ref());
        Ok(HostView { host: self.host(app, &d)?, overview })
    }

    async fn read_overview(&self, app: &AppHandle, d: &Device) -> std::result::Result<Overview, Problem> {
        let inv = app.state::<Inventory>();
        let store = |e: crate::error::Error| Problem::new(ProblemKind::Other, e.to_string());
        if inv.docker_host(d).map_err(store)?.found != Some(true) {
            self.probe_device(app, d).await?;
        }
        let out = self.run(app, d, &scripts::list_containers(), QUICK).await?;
        let text = String::from_utf8_lossy(&out.stdout);
        let containers = parse::containers(&text);
        // A container removed between listing and reading makes docker end with an
        // error while the rest is fine; only no answer at all is a failure.
        if out.exit_status != Some(0) && containers.is_empty() {
            return Err(docker_problem(d, &out));
        }
        let (by_project, others) = parse::by_project(containers);
        let seen: Vec<StackPlace> = by_project
            .iter()
            .map(|(name, list)| StackPlace {
                project: name.clone(),
                working_dir: list.iter().find_map(|c| c.working_dir.clone()),
                config_files: list.iter().find(|c| !c.config_files.is_empty()).map(|c| c.config_files.clone()).unwrap_or_default(),
            })
            .collect();
        inv.record_docker_stacks(d, &seen, now()).map_err(store)?;
        let mut by_project = by_project;
        let stacks = inv
            .docker_stacks(d)
            .map_err(store)?
            .into_iter()
            .map(|place| Stack {
                containers: by_project.remove(&place.project).unwrap_or_default(),
                name: place.project,
                working_dir: place.working_dir,
                config_files: place.config_files,
            })
            .collect();
        Ok(Overview { device_id: d.id, read_at: now(), stacks, others })
    }

    /// CPU and memory of the running containers.
    pub async fn stats(&self, app: &AppHandle, device_id: i64) -> Result<Vec<ContainerStats>> {
        let d = Self::device(app, device_id)?;
        let out = self.run_docker(app, &d, &scripts::stats(), QUICK).await?;
        Ok(parse::stats(&String::from_utf8_lossy(&out.stdout)))
    }

    pub async fn container_action(
        &self,
        app: &AppHandle,
        device_id: i64,
        container_id: &str,
        action: ContainerAction,
    ) -> Result<ActionOutput> {
        let d = Self::device(app, device_id)?;
        if !parse::is_container_id(container_id) {
            return Err(Error::Invalid("That isn't a container id.".into()));
        }
        let limit = if action == ContainerAction::Start { QUICK } else { SLOW };
        let out = self.run_docker(app, &d, &scripts::container_action(action, container_id), limit).await?;
        Ok(ActionOutput { output: tail(&out) })
    }

    fn stack_place(app: &AppHandle, d: &Device, project: &str) -> Result<StackPlace> {
        app.state::<Inventory>().docker_stack(d, project)?.ok_or_else(|| {
            Error::Invalid(format!("home-cc doesn't know a stack called {project} on {}. Refresh the list.", d.name))
        })
    }

    fn compose(app: &AppHandle, d: &Device) -> Result<Compose> {
        match app.state::<Inventory>().docker_host(d)?.compose {
            Compose::None => Err(Problem::new(
                ProblemKind::NoCompose,
                format!(
                    "Docker Compose isn't installed on {}, so stacks can't be managed from here. Install the \
                     docker-compose-plugin package there, then press Check again.",
                    d.name
                ),
            )
            .into()),
            c => Ok(c),
        }
    }

    pub async fn stack_action(&self, app: &AppHandle, device_id: i64, project: &str, action: StackAction) -> Result<ActionOutput> {
        let d = Self::device(app, device_id)?;
        let place = Self::stack_place(app, &d, project)?;
        let compose = Self::compose(app, &d)?;
        let script = scripts::stack_action(compose, &place, action).expect("compose is there");
        let limit = match action {
            StackAction::Pull | StackAction::Build | StackAction::Up => LONG,
            StackAction::Down | StackAction::Restart => SLOW,
        };
        let out = self.run(app, &d, &script, limit).await?;
        if out.exit_status == Some(0) {
            return Ok(ActionOutput { output: tail(&out) });
        }
        let p = docker_problem(&d, &out);
        if p.kind == ProblemKind::Other {
            // Compose's own output says what went wrong; give the end of it, not just the first line.
            let what = tail(&out);
            return Err(Error::Network(if what.is_empty() { p.message } else { what }));
        }
        Err(p.into())
    }

    pub fn forget_stack(&self, app: &AppHandle, device_id: i64, project: &str) -> Result<()> {
        let d = Self::device(app, device_id)?;
        app.state::<Inventory>().forget_docker_stack(&d, project)
    }

    /// The files of a stack that can be opened: its compose files first, then the
    /// compose files, Dockerfiles and .env files in its folder.
    pub async fn stack_files(&self, app: &AppHandle, device_id: i64, project: &str) -> Result<Vec<String>> {
        let d = Self::device(app, device_id)?;
        let place = Self::stack_place(app, &d, project)?;
        let mut files: Vec<String> = place.config_files.clone();
        if let Some(dir) = &place.working_dir {
            let out = self.run(app, &d, &scripts::list_stack_files(dir), QUICK).await?;
            let base = dir.trim_end_matches('/');
            let mut found: Vec<String> = String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter_map(|l| l.strip_prefix("./"))
                .filter(|l| !l.is_empty() && !l.contains(['\n', '\r']))
                .map(|l| format!("{base}/{l}"))
                .collect();
            found.sort();
            for f in found {
                if !files.contains(&f) {
                    files.push(f);
                }
            }
        }
        Ok(files)
    }

    /// Only files [`Self::stack_files`] lists for the stack can be opened or saved.
    async fn checked_path(&self, app: &AppHandle, d: &Device, project: &str, path: &str) -> Result<()> {
        if self.stack_files(app, d.id, project).await?.iter().any(|f| f == path) {
            Ok(())
        } else {
            Err(Error::Invalid(format!("{path} isn't one of the files of the {project} stack.")))
        }
    }

    pub async fn read_file(&self, app: &AppHandle, device_id: i64, project: &str, path: &str) -> Result<String> {
        let d = Self::device(app, device_id)?;
        self.checked_path(app, &d, project, path).await?;
        let out = self.run(app, &d, &scripts::read_file(path), QUICK).await?;
        match out.exit_status {
            Some(0) => {}
            Some(91) => return Err(Error::Invalid(format!("{path} is too big to open here."))),
            Some(92) => return Err(Error::Invalid(format!("{path} is no longer on {}.", d.name))),
            _ => return Err(Error::Network(format!("Couldn't read {path}: {}", said(&out)))),
        }
        if out.truncated {
            return Err(Error::Invalid(format!("{path} is too big to open here.")));
        }
        String::from_utf8(out.stdout).map_err(|_| Error::Invalid(format!("{path} isn't a text file home-cc can edit.")))
    }

    /// Replace a file, keeping a copy of the old one. Refused if the file changed on
    /// the machine since `original` was read from it.
    pub async fn save_file(
        &self,
        app: &AppHandle,
        device_id: i64,
        project: &str,
        path: &str,
        original: &str,
        text: &str,
    ) -> Result<SavedFile> {
        let d = Self::device(app, device_id)?;
        self.checked_path(app, &d, project, path).await?;
        if text.len() > scripts::MAX_FILE {
            return Err(Error::Invalid("That is too much text for one file.".into()));
        }
        let out = self.run(app, &d, &scripts::save_file(path, original, text), QUICK).await?;
        let backup = String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|l| l.strip_prefix("@@backup ").map(str::to_string));
        match (out.exit_status, backup) {
            (Some(0), Some(backup)) => {
                let place = Self::stack_place(app, &d, project)?;
                let warning = match Self::compose(app, &d).ok().and_then(|c| scripts::compose_check(c, &place)) {
                    Some(check) => match self.run(app, &d, &check, QUICK).await {
                        Ok(out) if out.exit_status == Some(0) => None,
                        Ok(out) => Some(tail(&out)).filter(|t| !t.is_empty()).or(Some("Compose doesn't accept the stack's files.".into())),
                        Err(_) => None,
                    },
                    None => None,
                };
                Ok(SavedFile { backup, warning })
            }
            (Some(scripts::SAVE_CHANGED), _) => Err(Error::Invalid(format!(
                "Not saved: {path} was changed on {} after you opened it. Open it again to see the new version.",
                d.name
            ))),
            (Some(scripts::SAVE_NO_BACKUP), _) => Err(Error::Invalid(format!(
                "Not saved: home-cc couldn't make a backup copy next to {path} (is the folder writable for {}?).",
                d.ssh_user.as_deref().unwrap_or("the SSH user")
            ))),
            (Some(scripts::SAVE_NO_WRITE), _) => Err(Error::Invalid(format!(
                "Not saved: {} may not write to {path}. The file is unchanged.",
                d.ssh_user.as_deref().unwrap_or("the SSH user")
            ))),
            (Some(scripts::SAVE_NO_TOOLS), _) => Err(Error::Invalid(format!(
                "Not saved: {} lacks the base64 or cmp command that saving needs.",
                d.name
            ))),
            _ => Err(Error::Network(format!("Not saved: {}", said(&out)))),
        }
    }

    pub async fn cleanup_preview(&self, app: &AppHandle, device_id: i64) -> Result<CleanupPreview> {
        let d = Self::device(app, device_id)?;
        let out = self.run_docker(app, &d, &scripts::disk_usage(), SLOW).await?;
        parse::cleanup_preview(&String::from_utf8_lossy(&out.stdout))
            .ok_or_else(|| Error::Network(format!("Docker on {} didn't list what it keeps. {}", d.name, said(&out))))
    }

    /// Remove what the owner picked. A fresh preview is read first, both for the sizes
    /// and so that only things that are still unused are passed on.
    pub async fn cleanup(&self, app: &AppHandle, device_id: i64, pick: CleanupPick) -> Result<CleanupResult> {
        let preview = self.cleanup_preview(app, device_id).await?;
        let d = Self::device(app, device_id)?;
        let keep = |picked: &[String], list: &[parse::CleanupItem]| -> Vec<parse::CleanupItem> {
            list.iter().filter(|i| picked.contains(&i.key)).cloned().collect()
        };
        let containers = keep(&pick.containers, &preview.containers);
        let images = keep(&pick.images, &preview.images);
        let volumes = keep(&pick.volumes, &preview.volumes);
        let build_cache = pick.build_cache && preview.build_cache_count > 0;
        let keys = |items: &[parse::CleanupItem]| items.iter().map(|i| i.key.clone()).collect::<Vec<_>>();
        let (ck, ik, vk) = (keys(&containers), keys(&images), keys(&volumes));
        if !(ck.iter().all(|k| parse::is_container_id(k))
            && ik.iter().all(|k| parse::is_image_ref(k))
            && vk.iter().all(|k| parse::is_volume_name(k)))
        {
            return Err(Error::Invalid("Something in the list isn't a name home-cc can pass on safely.".into()));
        }
        if ck.is_empty() && ik.is_empty() && vk.is_empty() && !build_cache {
            return Ok(CleanupResult::default());
        }
        let script = scripts::cleanup(&ck, &ik, &vk, build_cache);
        let out = self.run(app, &d, &script, LONG).await?;
        let mut result = CleanupResult::default();
        let size_of = |kind: &str, key: &str| -> u64 {
            let list = match kind {
                "container" => &containers,
                "image" => &images,
                "volume" => &volumes,
                _ => return if key == "all" { preview.build_cache_bytes } else { 0 },
            };
            list.iter().find(|i| i.key == key).and_then(|i| i.size_bytes).unwrap_or(0)
        };
        let label_of = |kind: &str, key: &str| -> String {
            [&containers, &images, &volumes]
                .iter()
                .find_map(|l| l.iter().find(|i| i.key == key))
                .map(|i| i.label.clone())
                .unwrap_or_else(|| if kind == "cache" { "Build cache".into() } else { key.to_string() })
        };
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(rest) = line.strip_prefix("@@ok ") {
                let (kind, key) = rest.split_once(' ').unwrap_or((rest, ""));
                result.removed += 1;
                result.freed_bytes += size_of(kind, key.trim());
            } else if let Some(rest) = line.strip_prefix("@@fail ") {
                let (kind, rest) = rest.split_once(' ').unwrap_or((rest, ""));
                let (key, why) = rest.split_once(' ').unwrap_or((rest, ""));
                let why = parse::text(why);
                result.failures.push(format!("{}: {}", label_of(kind, key), if why.is_empty() { "not removed" } else { &why }));
            }
        }
        Ok(result)
    }

    /// Follow a container's log. Lines arrive as "docker-logs" events, and the end as
    /// one "docker-logs-end" event, which also comes after [`Self::stop_logs`].
    pub async fn start_logs(&self, app: &AppHandle, device_id: i64, container_id: &str, tail: Option<u32>) -> Result<u32> {
        let d = Self::device(app, device_id)?;
        if !parse::is_container_id(container_id) {
            return Err(Error::Invalid("That isn't a container id.".into()));
        }
        let open = self.logs.lock().unwrap().values().filter(|l| l.device_id == d.id).count();
        if open >= MAX_LOGS_PER_HOST {
            return Err(Error::Invalid(format!(
                "{MAX_LOGS_PER_HOST} logs from {} are open already. Close one first.",
                d.name
            )));
        }
        let (conn, _) = self.conn(app, &d).await?;
        let channel = conn.session.open_channel().await.map_err(ssh_problem)?;
        conn.touch();

        let id = self.next_log.fetch_add(1, Ordering::Relaxed) + 1;
        let stop = Arc::new(Notify::new());
        self.logs.lock().unwrap().insert(id, LogStream { device_id: d.id, stop: stop.clone() });
        let command = scripts::follow_logs(container_id, tail.map(|n| n.min(100_000)));
        let (app, name) = (app.clone(), d.name.clone());
        tauri::async_runtime::spawn(async move {
            let emit_app = app.clone();
            let emit = move |lines: Vec<String>| {
                emit_app.emit(EVENT_LOGS, logs::LogLines { id, lines }).ok();
            };
            let reason = logs::follow(channel, &command, stop, emit, &name).await;
            conn.touch();
            drop(conn);
            app.state::<Docker>().logs.lock().unwrap().remove(&id);
            app.emit(EVENT_LOGS_END, logs::LogEnd { id, reason }).ok();
        });
        Ok(id)
    }

    pub fn stop_logs(&self, id: u32) {
        if let Some(stream) = self.logs.lock().unwrap().get(&id) {
            stream.stop.notify_one();
        }
    }
}

/// A file name that is safe on Windows and Linux: letters, digits, dots, dashes and
/// underscores only.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' })
        .take(120)
        .collect();
    let cleaned = cleaned.trim_matches(['.', '-']).to_string();
    if cleaned.is_empty() { "docker-log".to_string() } else { cleaned }
}

/// Save text into `dir` under `name`, adding " (2)" and so on rather than replace a
/// file that is there. Returns the path.
pub fn save_text(dir: &std::path::Path, name: &str, text: &str) -> Result<std::path::PathBuf> {
    let name = safe_file_name(name);
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    std::fs::create_dir_all(dir).map_err(|e| Error::Invalid(format!("Couldn't open {}: {e}", dir.display())))?;
    for n in 1..1000 {
        let file = if n == 1 { format!("{stem}{ext}") } else { format!("{stem} ({n}){ext}") };
        let path = dir.join(file);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(text.as_bytes())
                    .map_err(|e| Error::Invalid(format!("Couldn't write {}: {e}", path.display())))?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::Invalid(format!("Couldn't save in {}: {e}", dir.display()))),
        }
    }
    Err(Error::Invalid(format!("Too many files called {stem} in {} already.", dir.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(mode: HostMode, found: Option<bool>, checked_at: Option<i64>) -> store::HostRow {
        store::HostRow { mode, found, checked_at, ..store::HostRow::default() }
    }

    #[test]
    fn when_to_look_for_docker() {
        let now = 10 * RECHECK_MISSING;
        assert!(needs_check(&row(HostMode::Auto, None, None), None, now), "never looked");
        assert!(needs_check(&row(HostMode::On, None, None), None, now));
        assert!(!needs_check(&row(HostMode::Off, None, None), None, now), "the owner said no");
        assert!(!needs_check(&row(HostMode::Auto, Some(true), Some(0)), None, now), "found: the list itself checks");
        assert!(!needs_check(&row(HostMode::Auto, Some(false), Some(now - 60)), None, now), "looked a minute ago");
        assert!(needs_check(&row(HostMode::Auto, Some(false), Some(now - RECHECK_MISSING)), None, now), "a day ago");
        let refused = Problem::new(ProblemKind::Auth, "no");
        assert!(!needs_check(&row(HostMode::Auto, None, None), Some(&refused), now), "not after a refused login");
        let down = Problem::new(ProblemKind::Unreachable, "off");
        assert!(needs_check(&row(HostMode::Auto, None, None), Some(&down), now), "a machine that was off is tried again");
    }

    fn device(user: &str) -> Device {
        Device {
            id: 1,
            name: "Homelab VM".into(),
            host: "192.168.68.21".into(),
            kind: DeviceKind::Vm,
            os: Os::Linux,
            ssh_user: Some(user.into()),
            ssh_port: 22,
            notes: String::new(),
            mac: None,
            vendor: None,
            hostname: None,
            source: crate::inventory::Source::Manual,
            reviewed: true,
            first_seen: 0,
            last_seen: None,
        }
    }

    #[test]
    fn which_machines_may_be_docker_hosts() {
        assert!(may_host(&device("sam")));
        assert!(!may_host(&device(" ")));
        assert!(!may_host(&Device { os: Os::Windows, ..device("sam") }));
        assert!(!may_host(&Device { kind: DeviceKind::Ignore, ..device("sam") }));
        assert!(may_host(&Device { os: Os::Proxmox, ..device("root") }));
    }

    #[test]
    fn docker_group_problem_says_what_to_run() {
        let out = ExecOutput {
            stderr: b"permission denied while trying to connect to the docker API at unix:///var/run/docker.sock\n".to_vec(),
            exit_status: Some(1),
            ..ExecOutput::default()
        };
        let p = docker_problem(&device("sam"), &out);
        assert_eq!(p.kind, ProblemKind::Permission);
        assert!(p.message.contains("sudo usermod -aG docker sam"), "{}", p.message);
        assert!(p.message.contains("Homelab VM"));

        let other = ExecOutput { stdout: b"\n\x1b[31mError response from daemon: No such container: x\x1b[0m\n".to_vec(), exit_status: Some(1), ..ExecOutput::default() };
        let p = docker_problem(&device("sam"), &other);
        assert_eq!((p.kind, p.message.as_str()), (ProblemKind::Other, "Error response from daemon: No such container: x"));
    }

    #[test]
    fn output_tail_keeps_the_end() {
        let lines: Vec<String> = (0..100).map(|i| format!("line {i}")).collect();
        let out = ExecOutput { stdout: lines.join("\n").into_bytes(), ..ExecOutput::default() };
        let t = tail(&out);
        assert_eq!(t.lines().count(), OUTPUT_LINES);
        assert!(t.ends_with("line 99"));
    }

    #[test]
    fn saved_logs_get_safe_names_and_never_replace_a_file() {
        assert_eq!(safe_file_name("homelab media/jellyfin 2026-11-17.log"), "homelab-media-jellyfin-2026-11-17.log");
        assert_eq!(safe_file_name("../../etc/passwd"), "etc-passwd");
        assert_eq!(safe_file_name("..."), "docker-log");
        let dir = std::env::temp_dir().join(format!("home-cc-docker-logfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = save_text(&dir, "web.log", "one").unwrap();
        let b = save_text(&dir, "web.log", "two").unwrap();
        assert_eq!(a.file_name().unwrap(), "web.log");
        assert_eq!(b.file_name().unwrap(), "web (2).log");
        assert_eq!(std::fs::read_to_string(a).unwrap(), "one");
        assert_eq!(std::fs::read_to_string(b).unwrap(), "two");
    }
}
