//! The small `sh` scripts run on a Docker host. Each one is sent on stdin to `sh -s`,
//! as the stats script is, so it works whatever the user's login shell is (bash, zsh,
//! fish) and nothing has to survive two rounds of quoting. Every value from outside
//! (a path, a stack name, an image) goes in through [`quote`], and every docker
//! command reads from /dev/null so it can't swallow the rest of the script.

use super::parse::{Compose, INSPECT_FORMAT};
use data_encoding::BASE64;

/// Put first in every script. The usual places for docker (also the snap one) even
/// when the SSH login didn't set up a full PATH.
const PREAMBLE: &str = "PATH=\"$PATH:/usr/local/bin:/usr/bin:/bin:/snap/bin\"; export PATH\n";

/// `s` as a single word for `sh`, whatever it holds.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn script(body: &str) -> String {
    format!("{PREAMBLE}{body}")
}

/// Is docker there, does it answer, and which compose command is there? See
/// [`super::parse::probe`] for what it prints.
pub fn probe() -> String {
    script(
        r#"if ! command -v docker >/dev/null 2>&1; then echo "@@hcc-docker none"; exit 0; fi
echo "@@hcc-docker cli"
docker version --format '{{json .Server.Version}}' </dev/null 2>&1
echo "@@exit $?"
if docker compose version </dev/null >/dev/null 2>&1; then echo "@@compose plugin"
elif command -v docker-compose >/dev/null 2>&1; then echo "@@compose standalone"
else echo "@@compose none"; fi
"#,
    )
}

/// Every container, one line of JSON each (see [`INSPECT_FORMAT`]).
pub fn list_containers() -> String {
    script(&format!(
        "ids=$(docker ps -aq --no-trunc </dev/null) || exit $?\n\
         [ -n \"$ids\" ] || exit 0\n\
         docker inspect --format {} $ids </dev/null\n",
        quote(INSPECT_FORMAT)
    ))
}

/// CPU and memory of every running container. Takes about two seconds: Docker
/// measures CPU over a moment.
pub fn stats() -> String {
    script("docker stats --no-stream --no-trunc --format '{{json .}}' </dev/null\n")
}

/// What a container action runs. `id` must already be checked to be a container id.
pub fn container_action(action: ContainerAction, id: &str) -> String {
    let id = quote(id);
    let body = match action {
        ContainerAction::Start => format!("docker start {id}"),
        ContainerAction::Stop => format!("docker stop {id}"),
        ContainerAction::Restart => format!("docker restart {id}"),
        // A running container is stopped first; the screen asks before this.
        ContainerAction::Remove => format!("docker rm -f {id}"),
    };
    script(&format!("{body} </dev/null 2>&1\n"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerAction {
    Start,
    Stop,
    Restart,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StackAction {
    /// Create and start everything (`up -d`).
    Up,
    /// Stop and remove the stack's containers and networks. Volumes are kept.
    Down,
    /// Fetch newer images, then recreate what changed.
    Pull,
    /// Restart the containers as they are.
    Restart,
    /// Rebuild images that are built from a Dockerfile, then recreate.
    Build,
}

/// Where a stack lives, as remembered from its containers' labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackPlace {
    pub project: String,
    pub working_dir: Option<String>,
    pub config_files: Vec<String>,
}

/// The compose command for one stack: run from its folder, with its project name and
/// files, so it is the same stack that was started there, not a new one.
fn compose_command(compose: Compose, stack: &StackPlace) -> Option<String> {
    let base = match compose {
        Compose::Plugin => "docker compose",
        Compose::Standalone => "docker-compose",
        Compose::None => return None,
    };
    let mut cmd = format!("{base} -p {}", quote(&stack.project));
    for f in &stack.config_files {
        cmd += &format!(" -f {}", quote(f));
    }
    Some(cmd)
}

/// What a stack action runs, or None when the machine has no compose command.
pub fn stack_action(compose: Compose, stack: &StackPlace, action: StackAction) -> Option<String> {
    let c = compose_command(compose, stack)?;
    let run = |args: &str| format!("{c} {args} </dev/null 2>&1");
    let body = match action {
        StackAction::Up => run("up -d"),
        StackAction::Down => run("down"),
        StackAction::Pull => format!("{} && {}", run("pull"), run("up -d")),
        StackAction::Restart => run("restart"),
        StackAction::Build => run("up -d --build"),
    };
    Some(script(&format!("{}{body}\n", cd(stack))))
}

/// Go to the stack's folder first, as compose did when it was started; stop if it
/// is gone, rather than run compose somewhere else.
fn cd(stack: &StackPlace) -> String {
    match &stack.working_dir {
        Some(dir) => format!(
            "cd {} 2>/dev/null || {{ echo \"The folder {} is no longer on this machine.\"; exit 90; }}\n",
            quote(dir),
            dir.replace(['"', '\\', '$', '`'], "")
        ),
        None => String::new(),
    }
}

/// Checks a stack's compose files after an edit: prints nothing when they are fine,
/// or what compose says is wrong.
pub fn compose_check(compose: Compose, stack: &StackPlace) -> Option<String> {
    let c = compose_command(compose, stack)?;
    Some(script(&format!("{}{c} config -q </dev/null 2>&1\n", cd(stack))))
}

/// The files in a stack's folder worth editing: compose files, Dockerfiles and .env
/// files, two folders deep at most. One path per line.
pub fn list_stack_files(dir: &str) -> String {
    script(&format!(
        "cd {} 2>/dev/null || exit 90\n\
         find . -maxdepth 2 -type f \\( -name '*compose*.yml' -o -name '*compose*.yaml' -o -name 'Dockerfile*' \
         -o -name '*.Dockerfile' -o -name '.env' -o -name '*.env' \\) -size -1024k 2>/dev/null | head -n 100\n",
        quote(dir)
    ))
}

/// Largest file the editor opens. Compose files are a few kilobytes.
pub const MAX_FILE: usize = 512 * 1024;

/// Prints the file, or exits with 91 when it is too big, 92 when it isn't there.
pub fn read_file(path: &str) -> String {
    let p = quote(path);
    script(&format!(
        "[ -f {p} ] || exit 92\n\
         [ \"$(wc -c < {p})\" -le {MAX_FILE} ] || exit 91\n\
         cat -- {p}\n"
    ))
}

/// Exit codes of [`save_file`], each with its own message.
pub const SAVE_CHANGED: u32 = 93;
pub const SAVE_NO_BACKUP: u32 = 94;
pub const SAVE_NO_WRITE: u32 = 95;
pub const SAVE_NO_TOOLS: u32 = 96;

/// Replace a file with `new`, but only if it still holds `old` (what the owner started
/// editing). A copy of the old file is kept next to it, named after the file and the
/// machine's clock ("compose.yml.20261117-140312.bak"), and printed as "@@backup
/// <path>". The file is written in place so its owner and permissions stay as they were.
pub fn save_file(path: &str, old: &str, new: &str) -> String {
    let p = quote(path);
    script(&format!(
        "command -v base64 >/dev/null 2>&1 && command -v cmp >/dev/null 2>&1 || exit {SAVE_NO_TOOLS}\n\
         printf '%s' {old} | base64 -d | cmp -s - {p} || exit {SAVE_CHANGED}\n\
         b={p}.$(date +%Y%m%d-%H%M%S).bak\n\
         cp -p -- {p} \"$b\" || exit {SAVE_NO_BACKUP}\n\
         printf '%s' {new} | base64 -d > {p} || exit {SAVE_NO_WRITE}\n\
         echo \"@@backup $b\"\n",
        old = quote(&BASE64.encode(old.as_bytes())),
        new = quote(&BASE64.encode(new.as_bytes())),
    ))
}

/// Remove what the owner picked from a cleanup preview. Docker itself refuses to
/// remove a container that is running, an image a container uses, or a volume in
/// use, so anything that came into use since the preview is left alone. Each line
/// of output starts with "@@ok" or "@@fail" and the item.
pub fn cleanup(containers: &[String], images: &[String], volumes: &[String], build_cache: bool) -> String {
    let mut body = String::new();
    let mut each = |kind: &str, cmd: &str, items: &[String]| {
        for item in items {
            let q = quote(item);
            body += &format!(
                "if out=$({cmd} {q} </dev/null 2>&1); then echo \"@@ok {kind} \"{q}; \
                 else echo \"@@fail {kind} \"{q}\" $(echo \"$out\" | tail -n 1)\"; fi\n"
            );
        }
    };
    each("container", "docker rm", containers);
    each("image", "docker rmi", images);
    each("volume", "docker volume rm", volumes);
    if build_cache {
        body += "if out=$(docker builder prune -af </dev/null 2>&1); then echo \"@@ok cache all\"; \
                 else echo \"@@fail cache all $(echo \"$out\" | tail -n 1)\"; fi\n";
    }
    script(&body)
}

/// The cleanup preview: everything Docker keeps, with sizes.
pub fn disk_usage() -> String {
    script("docker system df -v --format '{{json .}}' </dev/null\n")
}

/// The command that follows a container's log, run with a terminal so it stops when
/// the channel closes. Runs through the login shell, so it is kept to plain words:
/// `id` must already be checked to be a container id.
pub fn follow_logs(id: &str, tail: Option<u32>) -> String {
    let tail = tail.map_or_else(|| "all".to_string(), |n| n.to_string());
    format!("docker logs --follow --timestamps --tail {tail} {id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::process::Command;

    /// Runs a script with the real `sh`, as the machine would.
    #[cfg(unix)]
    fn sh(script: &str, dir: &std::path::Path) -> (i32, String) {
        use std::io::Write;
        let mut child = Command::new("sh")
            .arg("-s")
            .current_dir(dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned())
    }

    #[cfg(unix)]
    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("home-cc-docker-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // Runs the script with this machine's own sh.
    #[cfg(unix)]
    #[test]
    fn quoting_survives_anything() {
        let dir = temp_dir("quote");
        for nasty in ["plain", "it's", "a b", "$(touch pwned)", "`touch pwned`", "x'; touch pwned; '", "\"\\n", ""] {
            let (code, out) = sh(&format!("printf '%s' {}", quote(nasty)), &dir);
            assert_eq!((code, out.as_str()), (0, nasty));
        }
        assert!(!dir.join("pwned").exists());
    }

    #[test]
    fn stack_commands_use_the_stack_folder_name_and_files() {
        let stack = StackPlace {
            project: "media".into(),
            working_dir: Some("/srv/my media".into()),
            config_files: vec!["/srv/my media/compose.yml".into(), "/srv/my media/override.yml".into()],
        };
        let up = stack_action(Compose::Plugin, &stack, StackAction::Up).unwrap();
        assert!(up.contains("cd '/srv/my media' 2>/dev/null ||"));
        assert!(up.contains(
            "docker compose -p 'media' -f '/srv/my media/compose.yml' -f '/srv/my media/override.yml' up -d </dev/null 2>&1"
        ));
        let pull = stack_action(Compose::Standalone, &stack, StackAction::Pull).unwrap();
        assert!(pull.contains("docker-compose -p 'media'"));
        assert!(pull.contains(" pull </dev/null 2>&1 && docker-compose"), "{pull}");
        assert!(pull.ends_with(" up -d </dev/null 2>&1\n"));
        assert_eq!(stack_action(Compose::None, &stack, StackAction::Up), None);
    }

    // Runs the script with this machine's own sh.
    #[cfg(unix)]
    #[test]
    fn a_stack_whose_folder_is_gone_is_not_run_elsewhere() {
        let dir = temp_dir("gone");
        let stack = StackPlace { project: "x".into(), working_dir: Some(dir.join("missing").display().to_string()), config_files: vec![] };
        let script = stack_action(Compose::Plugin, &stack, StackAction::Down).unwrap();
        let (code, out) = sh(&script, &dir);
        assert_eq!(code, 90);
        assert!(out.contains("is no longer on this machine"));
    }

    // Runs the script with this machine's own sh.
    #[cfg(unix)]
    #[test]
    fn saving_keeps_a_backup_and_refuses_a_file_changed_meanwhile() {
        let dir = temp_dir("save");
        let file = dir.join("my compose.yml");
        std::fs::write(&file, "services:\n  web: {}\n").unwrap();
        let p = file.display().to_string();
        let files = || std::fs::read_dir(&dir).unwrap().count();

        // Someone else changed it after it was opened.
        let (code, _) = sh(&save_file(&p, "something else\n", "new"), &dir);
        assert_eq!(code, SAVE_CHANGED as i32);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "services:\n  web: {}\n");
        assert_eq!(files(), 1, "no backup either");

        let new = "services:\n  web:\n    image: 'nginx'  # it's $HOME `x`\n";
        let (code, out) = sh(&save_file(&p, "services:\n  web: {}\n", new), &dir);
        assert_eq!(code, 0);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), new);
        let backup = out.trim().strip_prefix("@@backup ").expect("the backup is named");
        assert!(backup.starts_with(&format!("{p}.20")) && backup.ends_with(".bak"), "{backup}");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "services:\n  web: {}\n");
    }

    // Runs the script with this machine's own sh.
    #[cfg(unix)]
    #[test]
    fn reading_a_file() {
        let dir = temp_dir("read");
        let file = dir.join("it's here.yml");
        std::fs::write(&file, "a: 1\n").unwrap();
        let (code, out) = sh(&read_file(&file.display().to_string()), &dir);
        assert_eq!((code, out.as_str()), (0, "a: 1\n"));
        let (code, _) = sh(&read_file(&dir.join("nope").display().to_string()), &dir);
        assert_eq!(code, 92);
        std::fs::write(&file, vec![b'x'; MAX_FILE + 1]).unwrap();
        let (code, _) = sh(&read_file(&file.display().to_string()), &dir);
        assert_eq!(code, 91);
    }

    // Runs the script with this machine's own sh.
    #[cfg(unix)]
    #[test]
    fn stack_files_are_found_two_folders_deep() {
        let dir = temp_dir("files");
        for f in ["compose.yml", ".env", "web/Dockerfile", "web/deep/Dockerfile", "notes.txt", "docker-compose.override.yaml"] {
            let path = dir.join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        let (code, out) = sh(&list_stack_files(&dir.display().to_string()), &dir);
        assert_eq!(code, 0);
        let mut found: Vec<&str> = out.lines().collect();
        found.sort();
        assert_eq!(found, ["./.env", "./compose.yml", "./docker-compose.override.yaml", "./web/Dockerfile"]);
    }

    // Runs the script with this machine's own sh.
    #[cfg(unix)]
    #[test]
    fn cleanup_reports_each_item() {
        let dir = temp_dir("cleanup");
        // A stand-in docker that refuses one image.
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let fake = bin.join("docker");
        std::fs::write(&fake, "#!/bin/sh\nif [ \"$2\" = \"busy:1\" ]; then echo 'Error: image is in use'; exit 1; fi\necho removed\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let script = cleanup(&["abc123abc123".into()], &["busy:1".into(), "old:2".into()], &[], true);
        let script = format!("PATH={}:$PATH\n{script}", bin.display()).replace(PREAMBLE, "");
        let (_, out) = sh(&script, &dir);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines,
            [
                "@@ok container abc123abc123",
                "@@fail image busy:1 Error: image is in use",
                "@@ok image old:2",
                "@@ok cache all"
            ]
        );
    }

    #[test]
    fn logs_command_is_plain_words() {
        assert_eq!(follow_logs("abc123abc123", Some(500)), "docker logs --follow --timestamps --tail 500 abc123abc123");
        assert_eq!(follow_logs("abc123abc123", None), "docker logs --follow --timestamps --tail all abc123abc123");
    }
}
