# cerne

The code my Rust apps share ([vitals](https://github.com/victorlcampos/vitals),
[meridian](https://github.com/victorlcampos/meridian),
[grove](https://github.com/victorlcampos/grove),
[rabisco](https://github.com/victorlcampos/rabisco)), so each piece is written, fixed and
tested once instead of copied from app to app.

*Cerne* is Portuguese for heartwood: the core of a tree's trunk, which holds the rest up.

## What's inside

Every part sits behind a feature, and an app builds only the ones it turns on.

| Feature | Module | What it does |
| --- | --- | --- |
| `update` | `cerne::update` | Updates the running executable, or a macOS app bundle, from the latest GitHub release, after checking its SHA-256 |
| `store` | `cerne::store` | Settings in one JSON file, saved on every change and shared by copies running side by side |
| `text` | `cerne::text` | Matching text regardless of case, accents and punctuation |
| `tui` | `cerne::themes`, `cerne::snapshot` | For ratatui apps: the 50 editor themes, and pictures of the screen as text or SVG |

```toml
[dependencies]
cerne = { git = "https://github.com/victorlcampos/cerne", tag = "v0.1.0", features = ["update"] }
```

## Updates

`<app> update` downloads the latest release for the system it runs on, checks it against the
release's `SHA256SUMS`, runs it once to see that it reports the release's version, and only
then puts it in place of the running executable, in one rename. A download cut short, a
tampered file or the wrong version leaves the installed app as it was.

```rust
// `vitals update [--check]`
cerne::update::run("victorlcampos/vitals", "vitals", env!("CARGO_PKG_VERSION"), check)
```

```text
$ vitals update
Looking for a newer vitals…
Downloading vitals 0.3.0 for aarch64-apple-darwin…
Updated vitals 0.2.0 → 0.3.0: /Users/me/.cargo/bin/vitals
```

A macOS app shipped as a zipped bundle (`Rabisco.zip` holding `Rabisco.app`) updates with
`Updater::download_app`: the new bundle is unpacked beside the installed one, its version and
signature are checked, and the two are swapped atomically.

### Releasing a command-line app

The reusable workflow [`release-cli.yml`](.github/workflows/release-cli.yml) builds the app for
macOS (Apple silicon and Intel), Linux (x86_64 and ARM) and Windows, packs each executable
the way `cerne::update` expects it, and publishes the release. In the app's repository,
`.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write

jobs:
  release:
    uses: victorlcampos/cerne/.github/workflows/release-cli.yml@v0.1.0
    with:
      binary: vitals
```

Then releasing is: bump `version` in `Cargo.toml`, commit, and push a tag with that version
(`git tag v0.2.0 && git push origin v0.2.0`). The workflow refuses a tag that does not match.

A release holds `<app>-<target>.tar.gz` (`.zip` on Windows) per system, with the executable
inside as `<app>-<target>/<app>`, plus `SHA256SUMS`. The targets are
`aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`; elsewhere `update` points to
`cargo install`.

`macos-identifier` signs the macOS executables ad hoc under a lasting identifier, so macOS
keeps the permissions given to the app (meridian's calendar access) from one version to the
next.

## Settings

```rust
use cerne::store::{Origin, Persist, Store};

impl Persist for App {
    type State = State; // Serialize + Deserialize, with #[serde(default)]
    fn state(&self) -> State { /* what to save */ }
    fn restore(&mut self, state: &State, origin: Origin) { /* Origin::Start or Origin::Copy */ }
}

let mut store = Store::new(cerne::store::default_path("vitals").unwrap());
store.load(&mut app);
// On every turn of the event loop: takes in what other copies saved, saves what changed here.
store.sync(&mut app)?;
```

The file is written whole or not at all, readable by its owner only, and one that cannot be
read is set aside as `state.json.broken` rather than lost.

## Development

```sh
cargo test --all-features
cargo test --all-features real_release -- --ignored   # downloads a real release from GitHub
```

Changes reach the apps when they move to a new tag of cerne, so a release here never
changes an app by surprise.

## License

[MIT](LICENSE)
