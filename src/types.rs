//! Public data types for the layout engine (Space-based vertical strip).

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
    /// Custom height: thousandths of the space content height (150..=1000).
    Custom(i32),
}

impl Height {
    /// Height in thousandths of the space content height (150..=1000).
    pub fn permil(self) -> i32 {
        match self {
            Height::Third => 333,
            Height::Half => 500,
            Height::Full => 1000,
            Height::Custom(p) => p.clamp(150, 1000),
        }
    }

    pub fn from_permil(p: i32) -> Self {
        let p = p.clamp(150, 1000);
        match p {
            1000 => Height::Full,
            500 => Height::Half,
            333 => Height::Third,
            other => Height::Custom(other),
        }
    }

    /// Half of this height (splitting a window vertically in two), minimum 15%.
    pub(crate) fn halved(self) -> Self {
        match self {
            Height::Full => Height::Half,
            Height::Half => Height::Custom(250),
            Height::Third => Height::Custom(166),
            Height::Custom(p) => Height::Custom((p / 2).max(150)),
        }
    }

    pub(crate) fn pixels(self, space_h: i32) -> i32 {
        match self {
            Height::Third => space_h / 3,
            Height::Half => space_h / 2,
            Height::Full => space_h,
            Height::Custom(p) => (space_h as i64 * p as i64 / 1000) as i32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Full,
    Left,
    Right,
    /// Hidden (minimized, cloaked, etc.): excluded from the layout, but its slot
    /// (space, `restore` width, top, height) is remembered until it reappears.
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

    /// Logical horizontal interval in [0, 2] for overlap checks.
    pub(crate) fn h_range(self) -> (i32, i32) {
        match self {
            Span::Full => (0, 2),
            Span::Left => (0, 1),
            Span::Right => (1, 2),
            Span::Hidden => (0, 0),
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

/// One window placed inside a `Space`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Item {
    pub id: WindowId,
    pub span: Span,
    /// Vertical start inside its Space, in thousandths of the space height (0..1000).
    pub top: i32,
    pub height: Height,
    /// Span to restore when the window reappears (used when `span` = Hidden).
    pub restore: Span,
    /// Left lane width in thousandths (500 = 50%), kept in sync with the owning `Space`.
    pub split: i32,
}

impl Item {
    pub fn full(id: WindowId) -> Self {
        Self {
            id,
            span: Span::Full,
            top: 0,
            height: Height::Full,
            restore: Span::Full,
            split: 500,
        }
    }

    /// Vertical interval `(top, bottom)` in thousandths (0..=1000).
    pub fn v_range(&self) -> (i32, i32) {
        let top = self.top.clamp(0, 999);
        let bot = (top + self.height.permil()).min(1000);
        // Snap near-bottom (e.g. 333 * 3 = 999) cleanly to 1000.
        let bot = if bot >= 998 { 1000 } else { bot };
        (top, bot)
    }

    pub fn is_full_space(&self) -> bool {
        self.span == Span::Full && self.top == 0 && self.height.permil() >= 998
    }

    /// Relative area inside the Space (0..=2000), used for swap authority.
    pub fn area(&self) -> i32 {
        let (hx0, hx1) = self.span.h_range();
        let (vy0, vy1) = self.v_range();
        (hx1 - hx0) * (vy1 - vy0)
    }

    /// True if two visible items overlap in 2D space coordinates.
    pub fn overlaps_rect(&self, span: Span, top: i32, height: Height) -> bool {
        if self.span == Span::Hidden || span == Span::Hidden {
            return false;
        }
        let (hx0, hx1) = self.span.h_range();
        let (ox0, ox1) = span.h_range();
        if hx0 >= ox1 || ox0 >= hx1 {
            return false;
        }
        let (vy0, vy1) = self.v_range();
        let oy0 = top.clamp(0, 999);
        let oy1 = (oy0 + height.permil()).min(1000);
        let oy1 = if oy1 >= 998 { 1000 } else { oy1 };
        vy0 + 2 < oy1 && oy0 + 2 < vy1
    }
}

/// One full-screen-sized scrolling unit inside a workspace `Strip`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    pub items: Vec<Item>,
    /// Left lane width in thousandths (150..=850, default 500).
    pub split: i32,
}

impl Space {
    pub fn new_full(id: WindowId) -> Self {
        Self {
            items: vec![Item::full(id)],
            split: 500,
        }
    }

    pub fn visible_count(&self) -> usize {
        self.items
            .iter()
            .filter(|it| it.span != Span::Hidden)
            .count()
    }

    pub fn has_overlap(&self, span: Span, top: i32, height: Height, skip: Option<WindowId>) -> bool {
        self.items.iter().any(|it| {
            Some(it.id) != skip && it.overlaps_rect(span, top, height)
        })
    }
}

/// Layout result for one window. `visible = false` means it is entirely
/// outside the viewport (the platform layer may hide or park the window).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub id: WindowId,
    pub rect: Rect,
    pub visible: bool,
}

/// Where to insert a window into a strip (drag in the overview / pick mode).
/// Always relative to a window already in that strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertPosition {
    /// In the opposite lane from this half-width window, aligned with it (same Space).
    Beside(WindowId),
    /// Full-screen new Space directly above this window's Space.
    FullBefore(WindowId),
    /// Full-screen new Space directly below this window's Space.
    FullAfter(WindowId),
    /// Split this window in two inside its Space; `bool` = new window goes first (left / top).
    ///  - full-width window: split width side-by-side (same top/height)
    ///  - half-width window: split height vertically in the same lane
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
    /// Height of one Space in strip coordinates (= `view_h`).
    pub space_h: i32,
    /// Total number of Spaces with visible windows (or total spaces).
    pub space_count: usize,
}

pub struct Strip {
    pub(crate) spaces: Vec<Space>,
    pub(crate) focused: Option<WindowId>,
    pub(crate) scroll_y: i32,
    pub(crate) viewport: Rect,
    /// Minimum height measured from the original window (the app refuses to be resized smaller).
    pub(crate) min_heights: HashMap<WindowId, i32>,
    /// Minimum width measured from the original window (the app refuses to be narrowed).
    pub(crate) min_widths: HashMap<WindowId, i32>,
    /// Spacing between windows and from the Space/screen edge (pixels).
    pub(crate) gap: i32,
    /// Most recent swap of a full-space window, not yet retrieved by the app (see `take_move_cue`).
    pub(crate) cue: Option<(WindowId, Dir)>,
}
