//! Whether a newer Rigometry is out, and the one click that installs it.
//!
//! Modeled on superint-video's updater. The app asks GitHub for the latest
//! release a few seconds after it starts and again whenever its last look is
//! six hours old; the answer is kept with the settings. Only a stable release
//! with a plain version tag and a download for this computer is offered.
//!
//! Installing happens only when someone asks. The release's ZIP is downloaded
//! next to the app, checked against the SHA-256 GitHub publishes for it,
//! unpacked with the system's own tools and tried (`--version`, and on a Mac
//! `codesign --verify`). Then the new files take the old ones' place: Windows
//! lets a running program be renamed, macOS a running app bundle be moved. The
//! new version starts once this one has exited and saved its settings.
//!
//! Downloads use the operating system's `curl`, part of Windows 10 and later
//! and of macOS, so Rigometry carries no TLS stack of its own and uses the
//! system's certificate store. Nothing is sent but the request itself.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};

/// The repository every release comes from. Nothing is downloaded from anywhere else.
pub const REPO: &str = "marlonka/Rigometry";
/// A look older than this is taken again.
const STALE_MS: u64 = 6 * 60 * 60 * 1000;
/// Someone asking twice within this gets the first answer.
const ASKED_FLOOR_MS: u64 = 60 * 1000;
/// The first look waits this long after the start, so it never delays it.
pub const FIRST_LOOK: Duration = Duration::from_secs(5);
/// GitHub's answer for a release is a few kilobytes.
const ANSWER_CAP: u64 = 1024 * 1024;
/// A release ZIP is about 8 MB; anything far larger is not ours.
const MOST_BYTES: u64 = 256 * 1024 * 1024;
/// What of the release notes the app shows; the release page has all of it.
const NOTE_ITEMS: usize = 12;
const NOTE_CHARS: usize = 1500;
/// Leftovers of an update are named like this, beside the app.
const STAGING: &str = ".rigometry-update-";

/// Set, the app never asks GitHub by itself.
const NO_CHECK: &str = "RIGOMETRY_NO_UPDATE_CHECK";
/// Makes this program count as another version, to try an update against a
/// release that is already out. The updated app starts without it.
pub const PRETEND: &str = "RIGOMETRY_PRETEND_VERSION";
/// Given to the new version: the process it waits for before it starts.
pub const AFTER_PID: &str = "RIGOMETRY_UPDATE_AFTER_PID";
/// Given to the new version: the version it replaced.
pub const UPDATED_FROM: &str = "RIGOMETRY_UPDATED_FROM";

// ---------------------------------------------------------------------------------- versions

/// A release number: three plain numbers. A tag with more after them
/// (`-rc.1`, `+build`) is not a release anyone is offered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// `0.4.0` or `v0.4.0`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix('v').unwrap_or(text);
        let numbers = text
            .split('.')
            .map(|part| {
                let plain =
                    !part.is_empty() && part.len() <= 9 && part.bytes().all(|b| b.is_ascii_digit());
                plain.then(|| part.parse::<u64>().ok()).flatten()
            })
            .collect::<Option<Vec<_>>>()?;
        match numbers[..] {
            [major, minor, patch] => Some(Self(major, minor, patch)),
            _ => None,
        }
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The version this program counts as: its own, or the one `RIGOMETRY_PRETEND_VERSION` names.
pub fn running_version() -> Option<Version> {
    std::env::var(PRETEND)
        .ok()
        .and_then(|text| Version::parse(&text))
        .or_else(|| Version::parse(env!("CARGO_PKG_VERSION")))
}

// ---------------------------------------------------------------------------------- releases

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub tag: String,
    /// The tag's number, `0.5.0`.
    pub version: String,
    /// The release page, built from the tag and never taken from the answer.
    pub url: String,
    /// What is new, as plain text, one entry per paragraph or list item.
    pub notes: Vec<String>,
    /// This computer's download.
    pub asset: Option<Asset>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lower-case hex from GitHub's `digest`.
    pub sha256: Option<String>,
    /// The `.sha256` file published beside it, for a file GitHub keeps no digest of.
    pub sums_url: Option<String>,
}

/// The systems releases are built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Other,
}

impl Platform {
    pub const CURRENT: Self = if cfg!(windows) {
        Self::Windows
    } else if cfg!(target_os = "macos") {
        Self::MacOs
    } else {
        Self::Other
    };

    /// Whether `name` is this platform's download of release `version`. The
    /// Windows archive carries a build time after the platform.
    fn asset(self, name: &str, version: &str) -> bool {
        match self {
            Self::Windows => {
                name.starts_with(&format!("Rigometry-{version}-windows-x64"))
                    && name.ends_with(".zip")
            }
            Self::MacOs => name == format!("Rigometry-{version}-macos-universal.zip"),
            Self::Other => false,
        }
    }
}

fn is_sha256(hex: &str) -> bool {
    hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The release in GitHub's answer, if it is one to offer: published, not a
/// pre-release, with a plain version as its tag. Only files at this release's
/// own download address count.
pub fn parse_release(answer: &Value) -> Option<Release> {
    parse_release_for(answer, Platform::CURRENT)
}

fn parse_release_for(answer: &Value, platform: Platform) -> Option<Release> {
    if answer["draft"] == true || answer["prerelease"] == true {
        return None;
    }
    let tag = answer["tag_name"].as_str()?;
    let version = Version::parse(tag)?.to_string();
    let prefix = format!("https://github.com/{REPO}/releases/download/{tag}/");
    let assets: Vec<(&str, &str, &Value)> = answer["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let name = asset["name"].as_str()?;
            let url = asset["browser_download_url"].as_str()?;
            let plain = !name.is_empty()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
            (plain && url == format!("{prefix}{name}")).then_some((name, url, asset))
        })
        .collect();
    let asset = assets
        .iter()
        .find(|(name, _, _)| platform.asset(name, &version))
        .and_then(|&(name, url, asset)| {
            Some(Asset {
                name: name.into(),
                url: url.into(),
                size: asset["size"].as_u64()?,
                sha256: asset["digest"]
                    .as_str()
                    .and_then(|digest| digest.strip_prefix("sha256:"))
                    .filter(|hex| is_sha256(hex))
                    .map(str::to_ascii_lowercase),
                sums_url: assets
                    .iter()
                    .find(|(sums, _, _)| *sums == format!("{name}.sha256"))
                    .map(|(_, url, _)| (*url).to_owned()),
            })
        });
    Some(Release {
        url: format!("https://github.com/{REPO}/releases/tag/{tag}"),
        notes: notes(answer["body"].as_str().unwrap_or_default()),
        tag: tag.into(),
        version,
        asset,
    })
}

/// The release notes' list of changes as plain text: the part before the
/// platform notes, without Markdown, capped at what fits the settings page.
pub fn notes(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut used = 0;
    for line in body.lines() {
        let line = line.trim();
        // The platform notes and the source links follow the changes.
        if line.starts_with("**Windows:**") || line.starts_with("Source:") {
            break;
        }
        let item = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .unwrap_or(line);
        let text = plain(item);
        if text.is_empty() || line.starts_with('#') {
            continue;
        }
        used += text.chars().count();
        if out.len() == NOTE_ITEMS || used > NOTE_CHARS {
            break;
        }
        out.push(text);
    }
    out
}

/// Markdown's marks taken out: a link keeps its words, emphasis and code their text.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text.trim();
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let link = after.find(']').and_then(|close| {
            let target = after[close + 1..].strip_prefix('(')?;
            let end = target.find(')')?;
            Some((close, &target[end + 1..]))
        });
        match link {
            Some((close, next)) => {
                out.push_str(&after[..close]);
                rest = next;
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "").trim().to_string()
}

// ---------------------------------------------------------------------------------- the record

/// What is kept with the settings: the last look, and the version to skip.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Record {
    pub checked_at: Option<u64>,
    pub etag: Option<String>,
    pub release: Option<Release>,
    /// The version the user chose to skip.
    pub skipped: Option<String>,
    /// GitHub's request limit was reached: no look by itself before this.
    pub retry_after: Option<u64>,
}

impl Record {
    /// A look by itself is due: the last one is old and GitHub does not ask to wait.
    pub fn due(&self, now: u64) -> bool {
        let limited = self.retry_after.is_some_and(|until| until > now);
        let stale = self
            .checked_at
            .is_none_or(|at| now.saturating_sub(at) >= STALE_MS);
        stale && !limited
    }

    /// The release to offer a copy running `running`: a newer one with this computer's download.
    pub fn offered(&self, running: Option<Version>) -> Option<&Release> {
        let release = self.release.as_ref()?;
        let newer = Version::parse(&release.version)
            .zip(running)
            .is_some_and(|(release, running)| release > running);
        (newer && release.asset.is_some()).then_some(release)
    }

    /// Takes in what a look found.
    pub fn apply(&mut self, checked: &Checked, now: u64) {
        match checked {
            Checked::Unchanged => {
                self.checked_at = Some(now);
                self.retry_after = None;
            }
            Checked::Latest { release, etag } => {
                self.release = release.as_deref().cloned();
                self.etag = etag.clone();
                self.checked_at = Some(now);
                self.retry_after = None;
            }
            Checked::Limited(until) => self.retry_after = Some(*until),
        }
    }

    /// Someone asking within a minute of the last look gets that look.
    pub fn fresh(&self, now: u64) -> bool {
        self.checked_at
            .is_some_and(|at| now.saturating_sub(at) < ASKED_FLOOR_MS)
    }
}

/// Whether the app asks GitHub by itself: the setting, unless
/// `RIGOMETRY_NO_UPDATE_CHECK` is set or this is a CI machine.
pub fn automatic(setting: bool) -> bool {
    setting && std::env::var_os(NO_CHECK).is_none() && std::env::var_os("CI").is_none()
}

// ----------------------------------------------------------------------------------- looking

/// What a look found.
#[derive(Clone, Debug)]
pub enum Checked {
    /// The release is the one the record has (GitHub answered 304).
    Unchanged,
    Latest {
        release: Option<Box<Release>>,
        etag: Option<String>,
    },
    /// GitHub's request limit is reached until this time.
    Limited(u64),
}

/// The system's curl, by absolute path: never one found on `PATH` or in the
/// working folder.
fn curl() -> PathBuf {
    system_tool("curl")
}

#[cfg(windows)]
fn system_tool(name: &str) -> PathBuf {
    use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut buffer = [0u16; 260];
    let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    let folder = if length > 0 && length < buffer.len() {
        PathBuf::from(String::from_utf16_lossy(&buffer[..length]))
    } else {
        PathBuf::from(r"C:\Windows\System32")
    };
    folder.join(format!("{name}.exe"))
}

#[cfg(not(windows))]
fn system_tool(name: &str) -> PathBuf {
    PathBuf::from("/usr/bin").join(name)
}

/// A command that opens no console window and reads nothing.
fn quiet(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// curl's options for every request: HTTPS only, also after redirects, which
/// lead from GitHub's download address to its file storage.
fn curl_command() -> Command {
    let mut command = quiet(&curl());
    command
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--max-redirs",
            "5",
        ])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args(["--connect-timeout", "15"])
        .arg("--user-agent")
        .arg(concat!("Rigometry/", env!("CARGO_PKG_VERSION")));
    command
}

/// Runs curl to completion within `limit`, reporting the bytes written to
/// `watch` meanwhile. Stops early when `cancel` is set.
fn run_curl(
    mut command: Command,
    limit: Duration,
    watch: Option<(&Path, &AtomicU64)>,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("curl could not start: {error}"))?;
    // Read while waiting: a full pipe would stall curl.
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        text
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let started = Instant::now();
    let status = loop {
        if let Some((file, received)) = watch
            && let Ok(metadata) = std::fs::metadata(file)
        {
            received.store(metadata.len(), Ordering::Relaxed);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if cancel.load(Ordering::Relaxed) || started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(if cancel.load(Ordering::Relaxed) {
                    "Update cancelled".into()
                } else {
                    "The download took too long".into()
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) => return Err(format!("curl: {error}")),
        }
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if status.success() {
        Ok(stdout)
    } else {
        let reason = stderr.lines().last().unwrap_or_default().trim();
        Err(format!("GitHub could not be reached: {reason}"))
    }
}

/// Asks GitHub for the latest release, sending `etag` so an unchanged answer is
/// a short 304 that does not count against GitHub's request limit.
pub fn check(etag: Option<&str>) -> Result<Checked, String> {
    let folder = tempfile::tempdir().map_err(|error| error.to_string())?;
    let headers = folder.path().join("headers");
    let body = folder.path().join("body");
    let mut command = curl_command();
    command
        .args(["--max-time", "30", "--max-filesize"])
        .arg(ANSWER_CAP.to_string())
        .args(["--header", "Accept: application/vnd.github+json"])
        .args(["--header", "X-GitHub-Api-Version: 2022-11-28"])
        .arg("--dump-header")
        .arg(&headers)
        .arg("--output")
        .arg(&body)
        .args(["--write-out", "%{http_code}"]);
    if let Some(etag) = etag.filter(|etag| etag.is_ascii() && !etag.contains(['\r', '\n'])) {
        command
            .arg("--header")
            .arg(format!("If-None-Match: {etag}"));
    }
    command.arg(format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ));
    let status = run_curl(
        command,
        Duration::from_secs(40),
        None,
        &AtomicBool::new(false),
    )?;
    let headers = std::fs::read_to_string(&headers).unwrap_or_default();
    let header = |name: &str| last_header(&headers, name);
    match status.trim() {
        "304" => Ok(Checked::Unchanged),
        // No release yet.
        "404" => Ok(Checked::Latest {
            release: None,
            etag: None,
        }),
        "403" | "429"
            if status.trim() == "429"
                || header("x-ratelimit-remaining").as_deref() == Some("0") =>
        {
            let now = crate::model::now_ms();
            let until = header("x-ratelimit-reset")
                .and_then(|reset| reset.parse::<u64>().ok())
                .map(|seconds| seconds.saturating_mul(1000))
                .filter(|&until| until > now)
                .unwrap_or(now + 60 * 60 * 1000);
            Ok(Checked::Limited(until))
        }
        "200" => {
            let bytes = std::fs::read(&body).map_err(|error| error.to_string())?;
            let answer: Value = serde_json::from_slice(&bytes)
                .map_err(|_| "GitHub's answer is not JSON".to_string())?;
            Ok(Checked::Latest {
                release: parse_release(&answer).map(Box::new),
                etag: header("etag"),
            })
        }
        other => Err(format!("GitHub answered {other}")),
    }
}

/// The value of header `name` in the last response of a dump that may hold
/// several, one per redirect.
fn last_header(dump: &str, name: &str) -> Option<String> {
    let last = dump
        .split("\r\n\r\n")
        .filter(|block| !block.trim().is_empty())
        .last()?;
    last.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().to_owned())
    })
}

// -------------------------------------------------------------------------------- installing

/// How this copy came onto the computer, which decides whether it can replace itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The program unpacked from the Windows ZIP.
    Program(PathBuf),
    /// The app bundle on a Mac.
    Bundle(PathBuf),
    /// Built from the code, or anywhere it cannot replace itself: shown the release page.
    Fixed(&'static str),
}

/// What an update would replace.
pub fn target() -> Target {
    match std::env::current_exe() {
        Ok(exe) => target_of(&exe, Platform::CURRENT),
        Err(_) => Target::Fixed("Rigometry cannot find its own program"),
    }
}

/// A build from the code has the repository's `Cargo.toml` a few folders up; a
/// Mac app started from Downloads runs from a read-only translocated copy.
fn target_of(exe: &Path, platform: Platform) -> Target {
    if exe
        .ancestors()
        .skip(1)
        .take(5)
        .any(|folder| folder.join("Cargo.toml").is_file())
    {
        return Target::Fixed("This copy was built from the source code");
    }
    if platform == Platform::MacOs {
        let bundle = exe.ancestors().nth(3).filter(|bundle| {
            bundle.extension().is_some_and(|e| e == "app")
                && exe.parent().is_some_and(|p| p.ends_with("Contents/MacOS"))
        });
        return match bundle {
            Some(bundle) if bundle.to_string_lossy().contains("/AppTranslocation/") => {
                Target::Fixed("Move Rigometry to Applications to update it from the app")
            }
            Some(bundle) => Target::Bundle(bundle.to_path_buf()),
            None => Target::Fixed("Rigometry is not running from its app"),
        };
    }
    if platform == Platform::Windows {
        return Target::Program(exe.to_path_buf());
    }
    Target::Fixed("Updates are not available on this system")
}

impl Target {
    /// The folder the new files are put in first: beside the program, so they
    /// can take its place by renaming.
    fn folder(&self) -> Option<&Path> {
        match self {
            Self::Program(path) | Self::Bundle(path) => path.parent(),
            Self::Fixed(_) => None,
        }
    }
}

/// Downloads, checks and puts in place `release` for `target`. `received` counts
/// the downloaded bytes. Returns the program to start, which is the old path.
pub fn install(
    release: &Release,
    target: &Target,
    received: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<PathBuf, String> {
    let Some(folder) = target.folder() else {
        return Err(match target {
            Target::Fixed(reason) => (*reason).into(),
            _ => "No folder for the update".into(),
        });
    };
    let asset = release
        .asset
        .as_ref()
        .ok_or("The release has no download for this computer")?;
    if asset.size == 0 || asset.size > MOST_BYTES {
        return Err("The release's download has an unexpected size".into());
    }
    // Same folder, same volume: the new files take the old ones' place by renaming.
    let staging = tempfile::Builder::new()
        .prefix(STAGING)
        .tempdir_in(folder)
        .map_err(|error| format!("Rigometry cannot write beside itself: {error}"))?;
    let archive = staging.path().join("download.zip");
    let mut command = curl_command();
    command
        .args(["--fail", "--max-time", "1800", "--max-filesize"])
        .arg(asset.size.to_string())
        .arg("--output")
        .arg(&archive)
        .arg(&asset.url);
    run_curl(
        command,
        Duration::from_secs(1830),
        Some((&archive, received)),
        cancel,
    )?;
    let sha256 = match &asset.sha256 {
        Some(sha256) => sha256.clone(),
        None => sidecar_sha256(asset, staging.path(), cancel)?,
    };
    verify(&archive, asset.size, &sha256)?;
    let unpacked = staging.path().join("unpacked");
    std::fs::create_dir(&unpacked).map_err(|error| error.to_string())?;
    unpack(&archive, &unpacked)?;
    let started = match target {
        Target::Program(exe) => {
            let package = unpacked.join("Rigometry");
            let new = package.join("Rigometry.exe");
            confirm(&new, &release.version)?;
            replace_program(exe, &package)?;
            exe.clone()
        }
        Target::Bundle(bundle) => {
            let new = find_bundle(&unpacked).ok_or("The download holds no Rigometry.app")?;
            confirm(&new.join("Contents/MacOS/rigometry"), &release.version)?;
            codesign(&new)?;
            replace_bundle(bundle, &new, staging.path())?;
            bundle.join("Contents/MacOS/rigometry")
        }
        Target::Fixed(reason) => return Err((*reason).into()),
    };
    // The old program or bundle stays in the staging folder until the new
    // version, once started, tidies it away.
    let _ = staging.keep();
    Ok(started)
}

/// The checksum from the `.sha256` file beside the download.
fn sidecar_sha256(asset: &Asset, folder: &Path, cancel: &AtomicBool) -> Result<String, String> {
    let url = asset
        .sums_url
        .as_ref()
        .ok_or("The release has no checksum for its download, so it is not installed")?;
    let file = folder.join("download.sha256");
    let mut command = curl_command();
    command
        .args([
            "--fail",
            "--max-time",
            "60",
            "--max-filesize",
            "4096",
            "--output",
        ])
        .arg(&file)
        .arg(url);
    run_curl(command, Duration::from_secs(70), None, cancel)?;
    let text = std::fs::read_to_string(&file).map_err(|error| error.to_string())?;
    sums_line(&text, &asset.name)
        .ok_or_else(|| "The release's checksum file does not name its download".into())
}

/// The checksum for `name` in a `sha256sum`-style list.
fn sums_line(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (hash, file) = line.trim().split_once(char::is_whitespace)?;
        let file = file.trim_start().trim_start_matches('*');
        (file == name && is_sha256(hash)).then(|| hash.to_ascii_lowercase())
    })
}

/// The file has exactly the size and SHA-256 the release names.
fn verify(file: &Path, size: u64, sha256: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let mut reader = std::fs::File::open(file).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut length = 0u64;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        length += read as u64;
    }
    let got: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if length == size && got == sha256 {
        Ok(())
    } else {
        Err("The download is not the file the release names, so it is not installed".into())
    }
}

/// Unpacks the ZIP with the system's own tool: `tar` on Windows, `ditto` on a
/// Mac, which keeps an app bundle's signature and attributes intact.
fn unpack(archive: &Path, into: &Path) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = quiet(Path::new("/usr/bin/ditto"));
        command.args(["-x", "-k"]).arg(archive).arg(into);
        command
    } else if cfg!(windows) {
        let mut command = quiet(&system_tool("tar"));
        command.arg("-xf").arg(archive).arg("-C").arg(into);
        command
    } else {
        let mut command = quiet(Path::new("/usr/bin/unzip"));
        command.arg("-q").arg(archive).arg("-d").arg(into);
        command
    };
    let output = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("The download could not be unpacked: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "The download could not be unpacked: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The new program runs and is the version the release names.
fn confirm(program: &Path, version: &str) -> Result<(), String> {
    let output = quiet(program)
        .arg("--version")
        .env_remove(PRETEND)
        .stderr(Stdio::null())
        .output()
        .map_err(|error| format!("The new version does not start: {error}"))?;
    let said = String::from_utf8_lossy(&output.stdout);
    if output.status.success() && said.trim() == format!("Rigometry {version}") {
        Ok(())
    } else {
        Err(format!(
            "The new version did not report itself as {version}"
        ))
    }
}

/// On a Mac, the unpacked app's signature still covers every file.
fn codesign(bundle: &Path) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let status = quiet(Path::new("/usr/bin/codesign"))
        .args(["--verify", "--strict"])
        .arg(bundle)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("The new app's signature does not match its files".into())
    }
}

/// The `Rigometry.app` in the unpacked archive, one folder down.
fn find_bundle(unpacked: &Path) -> Option<PathBuf> {
    std::fs::read_dir(unpacked)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join("Rigometry.app"))
        .find(|bundle| bundle.is_dir())
}

/// Puts the new package's files beside `exe`. The running program is renamed
/// first, which Windows allows; every other file is replaced in place. If the
/// new program cannot take the old one's place, the old one is restored.
fn replace_program(exe: &Path, package: &Path) -> Result<(), String> {
    let folder = exe.parent().ok_or("No folder for the program")?;
    let old = old_program(exe);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old)
        .map_err(|error| format!("Rigometry cannot replace itself: {error}"))?;
    if let Err(error) = std::fs::rename(package.join("Rigometry.exe"), exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(format!("Rigometry cannot replace itself: {error}"));
    }
    // Documentation and notices follow; a file that cannot be replaced keeps
    // its old version and changes nothing about the program.
    let mut stack = vec![package.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(relative) = path.strip_prefix(package) {
                let destination = folder.join(relative);
                if let Some(parent) = destination.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::rename(&path, &destination);
            }
        }
    }
    Ok(())
}

/// Where the replaced program waits until the new one deletes it.
fn old_program(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    exe.with_file_name(name)
}

/// Moves the running bundle aside into `staging` and the new one into its place;
/// restores the old one if that fails.
fn replace_bundle(bundle: &Path, new: &Path, staging: &Path) -> Result<(), String> {
    let aside = staging.join("previous.app");
    std::fs::rename(bundle, &aside)
        .map_err(|error| format!("Rigometry cannot replace itself: {error}"))?;
    if let Err(error) = std::fs::rename(new, bundle) {
        let _ = std::fs::rename(&aside, bundle);
        return Err(format!("Rigometry cannot replace itself: {error}"));
    }
    Ok(())
}

/// Starts the new version, which waits until this process has exited.
pub fn launch(program: &Path, from: &str) -> Result<(), String> {
    Command::new(program)
        .env(AFTER_PID, std::process::id().to_string())
        .env(UPDATED_FROM, from)
        .env_remove(PRETEND)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("The new version did not start: {error}"))
}

// ------------------------------------------------------------------------- after an update

/// Called first thing in `main`: a version started by an update waits until the
/// one it replaced has exited and written its settings, then tidies up after it.
/// Returns the version it replaced.
pub fn after_update() -> Option<String> {
    let from = std::env::var(UPDATED_FROM)
        .ok()
        .filter(|v| Version::parse(v).is_some());
    if let Some(pid) = std::env::var(AFTER_PID)
        .ok()
        .and_then(|pid| pid.parse::<u32>().ok())
    {
        wait_for_exit(pid, Duration::from_secs(20));
    }
    // SAFETY: called at the start of `main`, before any other thread exists.
    unsafe {
        std::env::remove_var(AFTER_PID);
        std::env::remove_var(UPDATED_FROM);
    }
    tidy(from.is_some());
    from
}

/// Removes what an earlier update left beside the program: the replaced
/// program, and staging folders; right after an update all of them, otherwise
/// those older than a few minutes, which no update in progress can still use.
pub fn tidy(updated: bool) {
    let target = target();
    if let Target::Program(exe) = &target {
        let _ = std::fs::remove_file(old_program(exe));
    }
    let Some(folder) = target.folder() else {
        return;
    };
    for entry in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let named = entry.file_name().to_string_lossy().starts_with(STAGING);
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|at| at.elapsed().is_ok_and(|age| age > Duration::from_secs(120)));
        if named && (updated || old) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(windows)]
fn wait_for_exit(pid: u32, limit: Duration) {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
    };
    // SAFETY: the handle is checked and closed; waiting has no other effect.
    unsafe {
        if let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            WaitForSingleObject(process, limit.as_millis().min(u32::MAX as u128) as u32);
            let _ = CloseHandle(process);
        }
    }
}

#[cfg(unix)]
fn wait_for_exit(pid: u32, limit: Duration) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    let started = Instant::now();
    // SAFETY: signal 0 only asks whether the process exists.
    while unsafe { libc::kill(pid, 0) } == 0 && started.elapsed() < limit {
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests;

/// Opens a release page of this repository in the default browser.
pub fn open_page(url: &str) -> Result<(), String> {
    let prefix = format!("https://github.com/{REPO}/releases/");
    let safe = url.starts_with(&prefix)
        && url[prefix.len()..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b));
    if !safe {
        return Err("Not a release page of Rigometry".into());
    }
    let mut command = if cfg!(windows) {
        let windows = system_tool("cmd")
            .parent()
            .and_then(Path::parent)
            .map(|folder| folder.join("explorer.exe"))
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows\explorer.exe"));
        Command::new(windows)
    } else if cfg!(target_os = "macos") {
        Command::new("/usr/bin/open")
    } else {
        Command::new("/usr/bin/xdg-open")
    };
    let mut child = command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    // Collect the exit status, so no zombie process stays behind.
    std::thread::spawn(move || child.wait());
    Ok(())
}
