//! What the Docker screen can ask the back end to do. Registered in lib.rs; the
//! typed wrappers for the screens are in src/lib/docker/api.ts.

use super::parse::{CleanupPreview, ContainerStats};
use super::scripts::{ContainerAction, StackAction};
use super::{ActionOutput, CleanupPick, CleanupResult, Docker, DockerHost, HostMode, HostView, SavedFile};
use crate::error::{Error, Result};
use tauri::{AppHandle, Manager, State};

/// Every machine that is or could be a Docker host, with what is known about it.
#[tauri::command(async)]
pub fn docker_hosts(app: AppHandle, docker: State<Docker>) -> Result<Vec<DockerHost>> {
    docker.hosts(&app)
}

/// Look for Docker on one machine now and store the answer.
#[tauri::command]
pub async fn docker_probe(app: AppHandle, docker: State<'_, Docker>, device_id: i64) -> Result<DockerHost> {
    docker.probe(&app, device_id).await
}

/// The owner marks a machine as a Docker host ("on"), as not one ("off"), or leaves
/// it to home-cc ("auto").
#[tauri::command(async)]
pub fn docker_set_host_mode(app: AppHandle, docker: State<Docker>, device_id: i64, mode: HostMode) -> Result<DockerHost> {
    docker.set_mode(&app, device_id, mode)
}

#[tauri::command]
pub async fn docker_overview(app: AppHandle, docker: State<'_, Docker>, device_id: i64, auto: bool) -> Result<HostView> {
    docker.overview(&app, device_id, auto).await
}

#[tauri::command]
pub async fn docker_stats(app: AppHandle, docker: State<'_, Docker>, device_id: i64) -> Result<Vec<ContainerStats>> {
    docker.stats(&app, device_id).await
}

#[tauri::command]
pub async fn docker_container_action(
    app: AppHandle,
    docker: State<'_, Docker>,
    device_id: i64,
    container_id: String,
    action: ContainerAction,
) -> Result<ActionOutput> {
    docker.container_action(&app, device_id, &container_id, action).await
}

#[tauri::command]
pub async fn docker_stack_action(
    app: AppHandle,
    docker: State<'_, Docker>,
    device_id: i64,
    stack: String,
    action: StackAction,
) -> Result<ActionOutput> {
    docker.stack_action(&app, device_id, &stack, action).await
}

/// Forget a stack that is down, so it no longer shows.
#[tauri::command(async)]
pub fn docker_forget_stack(app: AppHandle, docker: State<Docker>, device_id: i64, stack: String) -> Result<()> {
    docker.forget_stack(&app, device_id, &stack)
}

#[tauri::command]
pub async fn docker_stack_files(app: AppHandle, docker: State<'_, Docker>, device_id: i64, stack: String) -> Result<Vec<String>> {
    docker.stack_files(&app, device_id, &stack).await
}

#[tauri::command]
pub async fn docker_read_file(
    app: AppHandle,
    docker: State<'_, Docker>,
    device_id: i64,
    stack: String,
    path: String,
) -> Result<String> {
    docker.read_file(&app, device_id, &stack, &path).await
}

#[tauri::command]
pub async fn docker_save_file(
    app: AppHandle,
    docker: State<'_, Docker>,
    device_id: i64,
    stack: String,
    path: String,
    original: String,
    text: String,
) -> Result<SavedFile> {
    docker.save_file(&app, device_id, &stack, &path, &original, &text).await
}

#[tauri::command]
pub async fn docker_cleanup_preview(app: AppHandle, docker: State<'_, Docker>, device_id: i64) -> Result<CleanupPreview> {
    docker.cleanup_preview(&app, device_id).await
}

#[tauri::command]
pub async fn docker_cleanup(
    app: AppHandle,
    docker: State<'_, Docker>,
    device_id: i64,
    pick: CleanupPick,
) -> Result<CleanupResult> {
    docker.cleanup(&app, device_id, pick).await
}

/// Start following a container's log. Returns the id that its "docker-logs" and
/// "docker-logs-end" events carry. `tail` is how many earlier lines to start with
/// (None: all of them).
#[tauri::command]
pub async fn docker_logs_start(
    app: AppHandle,
    docker: State<'_, Docker>,
    device_id: i64,
    container_id: String,
    tail: Option<u32>,
) -> Result<u32> {
    docker.start_logs(&app, device_id, &container_id, tail).await
}

#[tauri::command]
pub fn docker_logs_stop(docker: State<Docker>, id: u32) {
    docker.stop_logs(id);
}

/// Save a log into this PC's Downloads folder. Returns where it went.
#[tauri::command(async)]
pub fn docker_save_log(app: AppHandle, name: String, text: String) -> Result<String> {
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(|e| Error::Invalid(format!("Couldn't find the Downloads folder: {e}")))?;
    Ok(super::save_text(&dir, &name, &text)?.display().to_string())
}
