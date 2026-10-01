//! The code shared by Victor's Rust apps (vitals, meridian, grove, rabisco), so
//! each piece is written, fixed and tested once. Every part sits behind a
//! feature, and an app builds only the ones it turns on:
//!
//! - `update`: updates the running executable, or a macOS app bundle, from the
//!   latest GitHub release, after checking its SHA-256.
//! - `store`: settings in one JSON file, saved on every change and shared by
//!   copies running side by side.
//! - `text`: matching text regardless of case, accents and punctuation.
//! - `tui`: for ratatui apps, the 50 editor themes and pictures of the screen
//!   as text or SVG.

#[cfg(feature = "tui")]
pub mod snapshot;
#[cfg(feature = "store")]
pub mod store;
#[cfg(feature = "text")]
pub mod text;
#[cfg(feature = "tui")]
pub mod themes;
#[cfg(feature = "update")]
pub mod update;
