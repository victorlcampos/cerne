//! macOS apps, shipped as a zipped bundle: `Rabisco.zip` holding
//! `Rabisco.app`, made by `ditto -c -k --keepParent Rabisco.app Rabisco.zip`.

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Context, Error, Release, Result, Updater, Version, fail};

impl Updater {
    /// Downloads the app zipped as `asset` from `release` and checks it
    /// against the release's SHA-256, then unpacks it beside the installed
    /// bundle (`installed`, such as /Applications/Rabisco.app) and checks that
    /// it is the release's version and intact, ready to take its place.
    pub fn download_app(
        &self,
        release: &Release,
        asset: &str,
        installed: &Path,
    ) -> Result<StagedApp> {
        let (Some(parent), Some(name)) = (installed.parent(), installed.file_name()) else {
            return fail(format!("{} is not an app bundle", installed.display()));
        };
        let zipped = self.fetch_verified(release, asset)?;
        // Beside the installed bundle, so the swap is a rename on one disk.
        let dir = tempfile::Builder::new()
            .prefix(&format!(".{}-update-", self.bin))
            .tempdir_in(parent)
            .context(|| format!("cannot write to {}", parent.display()))?;
        let zip = dir.path().join(asset);
        fs::write(&zip, &zipped).context(|| format!("write {}", zip.display()))?;
        let unpacked = dir.path().join("unpacked");
        run(
            Command::new("/usr/bin/ditto")
                .args(["-x", "-k"])
                .arg(&zip)
                .arg(&unpacked),
            "unpack the app",
        )?;
        let app = unpacked.join(name);
        let version = bundle_version(&app)?;
        if version != release.version.to_string() {
            return fail(format!(
                "{asset} holds version {version} rather than {}; nothing changed",
                release.version
            ));
        }
        run(
            Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(&app),
            "check the app's signature",
        )?;
        Ok(StagedApp {
            _dir: dir,
            app,
            installed: installed.to_owned(),
            version: release.version.clone(),
        })
    }
}

/// A verified app bundle, waiting beside the installed one.
pub struct StagedApp {
    /// Deleted when this goes away, the replaced bundle with it.
    _dir: tempfile::TempDir,
    app: PathBuf,
    installed: PathBuf,
    pub version: Version,
}

impl StagedApp {
    /// Swaps the new bundle with the installed one in one step and deletes
    /// the old one. A running copy keeps running as it was until it quits.
    pub fn install(self) -> Result<PathBuf> {
        swap(&self.app, &self.installed)
            .context(|| format!("replace {}", self.installed.display()))?;
        Ok(self.installed)
    }
}

/// The bundle's `CFBundleShortVersionString`.
fn bundle_version(app: &Path) -> Result<String> {
    let plist = app.join("Contents/Info.plist");
    if !plist.is_file() {
        return fail(format!(
            "the release has no {}",
            app.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleShortVersionString"])
        .arg(&plist)
        .output()
        .context(|| "read the app's version".to_owned())?;
    if !output.status.success() {
        return fail("the downloaded app has no version");
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Exchanges two folders atomically (APFS and HFS+ can), or else with two
/// renames, putting the first back if the second fails.
fn swap(new: &Path, old: &Path) -> io::Result<()> {
    let path = |path: &Path| CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other);
    let (from, to) = (path(new)?, path(old)?);
    // SAFETY: both are valid NUL-terminated paths that outlive the call.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_SWAP) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if !matches!(error.raw_os_error(), Some(libc::ENOTSUP | libc::EINVAL)) {
        return Err(error);
    }
    let aside = new.with_extension("old");
    fs::rename(old, &aside)?;
    if let Err(error) = fs::rename(new, old) {
        let _ = fs::rename(&aside, old);
        return Err(error);
    }
    fs::rename(&aside, new)
}

fn run(command: &mut Command, doing: &str) -> Result<()> {
    let output = command.output().context(|| doing.to_owned())?;
    if output.status.success() {
        return Ok(());
    }
    Err(Error(format!(
        "{doing}: {}; nothing changed",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(dir: &Path, version: &str) -> PathBuf {
        let app = dir.join("Demo.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(
            app.join("Contents/Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleShortVersionString</key><string>{version}</string>
</dict></plist>
"#
            ),
        )
        .unwrap();
        fs::write(app.join("Contents/MacOS/demo"), version).unwrap();
        app
    }

    #[test]
    fn reads_the_bundle_version() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            bundle_version(&bundle(dir.path(), "1.2.3")).unwrap(),
            "1.2.3"
        );
        assert!(bundle_version(&dir.path().join("Missing.app")).is_err());
    }

    #[test]
    fn swaps_bundles_in_one_step() {
        let dir = tempfile::tempdir().unwrap();
        let installed = bundle(&dir.path().join("Applications"), "1.0.0");
        let new = bundle(&dir.path().join("staged"), "2.0.0");
        swap(&new, &installed).unwrap();
        assert_eq!(bundle_version(&installed).unwrap(), "2.0.0");
        assert_eq!(
            bundle_version(&new).unwrap(),
            "1.0.0",
            "the old one, to delete"
        );
        assert!(swap(&dir.path().join("Missing.app"), &installed).is_err());
        assert_eq!(bundle_version(&installed).unwrap(), "2.0.0");
    }

    #[test]
    fn a_staged_app_replaces_the_installed_one_and_cleans_up() {
        let dir = tempfile::tempdir().unwrap();
        let installed = bundle(&dir.path().join("Applications"), "1.0.0");
        let staging = tempfile::tempdir_in(dir.path()).unwrap();
        let app = bundle(staging.path(), "2.0.0");
        let staged_dir = staging.path().to_owned();
        let staged = StagedApp {
            _dir: staging,
            app,
            installed: installed.clone(),
            version: Version::new(2, 0, 0),
        };
        assert_eq!(staged.install().unwrap(), installed);
        assert_eq!(bundle_version(&installed).unwrap(), "2.0.0");
        assert!(
            !staged_dir.exists(),
            "the old bundle went with the staging folder"
        );
    }
}
