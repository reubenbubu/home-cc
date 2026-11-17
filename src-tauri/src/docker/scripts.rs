//! The small `sh` scripts run on a Docker host. Each one is sent on stdin to `sh -s`,
//! as the stats script is, so it works whatever the user's login shell is (bash, zsh,
//! fish) and nothing has to survive two rounds of quoting. Every value from outside
//! (a path, a stack name, an image) goes in through [`quote`], and every docker
//! command reads from /dev/null so it can't swallow the rest of the script.

use super::parse::{Compose, INSPECT_FORMAT};
use data_encoding::BASE64;

/// Put first in every script. The usual places for docker (also the snap one) even
/// when the SSH login didn't set up a full PATH.
/// The same list is DOCKER_PATH in src/lib/docker/logic.ts, for the terminal into a
/// container; a test below checks that they agree.
const PREAMBLE: &str = "PATH=\"$PATH:/usr/local/bin:/usr/bin:/bin:/snap/bin\"; export PATH\n";

/// The same places for a single command that runs through the login shell rather than
/// `sh -s`. `env`, because `PATH=… docker` isn't understood by csh or tcsh.
const DOCKER_ENV: &str = "env PATH=\"$PATH:/usr/local/bin:/usr/bin:/bin:/snap/bin\" ";

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

/// A compose service name: letters, digits and "._-", as compose itself allows.
pub fn is_service_name(s: &str) -> bool {
    s.len() <= 255
        && s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// Update some services of a stack: pull their images, then recreate only those services (`--no-deps`, so
/// nothing else in the stack restarts). None when the machine has no compose command. The services must
/// already be checked with [`is_service_name`]; they are quoted as well.
pub fn stack_update(compose: Compose, stack: &StackPlace, services: &[String]) -> Option<String> {
    let c = compose_command(compose, stack)?;
    let names = services.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ");
    Some(script(&format!(
        "{}{c} pull {names} </dev/null 2>&1 && {c} up -d --no-deps {names} </dev/null 2>&1\n",
        cd(stack)
    )))
}

/// Pull one image, for a container that isn't in a compose stack. The container itself is left alone.
pub fn pull_image(image: &str) -> String {
    script(&format!("docker pull {} </dev/null 2>&1\n", quote(image)))
}

/// What a local image is: its id and the digests registries gave it.
const LOCAL_FORMAT: &str = "{{json .Id}} {{json .RepoDigests}}";

/// The first step of an update check (see updates.rs): the images the containers run, by id, then what each
/// image name names on the host now. Ids and names must already be checked to be safe; they are quoted too.
pub fn local_images<'a>(ids: impl IntoIterator<Item = &'a String>, images: impl IntoIterator<Item = &'a String>) -> String {
    let fmt = quote(LOCAL_FORMAT);
    let ids: Vec<String> = ids.into_iter().map(|i| quote(i)).collect();
    let mut body = String::from("echo '@@ids'\n");
    if !ids.is_empty() {
        body += &format!("docker image inspect --format {fmt} {} </dev/null 2>/dev/null\n", ids.join(" "));
    }
    for image in images {
        let q = quote(image);
        body += &format!("echo '@@tag '{q}\ndocker image inspect --format {fmt} {q} </dev/null 2>/dev/null\n");
    }
    script(&body)
}

/// Asks a registry what a tag points at, with HEAD requests only (Docker Hub doesn't count those against its
/// pull limits): first without a login; when the registry asks for a token, with an anonymous one from where
/// it says (https only), and again. Prints the digest, or fails. Needs curl.
const HEAD_FUNCTION: &str = r#"hcc_acc='Accept: application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json'
hcc_head() {
  command -v curl >/dev/null 2>&1 || return 1
  u="https://$1/v2/$2/manifests/$3"
  h=$(curl -sS -I -L --max-time 20 -H "$hcc_acc" "$u" </dev/null 2>/dev/null | tr -d '\r')
  a=$(printf '%s\n' "$h" | sed -n 's/^[Ww][Ww][Ww]-[Aa][Uu][Tt][Hh][Ee][Nn][Tt][Ii][Cc][Aa][Tt][Ee]: *[Bb][Ee][Aa][Rr][Ee][Rr]  *//p' | tail -n 1)
  if [ -n "$a" ]; then
    realm=$(printf '%s' "$a" | sed -n 's/.*realm="\([^"]*\)".*/\1/p')
    service=$(printf '%s' "$a" | sed -n 's/.*service="\([^"]*\)".*/\1/p')
    scope=$(printf '%s' "$a" | sed -n 's/.*scope="\([^"]*\)".*/\1/p')
    [ -n "$scope" ] || scope="repository:$2:pull"
    case "$realm" in https://*) ;; *) return 1 ;; esac
    t=$(curl -sS -G --max-time 20 "$realm" --data-urlencode "service=$service" --data-urlencode "scope=$scope" </dev/null 2>/dev/null | sed -n 's/.*"token" *: *"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$t" ] || return 1
    h=$(curl -sS -I -L --max-time 20 -H "$hcc_acc" -H "Authorization: Bearer $t" "$u" </dev/null 2>/dev/null | tr -d '\r')
  fi
  s=$(printf '%s\n' "$h" | sed -n 's/^HTTP\/[0-9.]* \([0-9][0-9]*\).*/\1/p' | tail -n 1)
  d=$(printf '%s\n' "$h" | sed -n 's/^[Dd][Oo][Cc][Kk][Ee][Rr]-[Cc][Oo][Nn][Tt][Ee][Nn][Tt]-[Dd][Ii][Gg][Ee][Ss][Tt]: *\(sha256:[0-9a-f]\{64\}\).*/\1/p' | tail -n 1)
  [ "$s" = 200 ] && [ -n "$d" ] || return 1
  echo "$d"
}
hcc_missing() { printf '%s' "$1" | grep -qi -e 'is not a docker command' -e 'unknown command'; }
hcc_check() {
  echo "@@remote $1"
  if d=$(hcc_head "$2" "$3" "$4"); then echo "@@via head 0"; echo "| Digest: $d"; return; fi
  out=$(docker buildx imagetools inspect "$1" </dev/null 2>&1); rc=$?
  if [ $rc -eq 0 ] || ! hcc_missing "$out"; then echo "@@via buildx $rc"; printf '%s\n' "$out" | head -n 60 | sed 's/^/| /'; return; fi
  out=$(docker manifest inspect -v "$1" </dev/null 2>&1); rc=$?
  if [ $rc -eq 0 ] || ! hcc_missing "$out"; then echo "@@via manifest $rc"; printf '%s\n' "$out" | head -c 100000 | sed 's/^/| /'; return; fi
  echo "@@via none 1"
}
"#;

/// The second step of an update check: what each image's tag points at in its registry now, once per image.
/// See [`super::updates::remote_digests`] for what it prints. The names must already be checked to be safe
/// image names (they are quoted too).
pub fn remote_digests(images: &[(String, super::updates::ImageRef)]) -> String {
    let mut body = String::from(HEAD_FUNCTION);
    for (image, r) in images {
        body += &format!("hcc_check {} {} {} {}\n", quote(image), quote(r.api_host()), quote(&r.repo), quote(&r.tag));
    }
    script(&body)
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
/// `id` must already be checked to be a container id. Docker is looked for where the
/// scripts look for it.
pub fn follow_logs(id: &str, tail: Option<u32>) -> String {
    let tail = tail.map_or_else(|| "all".to_string(), |n| n.to_string());
    format!("{DOCKER_ENV}docker logs --follow --timestamps --tail {tail} {id}")
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
    fn an_update_pulls_and_recreates_only_the_services_named() {
        let stack = StackPlace { project: "media".into(), working_dir: Some("/srv/media".into()), config_files: vec!["/srv/media/compose.yml".into()] };
        let s = stack_update(Compose::Plugin, &stack, &["sonarr".into(), "it's".into()]).unwrap();
        assert!(s.contains("cd '/srv/media' 2>/dev/null ||"));
        let c = "docker compose -p 'media' -f '/srv/media/compose.yml'";
        assert!(s.contains(&format!("{c} pull 'sonarr' 'it'\\''s' </dev/null 2>&1 && {c} up -d --no-deps 'sonarr' 'it'\\''s' </dev/null 2>&1\n")), "{s}");
        assert_eq!(stack_update(Compose::None, &stack, &["sonarr".into()]), None);
        assert!(pull_image("lscr.io/linuxserver/speedtest-tracker:latest").ends_with("docker pull 'lscr.io/linuxserver/speedtest-tracker:latest' </dev/null 2>&1\n"));
        assert!(is_service_name("immich-machine-learning"));
        assert!(is_service_name("web.1_a"));
        for bad in ["", "-d", "a b", "a;b", "$(x)", "ü"] {
            assert!(!is_service_name(bad), "{bad}");
        }
    }

    #[test]
    fn local_images_are_read_by_id_then_by_name() {
        let id = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        let s = local_images([&id], [&"alpine:3.20".to_string()]);
        assert!(s.contains(&format!("docker image inspect --format '{{{{json .Id}}}} {{{{json .RepoDigests}}}}' '{id}' </dev/null")), "{s}");
        assert!(s.contains("echo '@@tag ''alpine:3.20'\ndocker image inspect"));
        let none = local_images([], []);
        assert!(!none.contains("inspect"), "nothing to ask about");
    }

    /// The registry check, run with this machine's own sh against a stand-in curl (a registry that wants an
    /// anonymous token, and a private one) and a stand-in docker (one without buildx).
    #[cfg(unix)]
    #[test]
    fn registry_check_asks_with_head_then_falls_back() {
        use super::super::updates::{self, Remote, UpdateState, image_ref};
        use std::os::unix::fs::PermissionsExt;
        let new = format!("sha256:{}", "2".repeat(64));
        let old = format!("sha256:{}", "1".repeat(64));
        let dir = temp_dir("registry");
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let curl = format!(
            r#"#!/bin/sh
echo "$*" >> "{log}"
auth=""; get=""; url=""
for a in "$@"; do
  case "$a" in
    -G) get=1 ;;
    "Authorization: Bearer tok123") auth=1 ;;
    https://*) url="$a" ;;
  esac
done
case "$url" in *nocurl*) exit 6 ;; esac
if [ -n "$get" ]; then echo '{{"access_token":"wrong","token":"tok123","expires_in":300}}'; exit 0; fi
case "$url" in *private*) printf 'HTTP/1.1 401 Unauthorized\r\nWww-Authenticate: Bearer realm="https://auth.example/token",service="reg"\r\n\r\n'; exit 0 ;; esac
if [ -n "$auth" ]; then printf 'HTTP/2 200 \r\ncontent-type: x\r\ndocker-content-digest: {new}\r\n\r\n'
else printf 'HTTP/1.1 401 Unauthorized\r\nwww-authenticate: Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/alpine:pull"\r\n\r\n'; fi
"#,
            log = dir.join("curl.log").display()
        );
        let docker = format!(
            r#"#!/bin/sh
case "$1 $2 $3" in
  "buildx imagetools inspect") case "$4" in *nobuildx*) echo "docker: 'buildx' is not a docker command."; exit 1 ;; *private*) echo "ERROR: unexpected status from HEAD request: 401 Unauthorized"; exit 1 ;; esac ;;
  "manifest inspect -v") echo '{{"Ref":"x","Descriptor":{{"digest":"{old}"}}}}'; exit 0 ;;
esac
exit 1
"#
        );
        for (name, text) in [("curl", curl), ("docker", docker)] {
            std::fs::write(bin.join(name), text).unwrap();
            std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let images: Vec<(String, updates::ImageRef)> = ["alpine:3.20", "registry.example/private:1", "nocurl.example/nobuildx:1"]
            .iter()
            .map(|i| (i.to_string(), image_ref(i).unwrap()))
            .collect();
        let script = format!("PATH={}:$PATH\n{}", bin.display(), remote_digests(&images));
        let (code, out) = sh(&script, &dir);
        assert_eq!(code, 0, "{out}");
        let found = updates::remote_digests(&out);
        assert_eq!(found["alpine:3.20"], Remote::Digest(new.clone()), "{out}");
        assert!(matches!(&found["registry.example/private:1"], Remote::Failed(UpdateState::NeedsLogin, m) if m.contains("401")), "{out}");
        assert_eq!(found["nocurl.example/nobuildx:1"], Remote::Digest(old), "{out}");
        let log = std::fs::read_to_string(dir.join("curl.log")).unwrap();
        assert!(log.contains("https://registry-1.docker.io/v2/library/alpine/manifests/3.20"), "{log}");
        assert!(log.lines().all(|l| l.contains(" -G ") || l.contains("-I")), "only HEAD requests and the token: {log}");
        assert!(log.contains("scope=repository:library/alpine:pull"), "{log}");
    }

    #[test]
    fn logs_command_is_plain_words() {
        let env = "env PATH=\"$PATH:/usr/local/bin:/usr/bin:/bin:/snap/bin\" ";
        assert_eq!(follow_logs("abc123abc123", Some(500)), format!("{env}docker logs --follow --timestamps --tail 500 abc123abc123"));
        assert_eq!(follow_logs("abc123abc123", None), format!("{env}docker logs --follow --timestamps --tail all abc123abc123"));
    }

    /// The scripts, the log command and the terminal into a container (built in the
    /// front end) all look for docker in the same places, the snap's among them.
    #[test]
    fn docker_is_looked_for_in_the_same_places_everywhere() {
        let path = "/usr/local/bin:/usr/bin:/bin:/snap/bin";
        assert_eq!(PREAMBLE, format!("PATH=\"$PATH:{path}\"; export PATH\n"));
        assert_eq!(DOCKER_ENV, format!("env PATH=\"$PATH:{path}\" "));
        let logic = include_str!("../../../src/lib/docker/logic.ts");
        assert!(
            logic.contains(&format!("export const DOCKER_PATH = \"{path}\";")),
            "DOCKER_PATH in src/lib/docker/logic.ts is not {path}"
        );
    }

    /// A command run through the login shell gets the added places on its PATH.
    #[cfg(unix)]
    #[test]
    fn a_single_command_gets_the_added_places() {
        let out = Command::new("sh")
            .arg("-c")
            .arg(format!("{DOCKER_ENV}sh -c 'echo \"$PATH\"'"))
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "/usr/bin:/bin:/usr/local/bin:/usr/bin:/bin:/snap/bin");
    }
}
