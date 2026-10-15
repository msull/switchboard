//! Updates from GitHub Releases: the newest release of the repository,
//! its bundle fetched to a staging directory, checked against the
//! checksum the release carries and against the signature the running
//! app has, and swapped into place by a helper once the app has quit.
//! The checks are the boundary: a release is installed only when its
//! zip hashes as published and its bundle is signed by the team that
//! signed the running one, so the grants macOS keys on that signature
//! survive the update.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::ports::updates::{Release, UpdateSource};

/// How long the whole download may take.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// How long the API call may take.
const API_TIMEOUT: Duration = Duration::from_secs(30);
/// More than any bundle will be; a reply past it is refused.
const ZIP_LIMIT: u64 = 256 << 20;
const USER_AGENT: &str = concat!("switchboard/", env!("CARGO_PKG_VERSION"));

/// The releases of one GitHub repository.
pub struct GitHubReleases {
    /// `owner/name`.
    repo: String,
}

impl GitHubReleases {
    #[must_use]
    pub fn new(repo: &str) -> Self {
        Self {
            repo: repo.to_owned(),
        }
    }

    /// Where a download is staged: a fixed directory rather than a
    /// temporary one, since the helper copies from it after the app
    /// has exited.
    fn staging() -> PathBuf {
        std::env::temp_dir().join("switchboard-update")
    }
}

/// The newest release in the API's reply, or `None` when it is a draft
/// or a pre-release or has no bundle. Pure, so a fixture tests it.
pub fn parse_latest(json: &str) -> Result<Option<Release>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("the release listing is not JSON: {e}"))?;
    if value["draft"].as_bool() == Some(true) || value["prerelease"].as_bool() == Some(true) {
        return Ok(None);
    }
    let Some(tag) = value["tag_name"].as_str() else {
        return Err("the release has no tag".into());
    };
    let version = tag.trim_start_matches('v').to_owned();
    let assets = value["assets"].as_array().cloned().unwrap_or_default();
    let asset = |pick: &dyn Fn(&str) -> bool| {
        assets.iter().find_map(|a| {
            let name = a["name"].as_str().filter(|n| pick(n))?;
            let url = a["browser_download_url"].as_str()?;
            Some((name.to_owned(), url.to_owned()))
        })
    };
    let Some((asset_name, asset_url)) = asset(&|n| {
        n.starts_with("Switchboard")
            && Path::new(n)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    }) else {
        return Ok(None);
    };
    let digest_url = asset(&|n| n == format!("{asset_name}.sha256")).map(|(_, url)| url);
    Ok(Some(Release {
        version,
        notes: value["body"].as_str().unwrap_or_default().to_owned(),
        asset_url,
        asset_name,
        digest_url,
    }))
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(USER_AGENT)
            .build(),
    )
}

fn fetch_text(url: &str, timeout: Duration) -> Result<String, String> {
    agent(timeout)
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("{url}: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("{url}: {e}"))
}

fn fetch_file(url: &str, to: &Path) -> Result<(), String> {
    let mut reply = agent(DOWNLOAD_TIMEOUT)
        .get(url)
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    let mut reader = reply.body_mut().with_config().limit(ZIP_LIMIT).reader();
    let mut file = std::fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("{url}: {e}"))?;
    file.flush().map_err(|e| format!("{}: {e}", to.display()))
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// The bundle the running executable belongs to: `Switchboard.app`
/// three levels up from `Contents/MacOS/switchboard`. A bare binary
/// (`cargo run`) has none, and an update cannot replace it.
fn running_bundle() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("the running executable: {e}"))?;
    exe.ancestors()
        .nth(3)
        .filter(|p| p.extension().is_some_and(|x| x == "app"))
        .map(Path::to_path_buf)
        .ok_or_else(|| "the running app is not a bundle; install the release by hand".to_owned())
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(text)
    } else {
        Err(format!("{program} {}: {}", args.join(" "), text.trim()))
    }
}

/// The Team ID a bundle is signed under, or `None` when it is unsigned
/// or signed without one (ad hoc, a self-signed certificate).
fn team_id(bundle: &Path) -> Option<String> {
    let text = run("/usr/bin/codesign", &["-dvv", &bundle.to_string_lossy()]).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("TeamIdentifier="))
        .filter(|id| *id != "not set")
        .map(str::to_owned)
}

/// The helper that swaps the bundles once the app has exited: the old
/// bundle is moved aside by rename (the running process keeps mapping
/// it), the new one copied in with `ditto` (the signature survives),
/// and the old put back if that fails; either way the app relaunches.
/// Paths arrive as arguments, so spaces are safe. The last argument is
/// the launcher, `/usr/bin/open` outside the tests.
const SWAP: &str = r#"#!/bin/sh
PID="$1"; SRC="$2"; DEST="$3"; LAUNCH="$4"
PREVIOUS="$(dirname "$DEST")/.$(basename "$DEST").previous"
while kill -0 "$PID" 2>/dev/null; do sleep 0.2; done
rm -rf "$PREVIOUS"
mv "$DEST" "$PREVIOUS" || exit 1
if /usr/bin/ditto "$SRC" "$DEST"; then
  "$LAUNCH" "$DEST"
else
  rm -rf "$DEST"
  mv "$PREVIOUS" "$DEST"
  "$LAUNCH" "$DEST"
  exit 1
fi
"#;

impl UpdateSource for GitHubReleases {
    fn latest(&self) -> Result<Option<Release>, String> {
        let url = format!("https://api.github.com/repos/{}/releases/latest", self.repo);
        match agent(API_TIMEOUT)
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .call()
        {
            Ok(mut reply) => parse_latest(
                &reply
                    .body_mut()
                    .read_to_string()
                    .map_err(|e| format!("{url}: {e}"))?,
            ),
            // No release yet.
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            Err(e) => Err(format!("{url}: {e}")),
        }
    }

    fn download(&self, release: &Release) -> Result<PathBuf, String> {
        let Some(digest_url) = &release.digest_url else {
            return Err("the release carries no checksum".into());
        };
        let staging = Self::staging();
        let _ = std::fs::remove_dir_all(&staging);
        std::fs::create_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
        let zip = staging.join(&release.asset_name);
        fetch_file(&release.asset_url, &zip)?;
        let published = fetch_text(digest_url, API_TIMEOUT)?;
        let published = published
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let actual = sha256_hex(&zip)?;
        if published.len() != 64 || published != actual {
            return Err("the bundle's checksum is not the published one".into());
        }
        let unpacked = staging.join("unpacked");
        run(
            "/usr/bin/ditto",
            &[
                "-x",
                "-k",
                &zip.to_string_lossy(),
                &unpacked.to_string_lossy(),
            ],
        )?;
        let app = unpacked.join("Switchboard.app");
        if !app.is_dir() {
            return Err("the zip holds no Switchboard.app".into());
        }
        run(
            "/usr/bin/codesign",
            &["--verify", "--deep", "--strict", &app.to_string_lossy()],
        )?;
        let running = running_bundle()?;
        let Some(expected) = team_id(&running) else {
            return Err(
                "the running app is not signed under a team; install the release by hand".into(),
            );
        };
        match team_id(&app) {
            Some(id) if id == expected => Ok(app),
            _ => Err("the bundle is not signed by the same team as the running app".into()),
        }
    }

    fn install(&self, staged: &Path) -> Result<(), String> {
        let running = running_bundle()?;
        let parent = running
            .parent()
            .ok_or_else(|| "the running app has no parent directory".to_owned())?;
        let probe = parent.join(".switchboard-update-probe");
        std::fs::File::create(&probe)
            .and_then(|_| std::fs::remove_file(&probe))
            .map_err(|_| {
                format!(
                    "{} is not writable; move the app to ~/Applications",
                    parent.display()
                )
            })?;
        let script = Self::staging().join("swap.sh");
        std::fs::write(&script, SWAP).map_err(|e| format!("{}: {e}", script.display()))?;
        let log = std::fs::File::create(Self::staging().join("swap.log"))
            .map_err(|e| format!("the swap log: {e}"))?;
        Command::new("/usr/bin/nohup")
            .arg("/bin/sh")
            .arg(&script)
            .arg(std::process::id().to_string())
            .arg(staged)
            .arg(&running)
            .arg("/usr/bin/open")
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone().map_err(|e| format!("the swap log: {e}"))?)
            .stderr(log)
            .spawn()
            .map_err(|e| format!("the swap helper: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = r###"{
      "tag_name": "v0.3.0", "draft": false, "prerelease": false,
      "body": "## [0.3.0]\n- A thing.",
      "assets": [
        {"name": "Switchboard-0.3.0.zip.sha256", "browser_download_url": "https://x/s.sha256"},
        {"name": "Switchboard-0.3.0.zip", "browser_download_url": "https://x/s.zip"}
      ]
    }"###;

    #[test]
    fn the_listing_names_the_bundle_its_checksum_and_the_notes() {
        let release = parse_latest(LISTING).unwrap().unwrap();
        assert_eq!(release.version, "0.3.0");
        assert_eq!(release.asset_name, "Switchboard-0.3.0.zip");
        assert_eq!(release.asset_url, "https://x/s.zip");
        assert_eq!(release.digest_url.as_deref(), Some("https://x/s.sha256"));
        assert!(release.notes.contains("A thing"));
    }

    #[test]
    fn a_draft_a_prerelease_or_a_release_without_a_bundle_is_nothing() {
        let draft = LISTING.replace(r#""draft": false"#, r#""draft": true"#);
        assert_eq!(parse_latest(&draft).unwrap(), None);
        let pre = LISTING.replace(r#""prerelease": false"#, r#""prerelease": true"#);
        assert_eq!(parse_latest(&pre).unwrap(), None);
        let bare = r#"{"tag_name": "v0.3.0", "assets": []}"#;
        assert_eq!(parse_latest(bare).unwrap(), None);
        assert!(parse_latest("nope").is_err());
        assert!(parse_latest(r#"{"assets": []}"#).is_err());
    }

    /// The helper swaps once the process is gone, keeps the old bundle
    /// aside, and puts it back when the copy fails.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_helper_swaps_after_exit_and_keeps_the_old_bundle_aside() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("swap.sh");
        std::fs::write(&script, SWAP).unwrap();
        let src = dir.path().join("new.app");
        let dest = dir.path().join("Switchboard.app");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(src.join("v"), "new").unwrap();
        std::fs::write(dest.join("v"), "old").unwrap();
        // The helper polls `kill -0`, which a zombie still answers, so
        // the stand-in process is reaped on a thread while the helper
        // waits for it; the app's own exit is reaped by launchd.
        let mut child = Command::new("/bin/sleep").arg("0.5").spawn().unwrap();
        let pid = child.id();
        let reaper = std::thread::spawn(move || child.wait());
        let status = Command::new("/bin/sh")
            .arg(&script)
            .arg(pid.to_string())
            .arg(&src)
            .arg(&dest)
            .arg("/usr/bin/true")
            .status()
            .unwrap();
        reaper.join().unwrap().unwrap();
        assert!(status.success());
        assert_eq!(std::fs::read_to_string(dest.join("v")).unwrap(), "new");
        let previous = dir.path().join(".Switchboard.app.previous");
        assert_eq!(std::fs::read_to_string(previous.join("v")).unwrap(), "old");

        // A source that is gone: the copy fails and the old comes back.
        std::fs::remove_dir_all(&src).unwrap();
        let status = Command::new("/bin/sh")
            .arg(&script)
            .arg("99999")
            .arg(&src)
            .arg(&dest)
            .arg("/usr/bin/true")
            .status()
            .unwrap();
        assert!(!status.success());
        assert_eq!(std::fs::read_to_string(dest.join("v")).unwrap(), "new");
    }
}
