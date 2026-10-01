//! What an app remembers between runs, in one JSON file saved on every change.
//! Copies running side by side (panes of a tiling window manager) share the
//! file and pick up each other's changes.
//!
//! The app says what it saves through [`Persist`]; a [`Store`] keeps the file
//! and the app in step:
//!
//! ```ignore
//! let mut store = Store::new(cerne::store::default_path("vitals").unwrap());
//! store.load(&mut app);
//! loop {
//!     // Saved on every change, not on exit, so a power cut loses nothing.
//!     app.unsaved = store.sync(&mut app).is_err();
//!     // draw, wait for a key...
//! }
//! ```

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// An app whose state goes into a [`Store`].
pub trait Persist {
    /// What is saved: usually a struct with `#[serde(default)]`, so files
    /// written by older versions, missing newer fields, still load.
    type State: Clone + PartialEq + Serialize + DeserializeOwned;

    /// The state to save now.
    fn state(&self) -> Self::State;

    /// Takes in a saved state.
    fn restore(&mut self, state: &Self::State, origin: Origin);
}

/// Where a restored state comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The file as the app starts.
    Start,
    /// Another copy, which just saved a change. Things that belong to each
    /// copy, such as which tab is on screen, are better left alone.
    Copy,
}

/// `$XDG_CONFIG_HOME/<app>/state.json`, or `~/.config/<app>/state.json`, or
/// `%APPDATA%\<app>\state.json`.
pub fn default_path(app: &str) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
    Some(base.join(app).join("state.json"))
}

/// Keeps the state file and the app in step.
pub struct Store<S> {
    path: PathBuf,
    /// The file's text when last read or written here.
    seen: Option<String>,
    /// What this copy last wrote, to tell its own writes from the others'.
    written: Option<String>,
    /// The app's state as last saved or loaded, to notice changes.
    synced: Option<S>,
}

impl<S: Clone + PartialEq + Serialize + DeserializeOwned> Store<S> {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            seen: None,
            written: None,
            synced: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Restores the saved state. A file that cannot be read as a state is set
    /// aside as `*.broken`, not lost.
    pub fn load<A: Persist<State = S>>(&mut self, app: &mut A) {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return;
        };
        match serde_json::from_str::<S>(&text) {
            Ok(state) => {
                app.restore(&state, Origin::Start);
                self.seen = Some(text);
                self.synced = Some(app.state());
            }
            Err(_) => {
                let _ = fs::rename(&self.path, self.path.with_extension("json.broken"));
            }
        }
    }

    /// Takes in what another copy saved, then saves what changed here.
    pub fn sync<A: Persist<State = S>>(&mut self, app: &mut A) -> io::Result<()> {
        if let Ok(text) = fs::read_to_string(&self.path)
            && self.seen.as_ref() != Some(&text)
        {
            if self.written.as_ref() != Some(&text)
                && let Ok(state) = serde_json::from_str::<S>(&text)
            {
                app.restore(&state, Origin::Copy);
                self.synced = Some(app.state());
            }
            self.seen = Some(text);
        }

        let state = app.state();
        if self.synced.as_ref() == Some(&state) {
            return Ok(());
        }
        let text = serde_json::to_string_pretty(&state).map_err(io::Error::other)? + "\n";
        write_atomically(&self.path, &text)?;
        self.seen = Some(text.clone());
        self.written = Some(text);
        self.synced = Some(state);
        Ok(())
    }
}

/// A crash or power cut leaves either the old file or the new one, whole.
fn write_atomically(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    // Settings can hold secrets (meridian keeps a calendar's secret address
    // there): readable by their owner only.
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temporary)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    struct State {
        theme: String,
        tab: usize,
    }

    /// An app with a theme every copy shares and a tab each copy keeps.
    #[derive(Default)]
    struct App {
        theme: String,
        tab: usize,
    }

    impl Persist for App {
        type State = State;

        fn state(&self) -> State {
            State {
                theme: self.theme.clone(),
                tab: self.tab,
            }
        }

        fn restore(&mut self, state: &State, origin: Origin) {
            self.theme.clone_from(&state.theme);
            if origin == Origin::Start {
                self.tab = state.tab;
            }
        }
    }

    /// A state file path in a fresh directory of its own.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cerne-store-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir.join("state.json")
    }

    fn open(path: &Path) -> (App, Store<State>) {
        let mut app = App::default();
        let mut store = Store::new(path.to_owned());
        store.load(&mut app);
        (app, store)
    }

    #[test]
    fn everything_comes_back_after_a_restart() {
        let path = scratch("restart");
        let (mut first, mut store) = open(&path);
        first.theme = "Nord".into();
        first.tab = 2;
        store.sync(&mut first).unwrap();

        let (second, _) = open(&path);
        assert_eq!((second.theme.as_str(), second.tab), ("Nord", 2));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn copies_running_side_by_side_share_their_changes() {
        let path = scratch("share");
        let (mut left, mut left_store) = open(&path);
        let (mut right, mut right_store) = open(&path);
        left_store.sync(&mut left).unwrap();
        right_store.sync(&mut right).unwrap();

        right.theme = "Dracula".into();
        right.tab = 3;
        right_store.sync(&mut right).unwrap();
        left_store.sync(&mut left).unwrap();
        assert_eq!(left.theme, "Dracula");
        assert_eq!(left.tab, 0, "each copy keeps its own tab");

        left.theme = "Nord".into();
        left_store.sync(&mut left).unwrap();
        right_store.sync(&mut right).unwrap();
        assert_eq!((right.theme.as_str(), right.tab), ("Nord", 3));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn writes_only_when_something_changed() {
        let path = scratch("quiet");
        let (mut app, mut store) = open(&path);
        store.sync(&mut app).unwrap();
        assert!(path.exists());
        fs::remove_file(&path).unwrap();
        store.sync(&mut app).unwrap();
        assert!(!path.exists(), "nothing changed, nothing written");
        app.theme = "Nord".into();
        store.sync(&mut app).unwrap();
        assert!(path.exists());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn sets_a_broken_file_aside_instead_of_losing_it() {
        let path = scratch("broken");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ not json").unwrap();
        let (mut app, mut store) = open(&path);
        let broken = path.with_extension("json.broken");
        assert_eq!(fs::read_to_string(&broken).unwrap(), "{ not json");
        store.sync(&mut app).unwrap();
        assert!(serde_json::from_str::<State>(&fs::read_to_string(&path).unwrap()).is_ok());
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn only_its_owner_can_read_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let path = scratch("private");
        let (mut app, mut store) = open(&path);
        app.theme = "Nord".into();
        store.sync(&mut app).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn the_default_path_is_under_the_config_folder() {
        let path = default_path("vitals").unwrap();
        assert!(path.ends_with("vitals/state.json") || path.ends_with(r"vitals\state.json"));
    }
}
