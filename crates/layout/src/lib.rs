//! Layout engine: a vertical strip with two lanes. Pure logic, without Win32.
//!
//! Each window has its own width (`Span`: full, left lane, right lane) and height
//! (1/3, 1/2, full). Windows are arranged in sequence like lines of text:
//!  - `Full` windows are placed below both lanes,
//!  - `Left` / `Right` windows are stacked in their respective lanes.
//! Each window's height is independent. The viewport scrolls to follow focus.
//!
//! Actions: snap (height / side), maximize, focus, and move_window (swap positions
//! with a neighbor in a given direction, regardless of size). Snap layouts (zone
//! presets applied by dragging) are in `snap`.

mod flow;
mod ops;
mod snap;
#[cfg(test)]
mod tests;
mod types;
mod workspaces;

pub use snap::{builtin_presets, SnapPreset, SnapZone, MAX_ZONES};
pub use types::*;
pub use workspaces::{SavedLayout, SavedStrip, Workspaces};
