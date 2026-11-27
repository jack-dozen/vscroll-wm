//! Layout engine: vertical Space-based scrolling strip. Pure logic, without Win32.
//!
//! Each workspace is a vertical strip of full-screen-sized `Space` pages.
//! Inside each `Space`, windows occupy 2D regions (Left/Right/Full × Top/Bottom/Full
//! or custom preset zones), with empty space allowed both horizontally and vertically.

mod flow;
mod ops;
mod snap;
#[cfg(test)]
mod tests;
mod types;
mod workspaces;

pub use snap::{builtin_presets, SnapPreset, SnapZone, MAX_ZONES};
pub use types::*;
pub use workspaces::{SavedLayout, SavedSpace, SavedStrip, Workspaces};
