//! Image updates (spec 5.5): does the registry have a newer image for the tag a container runs? Found out
//! without pulling anything, on the Docker host itself, so the host's own network and registry logins apply.
//!
//! A check goes in two steps over the host's SSH connection (scripts.rs builds both):
//! 1. What the containers run: each local image's RepoDigests, the digests the registry gave it when it was
//!    pulled. An image with none was built (or loaded) on the host, so there is nothing to compare.
//! 2. What each tag points at in its registry now, asked once per image, however many containers use it:
//!    - a HEAD request with curl, as `docker pull` makes first, with an anonymous token when the registry
//!      asks for one. Docker Hub doesn't count HEAD requests against its pull limits;
//!    - when that gives nothing (no curl, a private image, an odd registry), `docker buildx imagetools
//!      inspect`, which uses the host's logins;
//!    - on a Docker without buildx, `docker manifest inspect -v`.
//!
//! An image made for several kinds of machine is an index that lists one image per kind. `docker pull` writes
//! the index's digest into RepoDigests, and the HEAD request and buildx give the index's digest too, so the
//! two compare directly. `docker manifest inspect -v` only lists the digests of the images inside the index;
//! when none of those matches either, the answer is "can't tell" rather than a guess.

use super::parse::{is_image_id, is_image_ref, text};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// Most images one check asks the registries about.
pub const MAX_IMAGES: usize = 100;

/// What is known about a container's image compared with its registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateState {
    UpToDate,
    /// The registry has a different image for the tag now.
    Available,
    /// The image didn't come from a registry under this name: built or loaded on the host.
    BuiltHere,
    /// Named by digest ("…@sha256:…"), so it can't change.
    Pinned,
    /// The registry wants a login the host doesn't have.
    NeedsLogin,
    /// The registry couldn't be reached from the host.
    Unreachable,
    /// The registry doesn't have this tag (any more).
    NotFound,
    /// The registry said to slow down (Docker Hub's pull limit).
    RateLimited,
    /// Anything else; the message says what.
    Failed,
}

/// One image a container runs, compared with its registry. Keyed by the name the container gives (`image`)
/// and the local image it runs (`image_id`): containers that share both share the answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageUpdate {
    pub image: String,
    pub image_id: String,
    pub state: UpdateState,
    /// The digest the running image came from.
    pub current: Option<String>,
    /// The digest the registry has for the tag now.
    pub latest: Option<String>,
    /// The newer image is on the host already (pulled), but the container still runs the old one.
    pub downloaded: bool,
    /// What went wrong, for the states that aren't an answer.
    pub message: Option<String>,
}

/// A host's last check.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub device_id: i64,
    /// Unix seconds of the last check of every image. None while only some were checked.
    pub checked_at: Option<i64>,
    pub images: Vec<ImageUpdate>,
}

/// An image name taken apart the way Docker does it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// "docker.io", "ghcr.io", "registry.home.lan:5000".
    pub registry: String,
    /// "library/alpine", "linuxserver/sonarr".
    pub repo: String,
    pub tag: String,
    /// Set when the name pins a digest ("…@sha256:…").
    pub digest: Option<String>,
}

impl ImageRef {
    /// Where the registry's API answers.
    pub fn api_host(&self) -> &str {
        if self.registry == "docker.io" { "registry-1.docker.io" } else { &self.registry }
    }
}

fn is_digest(d: &str) -> bool {
    is_image_id(d)
}

/// "jellyfin/jellyfin:10.10" → docker.io, jellyfin/jellyfin, 10.10. "alpine" → docker.io, library/alpine,
/// latest. "registry.home.lan:5000/app" → that registry, app, latest. None for anything Docker wouldn't take
/// or home-cc wouldn't pass on.
pub fn image_ref(r: &str) -> Option<ImageRef> {
    if !is_image_ref(r) || r.starts_with("sha256:") {
        return None;
    }
    let (name, digest) = match r.split_once('@') {
        Some((name, d)) if is_digest(d) => (name, Some(d.to_ascii_lowercase())),
        Some(_) => return None,
        None => (r, None),
    };
    let (name, tag) = match name.rsplit_once(':') {
        Some((n, t)) if !t.contains('/') => (n, t),
        _ => (name, "latest"),
    };
    if name.is_empty() || tag.is_empty() || tag.contains('@') {
        return None;
    }
    let (registry, repo) = match name.split_once('/') {
        Some((first, rest)) if first.contains(['.', ':']) || first == "localhost" => (first.to_ascii_lowercase(), rest),
        _ => ("docker.io".to_string(), name),
    };
    let registry = if matches!(registry.as_str(), "index.docker.io" | "registry-1.docker.io") { "docker.io".to_string() } else { registry };
    if repo.is_empty() || repo.split('/').any(str::is_empty) || repo.contains(':') {
        return None;
    }
    let repo = if registry == "docker.io" && !repo.contains('/') { format!("library/{repo}") } else { repo.to_string() };
    Some(ImageRef { registry, repo, tag: tag.to_string(), digest })
}

/// A local image: its id and the digests the registries gave it ("name@sha256:…").
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocalImage {
    pub id: String,
    pub repo_digests: Vec<String>,
}

impl LocalImage {
    /// The digests it has from this image's repository.
    fn digests_for(&self, r: &ImageRef) -> Vec<String> {
        self.repo_digests
            .iter()
            .filter_map(|rd| {
                let (name, digest) = rd.split_once('@')?;
                let theirs = image_ref(name)?;
                (theirs.registry == r.registry && theirs.repo == r.repo && is_digest(digest)).then(|| digest.to_ascii_lowercase())
            })
            .collect()
    }
}

/// What step 1 printed: the local images by id, and what each tag names locally now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Local {
    pub by_id: HashMap<String, LocalImage>,
    pub tags: HashMap<String, LocalImage>,
}

/// `"sha256:…" ["name@sha256:…"]`, or `null` for no digests.
fn local_line(line: &str) -> Option<LocalImage> {
    let (id, digests) = line.trim().split_once(' ')?;
    let id: String = serde_json::from_str(id).ok()?;
    if !is_image_id(&id) {
        return None;
    }
    let repo_digests = match serde_json::from_str::<Value>(digests.trim()).ok()? {
        Value::Array(list) => list.iter().filter_map(Value::as_str).filter(|s| is_image_ref(s)).map(str::to_string).take(50).collect(),
        Value::Null => Vec::new(),
        _ => return None,
    };
    Some(LocalImage { id: id.to_ascii_lowercase(), repo_digests })
}

/// Reads what [`super::scripts::local_images`] prints:
/// ```text
/// @@ids
/// "sha256:…" ["jellyfin/jellyfin@sha256:…"]      one line per image a container runs
/// @@tag jellyfin/jellyfin:10.10
/// "sha256:…" [...]                                 what the tag names on the host now, if anything
/// ```
pub fn local_images(output: &str) -> Local {
    let mut local = Local::default();
    let mut tag: Option<String> = None;
    for line in output.lines() {
        if line.trim() == "@@ids" {
            tag = None;
        } else if let Some(r) = line.strip_prefix("@@tag ") {
            tag = Some(r.trim().to_string()).filter(|r| is_image_ref(r));
        } else if let Some(image) = local_line(line) {
            match tag.take() {
                Some(r) => {
                    local.tags.insert(r, image);
                }
                None => {
                    local.by_id.insert(image.id.clone(), image);
                }
            }
        }
    }
    local
}

/// What a registry said about one tag.
#[derive(Debug, Clone, PartialEq)]
pub enum Remote {
    /// The digest the tag points at now.
    Digest(String),
    /// Only the digests of the images inside its index are known (`docker manifest inspect -v`).
    Platforms(Vec<String>),
    Failed(UpdateState, String),
}

/// Reads what [`super::scripts::remote_digests`] prints, one block per image:
/// ```text
/// @@remote jellyfin/jellyfin:10.10
/// @@via head|buildx|manifest|none <exit code>
/// | what that command printed, each line after "| "
/// ```
/// An image whose block is missing (the output was cut short) is left out.
pub fn remote_digests(output: &str) -> HashMap<String, Remote> {
    let mut found = HashMap::new();
    let mut current: Option<(String, String, i32, Vec<String>)> = None;
    let finish = |c: Option<(String, String, i32, Vec<String>)>, found: &mut HashMap<String, Remote>| {
        if let Some((image, via, code, body)) = c {
            if !via.is_empty() {
                found.insert(image, remote(&via, code, &body.join("\n")));
            }
        }
    };
    for line in output.lines() {
        if let Some(image) = line.strip_prefix("@@remote ") {
            finish(current.take(), &mut found);
            let image = image.trim();
            if is_image_ref(image) {
                current = Some((image.to_string(), String::new(), -1, Vec::new()));
            }
        } else if let Some(rest) = line.strip_prefix("@@via ") {
            if let Some(c) = current.as_mut() {
                let (via, code) = rest.trim().split_once(' ').unwrap_or((rest.trim(), ""));
                c.1 = via.to_string();
                c.2 = code.trim().parse().unwrap_or(-1);
            }
        } else if let Some(body) = line.strip_prefix("| ").or_else(|| (line == "|").then_some("")) {
            if let Some(c) = current.as_mut() {
                c.3.push(body.to_string());
            }
        }
    }
    finish(current.take(), &mut found);
    found
}

/// One image's answer, from the way it was asked.
pub fn remote(via: &str, code: i32, body: &str) -> Remote {
    match via {
        "head" | "buildx" if code == 0 => match top_digest(body) {
            Some(d) => Remote::Digest(d),
            None => Remote::Failed(UpdateState::Failed, "The registry's answer had no digest in it.".into()),
        },
        "manifest" if code == 0 => manifest_digests(body)
            .unwrap_or_else(|| Remote::Failed(UpdateState::Failed, "docker manifest inspect gave an answer home-cc can't read.".into())),
        "none" => Remote::Failed(
            UpdateState::Failed,
            "This host has neither curl nor docker buildx nor docker manifest, so its images can't be checked. Installing \
             curl or the docker-buildx-plugin package there fixes that."
                .into(),
        ),
        _ => {
            let (state, message) = refusal(body);
            Remote::Failed(state, message)
        }
    }
}

/// The first "Digest: sha256:…" line, which is the tag's own (an index lists its images' digests below it).
fn top_digest(body: &str) -> Option<String> {
    body.lines()
        .map(str::trim)
        .take_while(|l| !l.starts_with("Manifests:"))
        .find_map(|l| l.strip_prefix("Digest:"))
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| is_digest(d))
}

/// `docker manifest inspect -v`: one object (Ref, Descriptor, …) for a single image, a list of them for an index.
fn manifest_digests(body: &str) -> Option<Remote> {
    let v: Value = serde_json::from_str(body.trim()).ok()?;
    let digest = |o: &Value| o.get("Descriptor")?.get("digest")?.as_str().map(str::to_ascii_lowercase).filter(|d| is_digest(d));
    match &v {
        Value::Object(_) => digest(&v).map(Remote::Digest),
        Value::Array(list) => {
            let all: Vec<String> = list.iter().filter_map(digest).collect();
            (!all.is_empty()).then_some(Remote::Platforms(all))
        }
        _ => None,
    }
}

/// Why a registry said no, from what docker or the registry printed. Wording differs between versions and
/// registries, so several forms are checked.
pub fn refusal(said: &str) -> (UpdateState, String) {
    let first = said.lines().map(str::trim).find(|l| !l.is_empty()).map(text).unwrap_or_default();
    let message = if first.is_empty() { "The registry gave no answer.".to_string() } else { first };
    let s = said.to_ascii_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| s.contains(w));
    let state = if any(&["toomanyrequests", "too many requests", "rate limit"]) {
        UpdateState::RateLimited
    } else if any(&["unauthorized", "authentication required", "denied", "no basic auth credentials", "401"]) {
        UpdateState::NeedsLogin
    } else if any(&["manifest unknown", "not found", "name unknown", "no such manifest"]) {
        UpdateState::NotFound
    } else if any(&[
        "no such host",
        "dial tcp",
        "i/o timeout",
        "connection refused",
        "network is unreachable",
        "tls handshake timeout",
        "server misbehaving",
        "context deadline exceeded",
        "temporary failure in name resolution",
        "connection reset",
        "no route to host",
    ]) {
        UpdateState::Unreachable
    } else {
        UpdateState::Failed
    };
    (state, message)
}

/// Whether the registry is worth asking about this image at all: it came from one under this name.
pub fn from_registry(r: &ImageRef, runs: &[&str], local: &Local, image: &str) -> bool {
    r.digest.is_none()
        && (runs.iter().filter_map(|id| local.by_id.get(*id)).any(|l| !l.digests_for(r).is_empty())
            || local.tags.get(image).is_some_and(|l| !l.digests_for(r).is_empty()))
}

/// A container's image (`image`, run as `image_id`) compared with what its registry has. None while the
/// registry wasn't asked yet.
pub fn compare(image: &str, image_id: &str, local: &Local, remote: Option<&Remote>) -> Option<ImageUpdate> {
    let answer = |state, current: Option<String>, latest: Option<String>, downloaded, message: Option<String>| ImageUpdate {
        image: image.to_string(),
        image_id: image_id.to_string(),
        state,
        current,
        latest,
        downloaded,
        message,
    };
    let Some(r) = image_ref(image) else {
        return Some(answer(UpdateState::Failed, None, None, false, Some("home-cc can't read this image's name.".into())));
    };
    if let Some(d) = &r.digest {
        return Some(answer(UpdateState::Pinned, Some(d.clone()), None, false, None));
    }
    let Some(mine) = local.by_id.get(image_id) else {
        return Some(answer(UpdateState::Failed, None, None, false, Some("The host didn't describe this image.".into())));
    };
    let have = mine.digests_for(&r);
    if have.is_empty() {
        return Some(answer(UpdateState::BuiltHere, None, None, false, None));
    }
    let current = have.first().cloned();
    match remote? {
        Remote::Failed(state, message) => Some(answer(*state, current, None, false, Some(message.clone()))),
        Remote::Digest(latest) if have.contains(latest) => Some(answer(UpdateState::UpToDate, current, Some(latest.clone()), false, None)),
        Remote::Digest(latest) => {
            let downloaded = local.tags.get(image).is_some_and(|t| t.id != mine.id && t.digests_for(&r).contains(latest));
            Some(answer(UpdateState::Available, current, Some(latest.clone()), downloaded, None))
        }
        Remote::Platforms(list) if list.iter().any(|d| have.contains(d)) => {
            Some(answer(UpdateState::UpToDate, current, None, false, None))
        }
        Remote::Platforms(_) => Some(answer(
            UpdateState::Failed,
            current,
            None,
            false,
            Some(
                "Can't tell: this image is made for several kinds of machine, and without docker buildx or curl on the host \
                 home-cc can't read the digest to compare. Installing curl or the docker-buildx-plugin package there fixes that."
                    .into(),
            ),
        )),
    }
}

/// Every (image, local image) pair the containers run, compared, in image order.
pub fn compare_all(pairs: &BTreeSet<(String, String)>, local: &Local, remote: &HashMap<String, Remote>) -> Vec<ImageUpdate> {
    pairs.iter().filter_map(|(image, id)| compare(image, id, local, remote.get(image))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const NEW: &str = "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    const AMD: &str = "sha256:3333333333333333333333333333333333333333333333333333333333333333";
    const ID1: &str = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const ID2: &str = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn image_names_are_taken_apart_as_docker_does() {
        let r = |s: &str| image_ref(s).map(|r| (r.registry, r.repo, r.tag, r.digest));
        let hub = |repo: &str, tag: &str| Some(("docker.io".to_string(), repo.to_string(), tag.to_string(), None));
        assert_eq!(r("alpine"), hub("library/alpine", "latest"));
        assert_eq!(r("alpine:3.20"), hub("library/alpine", "3.20"));
        assert_eq!(r("jellyfin/jellyfin:10.10"), hub("jellyfin/jellyfin", "10.10"));
        assert_eq!(r("docker.io/library/redis:7-alpine"), hub("library/redis", "7-alpine"));
        assert_eq!(r("index.docker.io/koenkk/zigbee2mqtt"), hub("koenkk/zigbee2mqtt", "latest"));
        assert_eq!(
            r("registry.home.lan:5000/sam/backup:1.4"),
            Some(("registry.home.lan:5000".into(), "sam/backup".into(), "1.4".into(), None))
        );
        assert_eq!(r("localhost/app"), Some(("localhost".into(), "app".into(), "latest".into(), None)));
        assert_eq!(r(&format!("redis:7@{NEW}")), Some(("docker.io".into(), "library/redis".into(), "7".into(), Some(NEW.into()))));
        assert_eq!(image_ref("ghcr.io/immich-app/immich-server:release").unwrap().api_host(), "ghcr.io");
        assert_eq!(image_ref("alpine").unwrap().api_host(), "registry-1.docker.io");
        for bad in ["", "sha256:abc", "a b", "-x", "x@sha256:short", "x//y", "a:b:c", "$(touch x)"] {
            assert_eq!(image_ref(bad), None, "{bad}");
        }
    }

    fn local_sample() -> String {
        format!(
            "warning: something\n@@ids\n\"{ID1}\" [\"lscr.io/linuxserver/sonarr@{OLD}\"]\n\"{ID2}\" null\n\
             @@tag lscr.io/linuxserver/sonarr:latest\n\"{ID2}\" [\"lscr.io/linuxserver/sonarr@{NEW}\"]\n\
             @@tag gone:1\n@@tag homepage:dev\n\"{ID2}\" []\n"
        )
    }

    #[test]
    fn local_images_and_what_each_tag_names() {
        let l = local_images(&local_sample());
        assert_eq!(l.by_id[ID1].repo_digests, [format!("lscr.io/linuxserver/sonarr@{OLD}")]);
        assert!(l.by_id[ID2].repo_digests.is_empty(), "null is no digests");
        assert_eq!(l.tags["lscr.io/linuxserver/sonarr:latest"].id, ID2);
        assert!(!l.tags.contains_key("gone:1"), "a tag not on the host");
        assert!(l.tags["homepage:dev"].repo_digests.is_empty());
        assert_eq!(l.by_id.len(), 2);
    }

    /// Shaped like `docker buildx imagetools inspect` of an image made for several kinds of machine.
    fn buildx_index() -> String {
        format!(
            "Name:      docker.io/jellyfin/jellyfin:10.10\nMediaType: application/vnd.oci.image.index.v1+json\nDigest:    {NEW}\n\n\
             Manifests:\n  Name:        docker.io/jellyfin/jellyfin:10.10@{AMD}\n  MediaType:   application/vnd.oci.image.manifest.v1+json\n\
             \x20 Digest:      {AMD}\n  Platform:    linux/amd64\n"
        )
    }

    #[test]
    fn registry_answers_by_each_way_of_asking() {
        assert_eq!(remote("head", 0, &format!("Digest: {NEW}")), Remote::Digest(NEW.into()));
        assert_eq!(remote("buildx", 0, &buildx_index()), Remote::Digest(NEW.into()), "the index's digest, not one inside it");
        let single = format!("Name:      docker.io/library/alpine:3.20\nMediaType: application/vnd.docker.distribution.manifest.v2+json\nDigest:    {OLD}\n");
        assert_eq!(remote("buildx", 0, &single), Remote::Digest(OLD.into()));

        let one = format!(r#"{{"Ref":"docker.io/library/alpine:3.20","Descriptor":{{"mediaType":"application/vnd.docker.distribution.manifest.v2+json","digest":"{OLD}","size":528}},"SchemaV2Manifest":{{"layers":[{{"digest":"{AMD}"}}]}}}}"#);
        assert_eq!(remote("manifest", 0, &one), Remote::Digest(OLD.into()));
        let index = format!(
            "[\n  {{\"Ref\":\"x@{AMD}\",\"Descriptor\":{{\"digest\":\"{AMD}\",\"platform\":{{\"architecture\":\"amd64\"}}}}}},\n  {{\"Ref\":\"x@{OLD}\",\"Descriptor\":{{\"digest\":\"{OLD}\"}}}}\n]"
        );
        assert_eq!(remote("manifest", 0, &index), Remote::Platforms(vec![AMD.into(), OLD.into()]));
        assert!(matches!(remote("manifest", 0, "not json"), Remote::Failed(UpdateState::Failed, _)));
        assert!(matches!(remote("buildx", 0, "Name: x\n"), Remote::Failed(UpdateState::Failed, _)));
        assert!(matches!(remote("none", 1, ""), Remote::Failed(UpdateState::Failed, m) if m.contains("docker-buildx-plugin")));
    }

    #[test]
    fn registry_refusals_in_their_usual_wordings() {
        let kind = |s: &str| refusal(s).0;
        assert_eq!(kind("ERROR: unexpected status from HEAD request to https://registry.home.lan/v2/sam/backup/manifests/1.4: 401 Unauthorized"), UpdateState::NeedsLogin);
        assert_eq!(kind("unauthorized: authentication required"), UpdateState::NeedsLogin);
        assert_eq!(kind("errors:\ndenied: requested access to the resource is denied\n"), UpdateState::NeedsLogin);
        assert_eq!(kind("no basic auth credentials"), UpdateState::NeedsLogin);
        assert_eq!(kind("manifest unknown: manifest unknown"), UpdateState::NotFound);
        assert_eq!(kind("ERROR: docker.io/library/alpine:9.99: not found"), UpdateState::NotFound);
        assert_eq!(kind("no such manifest: docker.io/library/alpine:9.99"), UpdateState::NotFound);
        assert_eq!(kind("ERROR: failed to do request: Head \"https://ghcr.io/v2/x/manifests/y\": dial tcp: lookup ghcr.io on 127.0.0.53:53: no such host"), UpdateState::Unreachable);
        assert_eq!(kind("Get \"https://registry-1.docker.io/v2/\": net/http: TLS handshake timeout"), UpdateState::Unreachable);
        assert_eq!(kind("context deadline exceeded"), UpdateState::Unreachable);
        assert_eq!(kind("toomanyrequests: You have reached your unauthenticated pull rate limit."), UpdateState::RateLimited);
        assert_eq!(kind("something new"), UpdateState::Failed);
        assert_eq!(refusal("\n\n  first line\nsecond").1, "first line");
        assert_eq!(refusal("").1, "The registry gave no answer.");
    }

    #[test]
    fn remote_output_is_read_per_image() {
        let out = format!(
            "@@remote jellyfin/jellyfin:10.10\n@@via head 0\n| Digest: {NEW}\n\
             @@remote registry.home.lan/sam/backup:1.4\n@@via buildx 1\n| ERROR: 401 Unauthorized\n\
             @@remote alpine:3.20\n@@via manifest 0\n| {{\"Descriptor\":{{\"digest\":\"{OLD}\"}}}}\n\
             @@remote bad name\n@@via head 0\n| Digest: {NEW}\n\
             @@remote cut:short\n"
        );
        let found = remote_digests(&out);
        assert_eq!(found["jellyfin/jellyfin:10.10"], Remote::Digest(NEW.into()));
        assert!(matches!(&found["registry.home.lan/sam/backup:1.4"], Remote::Failed(UpdateState::NeedsLogin, m) if m.contains("401")));
        assert_eq!(found["alpine:3.20"], Remote::Digest(OLD.into()));
        assert!(!found.contains_key("bad name"), "a name that isn't safe is skipped");
        assert!(!found.contains_key("cut:short"), "an image the output didn't get to stays unchecked");
        // A line in a registry's message that looks like one of ours is only ever body text.
        let sneaky = "@@remote a:1\n@@via buildx 1\n| @@remote b:1\n| denied\n";
        let found = remote_digests(sneaky);
        assert_eq!(found.len(), 1);
        assert_eq!(found["a:1"], Remote::Failed(UpdateState::NeedsLogin, "@@remote b:1".into()));
    }

    fn local_with(id: &str, digests: &[String]) -> Local {
        let mut l = Local::default();
        l.by_id.insert(id.into(), LocalImage { id: id.into(), repo_digests: digests.to_vec() });
        l
    }

    #[test]
    fn each_state_is_told_apart() {
        let sonarr = "lscr.io/linuxserver/sonarr:latest";
        let l = local_with(ID1, &[format!("lscr.io/linuxserver/sonarr@{OLD}")]);
        let state = |l: &Local, r: Option<&Remote>| compare(sonarr, ID1, l, r).map(|u| u.state);

        assert_eq!(state(&l, None), None, "not asked yet");
        assert_eq!(state(&l, Some(&Remote::Digest(OLD.into()))), Some(UpdateState::UpToDate));
        let u = compare(sonarr, ID1, &l, Some(&Remote::Digest(NEW.into()))).unwrap();
        assert_eq!((u.state, u.current.as_deref(), u.latest.as_deref(), u.downloaded), (UpdateState::Available, Some(OLD), Some(NEW), false));

        // Pulled already: the tag names the new image, the container still runs the old one.
        let mut pulled = l.clone();
        pulled.tags.insert(sonarr.into(), LocalImage { id: ID2.into(), repo_digests: vec![format!("lscr.io/linuxserver/sonarr@{NEW}")] });
        assert!(compare(sonarr, ID1, &pulled, Some(&Remote::Digest(NEW.into()))).unwrap().downloaded);

        let built = local_with(ID1, &[]);
        assert_eq!(state(&built, Some(&Remote::Digest(NEW.into()))), Some(UpdateState::BuiltHere));
        assert_eq!(state(&built, None), Some(UpdateState::BuiltHere), "known without asking the registry");
        let renamed = local_with(ID1, &[format!("other/name@{OLD}")]);
        assert_eq!(state(&renamed, None), Some(UpdateState::BuiltHere), "came from the registry under another name");

        let pinned = compare(&format!("redis:7@{OLD}"), ID1, &l, None).unwrap();
        assert_eq!((pinned.state, pinned.current.as_deref()), (UpdateState::Pinned, Some(OLD)));

        let login = Remote::Failed(UpdateState::NeedsLogin, "401 Unauthorized".into());
        let u = compare(sonarr, ID1, &l, Some(&login)).unwrap();
        assert_eq!((u.state, u.message.as_deref()), (UpdateState::NeedsLogin, Some("401 Unauthorized")));

        assert_eq!(state(&l, Some(&Remote::Platforms(vec![AMD.into(), OLD.into()]))), Some(UpdateState::UpToDate));
        assert_eq!(state(&l, Some(&Remote::Platforms(vec![AMD.into()]))), Some(UpdateState::Failed), "can't tell, so no guess");

        assert_eq!(compare(sonarr, ID2, &l, Some(&Remote::Digest(NEW.into()))).unwrap().state, UpdateState::Failed, "no local image");
    }

    #[test]
    fn library_images_match_whatever_way_the_digest_is_written() {
        // The classic image store writes "redis@…"; the containerd one writes "docker.io/library/redis@…".
        for written in ["redis", "docker.io/library/redis", "library/redis"] {
            let l = local_with(ID1, &[format!("{written}@{OLD}")]);
            assert_eq!(compare("redis:7-alpine", ID1, &l, Some(&Remote::Digest(OLD.into()))).unwrap().state, UpdateState::UpToDate, "{written}");
        }
    }

    #[test]
    fn only_images_from_a_registry_are_asked_about() {
        let l = local_images(&local_sample());
        let sonarr = image_ref("lscr.io/linuxserver/sonarr:latest").unwrap();
        assert!(from_registry(&sonarr, &[ID1], &l, "lscr.io/linuxserver/sonarr:latest"));
        let built = image_ref("homepage:dev").unwrap();
        assert!(!from_registry(&built, &[ID2], &l, "homepage:dev"));
        let pinned = image_ref(&format!("lscr.io/linuxserver/sonarr@{OLD}")).unwrap();
        assert!(!from_registry(&pinned, &[ID1], &l, "x"));
    }

    #[test]
    fn containers_sharing_an_image_share_one_answer() {
        let l = local_with(ID1, &[format!("ghcr.io/immich-app/immich-server@{OLD}")]);
        let image = "ghcr.io/immich-app/immich-server:release".to_string();
        let pairs: BTreeSet<(String, String)> = [(image.clone(), ID1.to_string()), (image.clone(), ID1.to_string())].into();
        let remote = HashMap::from([(image, Remote::Digest(NEW.into()))]);
        let all = compare_all(&pairs, &l, &remote);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].state, UpdateState::Available);
    }
}
