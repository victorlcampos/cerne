//! Updates from the latest GitHub release.
//!
//! A release is a tag `vX.Y.Z` with these assets, as the reusable workflow
//! `.github/workflows/release-cli.yml` publishes them:
//!
//! - `<bin>-<target>.tar.gz` (`.zip` on Windows) for each system, holding the
//!   executable as `<bin>-<target>/<bin>`;
//! - `SHA256SUMS`, with a line `<sha-256>  <file>` per archive.
//!
//! A macOS app ships as a zipped bundle instead, such as `Rabisco.zip` holding
//! `Rabisco.app` (see [`Updater::download_app`]).
//!
//! Nothing changes until the download matches its SHA-256 and turns out to be
//! the release's version; then one rename puts it in place, so an update cut
//! short leaves the old version whole.
//!
//! For a command-line app, `<bin> update` is one call:
//!
//! ```no_run
//! # let check = false;
//! cerne::update::run("victorlcampos/vitals", "vitals", env!("CARGO_PKG_VERSION"), check);
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::Duration;

use sha2::{Digest, Sha256};

pub use semver::Version;

#[cfg(target_os = "macos")]
mod app;
mod archive;

#[cfg(target_os = "macos")]
pub use app::StagedApp;

/// Largest download taken: an archive or a zipped app.
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

/// This executable's system, as release archives name it, or `None` where
/// releases bring no executable (it updates from source there).
pub const TARGET: Option<&str> = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("aarch64-apple-darwin")
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Some("x86_64-apple-darwin")
} else if cfg!(all(
    target_os = "linux",
    target_arch = "x86_64",
    target_env = "gnu"
)) {
    Some("x86_64-unknown-linux-gnu")
} else if cfg!(all(
    target_os = "linux",
    target_arch = "aarch64",
    target_env = "gnu"
)) {
    Some("aarch64-unknown-linux-gnu")
} else if cfg!(all(
    target_os = "windows",
    target_arch = "x86_64",
    target_env = "msvc"
)) {
    Some("x86_64-pc-windows-msvc")
} else {
    None
};

/// Why an update did not happen, in words for whoever asked for it.
#[derive(Debug)]
pub struct Error(String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(Error(message.into()))
}

/// Says what was being done when an error came up.
trait Context<T> {
    fn context(self, doing: impl FnOnce() -> String) -> Result<T>;
}

impl<T, E: fmt::Display> Context<T> for std::result::Result<T, E> {
    fn context(self, doing: impl FnOnce() -> String) -> Result<T> {
        self.map_err(|error| Error(format!("{}: {error}", doing())))
    }
}

/// Finds and fetches the releases of one app.
pub struct Updater {
    repo: String,
    bin: String,
    current: Version,
    agent: ureq::Agent,
}

impl Updater {
    /// `repo` is the app's GitHub repository ("owner/name"), `bin` its
    /// executable's name and `version` the running one, normally
    /// `env!("CARGO_PKG_VERSION")`.
    pub fn new(repo: &str, bin: &str, version: &str) -> Result<Self> {
        let current = Version::parse(version)
            .context(|| format!("the running version {version:?} is not like 1.2.3"))?;
        // The system's trust store, like browsers and curl: networks that
        // inspect TLS (at work, in a coworking) sign with roots only it knows.
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .timeout_connect(Some(Duration::from_secs(20)))
            .timeout_global(Some(Duration::from_secs(600)))
            .user_agent(format!("{bin}/{version}"))
            .tls_config(tls)
            .build()
            .into();
        Ok(Self {
            repo: repo.to_owned(),
            bin: bin.to_owned(),
            current,
            agent,
        })
    }

    /// The running version.
    pub fn current(&self) -> &Version {
        &self.current
    }

    /// The latest release, drafts and pre-releases aside: the one GitHub's
    /// `releases/latest` page points to. GitHub's pages, unlike its API, set
    /// no hourly limit on requests without a login, which offices and CI
    /// runners sharing one address soon reach.
    pub fn latest(&self) -> Result<Release> {
        let url = format!("https://github.com/{}/releases/latest", self.repo);
        let response = self
            .agent
            .get(&url)
            .config()
            .max_redirects(0)
            .build()
            .call();
        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(404)) => {
                return fail(format!(
                    "found no repository at https://github.com/{}",
                    self.repo
                ));
            }
            Err(error) => {
                return fail(format!(
                    "could not reach GitHub ({error}); check the network"
                ));
            }
        };
        if !response.status().is_redirection() {
            return fail(format!(
                "GitHub answered {} for {url}, not where its latest release is",
                response.status()
            ));
        }
        let location = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let Some(tag) = tag_in(location, &self.repo) else {
            return fail(format!(
                "{} has no release yet: https://github.com/{}/releases",
                self.repo, self.repo
            ));
        };
        let version = tag
            .strip_prefix('v')
            .and_then(|version| Version::parse(version).ok())
            .ok_or_else(|| {
                Error(format!(
                    "the latest release's tag {tag:?} is not like v1.2.3"
                ))
            })?;
        Ok(Release { version, tag })
    }

    /// Downloads this system's executable from `release`, checks it against
    /// the release's SHA-256 and that it says it is the release's version,
    /// and leaves it beside the running executable, ready to take its place.
    pub fn download(&self, release: &Release) -> Result<Download> {
        let Some(target) = TARGET else {
            return fail(format!(
                "releases bring no ready-made {} for this system; update from source: \
                 cargo install --force --git https://github.com/{}",
                self.bin, self.repo
            ));
        };
        let folder = format!("{}-{target}", self.bin);
        let name = format!("{folder}.{}", if cfg!(windows) { "zip" } else { "tar.gz" });
        let archive = self.fetch_verified(release, &name)?;
        let executable = archive::executable(&archive, &folder, &self.bin)?;

        let installed = std::env::current_exe()
            .and_then(fs::canonicalize)
            .context(|| "find the running executable".to_owned())?;
        let dir = installed
            .parent()
            .ok_or_else(|| Error("the running executable has no folder".to_owned()))?;
        let mut staged = tempfile::Builder::new()
            .prefix(&format!(".{}-update-", self.bin))
            .suffix(std::env::consts::EXE_SUFFIX)
            .tempfile_in(dir)
            .context(|| format!("cannot write to {}", dir.display()))?;
        staged
            .write_all(&executable)
            .and_then(|()| staged.as_file().sync_all())
            .context(|| format!("write the new {} in {}", self.bin, dir.display()))?;
        let permissions = fs::metadata(&installed)
            .context(|| format!("read {}", installed.display()))?
            .permissions();
        #[cfg(unix)]
        let permissions = {
            use std::os::unix::fs::PermissionsExt;
            fs::Permissions::from_mode(permissions.mode() | 0o755)
        };
        staged
            .as_file()
            .set_permissions(permissions)
            .context(|| format!("make the new {} executable", self.bin))?;
        // Closed before running it: Linux will not run a file open for writing.
        let staged = staged.into_temp_path();
        let expected = format!("{} {}", self.bin, release.version);
        let says = version_of(&staged)?;
        if says != expected {
            return fail(format!(
                "the downloaded executable says {says:?} rather than {expected:?}; nothing changed"
            ));
        }
        Ok(Download {
            staged,
            installed,
            version: release.version.clone(),
        })
    }

    /// Downloads the release's `name` once its line in `SHA256SUMS` vouches
    /// for it.
    fn fetch_verified(&self, release: &Release, name: &str) -> Result<Vec<u8>> {
        let sums = self.fetch(release, "SHA256SUMS", 1024 * 1024)?;
        let sums = String::from_utf8(sums).context(|| "read SHA256SUMS".to_owned())?;
        let bytes = self.fetch(release, name, MAX_DOWNLOAD)?;
        verify(&bytes, &sums, name)?;
        Ok(bytes)
    }

    /// The release's file `name`, always from the app's own repository.
    fn fetch(&self, release: &Release, name: &str, limit: u64) -> Result<Vec<u8>> {
        let url = format!(
            "https://github.com/{}/releases/download/{}/{name}",
            self.repo, release.tag
        );
        match self.agent.get(&url).call() {
            Ok(mut response) => response
                .body_mut()
                .with_config()
                .limit(limit)
                .read_to_vec()
                .context(|| format!("download {url}")),
            Err(ureq::Error::StatusCode(404)) => fail(format!(
                "release {} has no {name} (if it just came out, its files may still be \
                 uploading: try again in a few minutes)",
                release.tag
            )),
            Err(error) => fail(format!("download {url}: {error}")),
        }
    }
}

/// The tag in the address `releases/latest` points to,
/// `https://github.com/<repo>/releases/tag/<tag>`; none when it points to
/// the release list instead, as it does before the first release.
fn tag_in(location: &str, repo: &str) -> Option<String> {
    let path = location
        .strip_prefix("https://github.com")
        .unwrap_or(location);
    let prefix = format!("/{repo}/releases/tag/");
    let start = path.get(..prefix.len())?;
    if !start.eq_ignore_ascii_case(&prefix) {
        return None;
    }
    let tag = unescape(&path[prefix.len()..])?;
    (!tag.is_empty() && !tag.contains('/')).then_some(tag)
}

/// Undoes the `%XX` escapes of an address.
fn unescape(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'%' {
            let hex = std::str::from_utf8(tail.get(..2)?).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(byte);
            rest = tail;
        }
    }
    String::from_utf8(bytes).ok()
}

/// A published release.
pub struct Release {
    pub version: Version,
    /// The git tag, like "v1.2.3".
    pub tag: String,
}

/// A verified executable, waiting beside the running one.
pub struct Download {
    staged: tempfile::TempPath,
    installed: PathBuf,
    pub version: Version,
}

impl Download {
    /// Where it goes: the running executable, symbolic links followed.
    pub fn installed(&self) -> &Path {
        &self.installed
    }

    /// Puts the new executable in place of the running one, which keeps
    /// running as it was until it quits.
    pub fn install(self) -> Result<PathBuf> {
        #[cfg(unix)]
        self.staged
            .persist(&self.installed)
            .context(|| format!("replace {}", self.installed.display()))?;
        // Windows will not replace a running executable, only rename it.
        #[cfg(windows)]
        self_replace::self_replace(&self.staged)
            .context(|| format!("replace {}", self.installed.display()))?;
        Ok(self.installed)
    }
}

/// What `executable --version` prints, like "vitals 1.2.3".
fn version_of(executable: &Path) -> Result<String> {
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .context(|| "run the downloaded executable".to_owned())?;
    if !output.status.success() {
        return fail(format!(
            "the downloaded executable did not run ({}); nothing changed",
            output.status
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Checks `bytes` against the line for `name` in a `SHA256SUMS` file:
/// "<64 hex digits>  <name>", or "<digits> *<name>" for files read as binary.
fn verify(bytes: &[u8], sums: &str, name: &str) -> Result<()> {
    let listed: Vec<&str> = sums
        .lines()
        .filter_map(|line| {
            let (digest, file) = line.trim_end().split_once(char::is_whitespace)?;
            let file = file.trim_start().trim_start_matches('*');
            (file == name).then_some(digest)
        })
        .collect();
    let [digest] = listed[..] else {
        return fail(format!(
            "SHA256SUMS lists {name} {} times instead of once; nothing changed",
            listed.len()
        ));
    };
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return fail(format!(
            "SHA256SUMS has no valid digest for {name}; nothing changed"
        ));
    }
    if !format!("{:x}", Sha256::digest(bytes)).eq_ignore_ascii_case(digest) {
        return fail(format!(
            "{name} does not match its SHA-256, so it was not used; nothing changed"
        ));
    }
    Ok(())
}

/// `<bin> update` for a command-line app: updates the running executable to
/// the latest release, telling what it does on the standard error and how it
/// went on the standard output. With `check`, only tells whether there is a
/// newer version.
pub fn run(repo: &str, bin: &str, version: &str, check: bool) -> ExitCode {
    match update(repo, bin, version, check) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{bin}: {error}");
            ExitCode::FAILURE
        }
    }
}

fn update(repo: &str, bin: &str, version: &str, check: bool) -> Result<()> {
    let updater = Updater::new(repo, bin, version)?;
    eprintln!("Looking for a newer {bin}…");
    let release = updater.latest()?;
    let current = updater.current();
    match release.version.cmp(current) {
        Ordering::Equal => {
            println!("{bin} {current} is the latest version.");
            return Ok(());
        }
        Ordering::Less => {
            println!(
                "{bin} {current} is newer than the latest release, {}.",
                release.version
            );
            return Ok(());
        }
        Ordering::Greater if check => {
            println!(
                "{bin} {} is out (this is {current}): `{bin} update` installs it.",
                release.version
            );
            return Ok(());
        }
        Ordering::Greater => {}
    }
    eprintln!(
        "Downloading {bin} {} for {}…",
        release.version,
        TARGET.unwrap_or("this system")
    );
    let path = updater.download(&release)?.install()?;
    println!(
        "Updated {bin} {current} → {}: {}",
        release.version,
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_reject_tampering_and_missing_or_repeated_lines() {
        let bytes = b"verified release";
        let digest = format!("{:x}", Sha256::digest(bytes));
        let line = format!("{digest}  app.tar.gz\r\n");
        verify(bytes, &line, "app.tar.gz").unwrap();
        verify(bytes, &format!("{digest} *app.tar.gz\n"), "app.tar.gz").unwrap();
        let both = format!("{}  other.zip\n{line}", "0".repeat(64));
        verify(bytes, &both, "app.tar.gz").unwrap();
        verify(
            bytes,
            &line.to_uppercase().replace("APP.TAR.GZ", "app.tar.gz"),
            "app.tar.gz",
        )
        .unwrap();

        assert!(verify(b"modified", &line, "app.tar.gz").is_err());
        assert!(verify(bytes, &line, "app.zip").is_err());
        assert!(verify(bytes, &(line.clone() + &line), "app.tar.gz").is_err());
        assert!(verify(bytes, "nothing app.tar.gz", "app.tar.gz").is_err());
        assert!(verify(bytes, "", "app.tar.gz").is_err());
    }

    #[test]
    fn reads_the_tag_where_the_latest_release_points() {
        let tag = |location: &str| tag_in(location, "victorlcampos/vitals");
        let at = "https://github.com/victorlcampos/vitals/releases/tag/";
        assert_eq!(tag(&format!("{at}v0.2.0")), Some("v0.2.0".into()));
        assert_eq!(
            tag("https://github.com/VictorLCampos/Vitals/releases/tag/v1.0.0"),
            Some("v1.0.0".into())
        );
        assert_eq!(
            tag("/victorlcampos/vitals/releases/tag/v1.0.0%2Bbuild.7"),
            Some("v1.0.0+build.7".into())
        );
        assert_eq!(
            tag(&format!("{at}v0.1.0+build.26.c54f69252a35")),
            Some("v0.1.0+build.26.c54f69252a35".into())
        );
        // Before the first release, it points to the list of releases.
        assert_eq!(
            tag("https://github.com/victorlcampos/vitals/releases"),
            None
        );
        assert_eq!(
            tag("https://github.com/someone/vitals/releases/tag/v1.0.0"),
            None
        );
        assert_eq!(tag(at), None);
        assert_eq!(tag(&format!("{at}v1/assets")), None);
        assert_eq!(tag(&format!("{at}v1%2")), None);
        assert_eq!(tag(""), None);
    }

    #[test]
    fn this_system_has_a_release_target() {
        if cfg!(any(target_os = "macos", target_os = "linux", windows)) {
            assert!(TARGET.is_some());
        }
    }

    #[test]
    fn running_versions_must_be_semantic_versions() {
        assert!(Updater::new("victorlcampos/vitals", "vitals", "0.2.0").is_ok());
        assert!(Updater::new("victorlcampos/vitals", "vitals", "0.2").is_err());
    }

    #[test]
    fn newer_means_a_higher_version() {
        let version = |text| Version::parse(text).unwrap();
        assert!(version("0.10.0") > version("0.9.9"));
        assert!(version("1.0.0") > version("1.0.0-beta.1"));
        assert_eq!(version("0.2.0").cmp(&version("0.2.0")), Ordering::Equal);
    }

    /// Fetches a real release, aditor's, whose archives follow the same
    /// layout, and stops short of installing it:
    /// `cargo test --all-features real_release -- --ignored`.
    #[test]
    #[ignore = "downloads a release from GitHub"]
    fn real_release() {
        let updater = Updater::new("victorlcampos/aditor", "aditor", "0.0.1").unwrap();
        let release = updater.latest().unwrap();
        assert!(release.version > *updater.current());
        let download = updater.download(&release).unwrap();
        assert_eq!(download.version, release.version);
        assert!(download.staged.exists());
        let staged = download.staged.to_path_buf();
        drop(download);
        assert!(
            !staged.exists(),
            "an update not installed leaves nothing behind"
        );
    }
}
