//! Public data types for the layout engine.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Height {
    Third,
    Half,
    Full,
    /// Custom height from `step` mode: thousandths of the content area height (15-100%).
    Custom(i32),
}

impl Height {
    pub(crate) fn grow(self) -> Self {
        match self {
            Height::Third => Height::Half,
            Height::Custom(p) if p < 333 => Height::Third,
            Height::Custom(p) if p < 500 => Height::Half,
            _ => Height::Full,
        }
    }
    pub(crate) fn shrink(self) -> Self {
        match self {
            Height::Full => Height::Half,
            Height::Custom(p) if p > 500 => Height::Half,
            Height::Custom(p) if p > 333 => Height::Third,
            Height::Custom(_) => self, // Already below 1/3: there is no smaller preset.
            _ => Height::Third,
        }
    }
    /// Half of this height (splitting a stacked window in two): Full -> Half, Half ->
    /// 1/4, otherwise thousandths / 2 (minimum 15%).
    pub(crate) fn halved(self) -> Self {
        match self {
            Height::Full => Height::Half,
            Height::Half => Height::Custom(250),
            Height::Third => Height::Custom(166),
            Height::Custom(p) => Height::Custom((p / 2).max(150)),
        }
    }
    pub(crate) fn pixels(self, viewport_h: i32) -> i32 {
        match self {
            Height::Third => viewport_h / 3,
            Height::Half => viewport_h / 2,
            Height::Full => viewport_h,
            Height::Custom(p) => viewport_h * p / 1000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Full,
    Left,
    Right,
    /// Hidden (minimized, cloaked, etc.): excluded from the layout, but its slot
    /// (order, `restore` width, height) is remembered until it reappears.
    Hidden,
}

impl Span {
    pub(crate) fn opposite(self) -> Span {
        match self {
            Span::Left => Span::Right,
            Span::Right => Span::Left,
            Span::Full => Span::Full,
            Span::Hidden => Span::Hidden,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub id: WindowId,
    pub span: Span,
    pub height: Height,
    /// Width to restore when the window reappears (used when `span` = Hidden).
    pub restore: Span,
    /// Left lane width in thousandths (500 = 50%). Applies to the entire block;
    /// the value from the first lane item in the block is used.
    pub split: i32,
}

/// Layout result for one window. `visible = false` means it is entirely
/// outside the viewport (the platform layer may hide or park the window).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub id: WindowId,
    pub rect: Rect,
    pub visible: bool,
}

/// Where to insert a window into a strip (drag in the overview). Always relative to
/// a window already in that strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertPosition {
    /// In the opposite lane from this half-width window, aligned with it (same block).
    Beside(WindowId),
    /// Full width, above this window's block (block separator).
    FullBefore(WindowId),
    /// Full width, below this window's block (block separator).
    FullAfter(WindowId),
    /// Split this window in two; `bool` = the new window goes on the first side (left / top).
    ///  - full-width window: split the width, side by side (height stays the same)
    ///  - half-width window: split the height, stacked in the same lane
    Split(WindowId, bool),
}

/// One window in the overview map (strip coordinates, without scrolling).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverviewItem {
    pub id: WindowId,
    pub rect: Rect,
}

/// Map of the entire strip for the overview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    pub items: Vec<OverviewItem>,
    /// Total strip width and height.
    pub width: i32,
    pub height: i32,
    /// The part of the strip currently shown on screen: starts at `view_y`, with height `view_h`.
    pub view_y: i32,
    pub view_h: i32,
}

pub struct Strip {
    pub(crate) items: Vec<Item>,
    pub(crate) focused: Option<WindowId>,
    pub(crate) scroll_y: i32,
    pub(crate) viewport: Rect,
    /// Minimum height measured from the original window (the app refuses to be resized smaller).
    pub(crate) min_heights: HashMap<WindowId, i32>,
    /// Minimum width measured from the original window (the app refuses to be narrowed).
    pub(crate) min_widths: HashMap<WindowId, i32>,
    /// Spacing between windows and from the screen edge (pixels).
    pub(crate) gap: i32,
    /// Most recent swap of a full-width window, not yet retrieved by the app (see `take_move_cue`).
    pub(crate) cue: Option<(WindowId, Dir)>,
}
