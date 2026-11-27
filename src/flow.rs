//! Position calculations: Space-based layout, scrolling, and read-only queries.

use crate::types::*;

/// Window position in strip coordinates (before subtracting `scroll_y`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Flow {
    pub(crate) id: WindowId,
    pub(crate) space: usize,
    pub(crate) span: Span,
    /// Horizontal position and width in the content area.
    pub(crate) x: i32,
    pub(crate) w: i32,
    pub(crate) top: i32,
    pub(crate) h: i32,
}

impl Flow {
    pub(crate) fn bottom(&self) -> i32 {
        self.top + self.h
    }
    pub(crate) fn x_range(&self) -> (i32, i32) {
        (self.x, self.x + self.w)
    }
}

impl Strip {
    /// Effective Space content area: work area minus half the gap on each side.
    /// Each window is inset by another half-gap, so:
    /// - outer margin around every Space = `gap`
    /// - gap between adjacent windows inside a Space = `gap`
    /// - adjacent Spaces are spaced by `content().h`, so off-screen Spaces never peek.
    pub(crate) fn content(&self) -> Rect {
        let g = self.gap / 2;
        Rect {
            x: self.viewport.x + g,
            y: self.viewport.y + g,
            w: (self.viewport.w - 2 * g).max(1),
            h: (self.viewport.h - 2 * g).max(1),
        }
    }

    /// Find `(space_idx, item_idx)` for window `id`.
    pub(crate) fn find_item(&self, id: WindowId) -> Option<(usize, usize)> {
        for (si, sp) in self.spaces.iter().enumerate() {
            if let Some(ii) = sp.items.iter().position(|it| it.id == id) {
                return Some((si, ii));
            }
        }
        None
    }

    /// Left lane width (content pixels) for one Space. Clamped to 15%-85%, then constrained
    /// by the minimum window width in each non-empty lane.
    pub(crate) fn space_left_width(&self, space: &Space, cw: i32) -> i32 {
        let gap = 2 * (self.gap / 2);
        let lane_min = |target: Span| {
            space
                .items
                .iter()
                .filter(|it| it.span == target)
                .filter_map(|it| self.min_widths.get(&it.id))
                .max()
                .map_or(0, |m| m + gap)
        };
        let has_left = space.items.iter().any(|it| it.span == Span::Left);
        let has_right = space.items.iter().any(|it| it.span == Span::Right);
        let mut lw = (cw as i64 * space.split as i64 / 1000) as i32;
        lw = lw.clamp(cw * 15 / 100, cw * 85 / 100);
        if has_left {
            lw = lw.max(lane_min(Span::Left));
        }
        if has_right {
            lw = lw.min(cw - lane_min(Span::Right));
        }
        lw.clamp(0, cw)
    }

    /// Arrange all visible Spaces in strip coordinates.
    /// Spaces containing only hidden windows take no vertical space until a window reappears.
    pub(crate) fn flow(&self) -> Vec<Flow> {
        let c = self.content();
        let ch = c.h;
        let cw = c.w;
        let mut out = Vec::new();
        let mut vis_space_idx = 0i32;

        for (si, space) in self.spaces.iter().enumerate() {
            let has_visible = space.items.iter().any(|it| it.span != Span::Hidden);
            let space_top = vis_space_idx * ch;
            if has_visible {
                vis_space_idx += 1;
            }
            let lw = self.space_left_width(space, cw);

            for it in &space.items {
                if it.span == Span::Hidden {
                    out.push(Flow {
                        id: it.id,
                        space: si,
                        span: Span::Hidden,
                        x: 0,
                        w: 0,
                        top: space_top,
                        h: 0,
                    });
                    continue;
                }
                let (x, w) = match it.span {
                    Span::Full => (0, cw),
                    Span::Left => (0, lw),
                    Span::Right => (lw, cw - lw),
                    Span::Hidden => unreachable!(),
                };
                let (vy0, vy1) = it.v_range();
                let y0 = (ch as i64 * vy0 as i64 / 1000) as i32;
                let y1 = if vy1 >= 1000 {
                    ch
                } else {
                    (ch as i64 * vy1 as i64 / 1000) as i32
                };
                let mut h = (y1 - y0).max(1);
                let mut top_in_space = y0;

                // Respect measured minimum height while staying inside the Space bounds.
                if let Some(&min_h) = self.min_heights.get(&it.id) {
                    let min_slot = (min_h + 2 * (self.gap / 2)).min(ch);
                    if h < min_slot {
                        h = min_slot;
                        if top_in_space + h > ch {
                            top_in_space = (ch - h).max(0);
                        }
                    }
                }

                out.push(Flow {
                    id: it.id,
                    space: si,
                    span: it.span,
                    x,
                    w: w.max(1),
                    top: space_top + top_in_space,
                    h,
                });
            }
        }
        out
    }

    /// Number of visible Spaces (spaces with at least one non-hidden window).
    pub(crate) fn visible_space_count(&self) -> usize {
        self.spaces
            .iter()
            .filter(|sp| sp.items.iter().any(|it| it.span != Span::Hidden))
            .count()
    }

    /// Snap `scroll_y` cleanly to the Space containing the focused window.
    pub(crate) fn ensure_visible(&mut self) {
        let ch = self.content().h;
        let vis_count = self.visible_space_count();
        if vis_count == 0 {
            self.scroll_y = 0;
            return;
        }
        let max_scroll = ((vis_count as i32 - 1) * ch).max(0);
        let Some(fid) = self.focused else {
            self.scroll_y = self.scroll_y.clamp(0, max_scroll);
            return;
        };
        let mut vis_idx = 0i32;
        for sp in &self.spaces {
            let has_visible = sp.items.iter().any(|it| it.span != Span::Hidden);
            if sp
                .items
                .iter()
                .any(|it| it.id == fid && it.span != Span::Hidden)
            {
                self.scroll_y = (vis_idx * ch).clamp(0, max_scroll);
                return;
            }
            if has_visible {
                vis_idx += 1;
            }
        }
        self.scroll_y = self.scroll_y.clamp(0, max_scroll);
    }

    /// Nearest neighboring window in direction `dir`, based on screen position.
    pub(crate) fn neighbor(&self, dir: Dir) -> Option<WindowId> {
        self.neighbor_of(self.focused?, dir)
    }

    /// Nearest neighbor of `id` in direction `dir`, without changing focus.
    pub fn neighbor_of(&self, id: WindowId, dir: Dir) -> Option<WindowId> {
        let flow = self.flow();
        let f = flow.iter().find(|c| c.id == id && c.span != Span::Hidden)?;
        let (fx0, fx1) = f.x_range();
        let (fy0, fy1) = (f.top, f.bottom());

        let mut best: Option<((bool, i32, i32, i32), WindowId)> = None;
        for c in flow.iter().filter(|c| c.id != id && c.span != Span::Hidden) {
            let (cx0, cx1) = c.x_range();
            let (cy0, cy1) = (c.top, c.bottom());
            let h_ov = (fx1.min(cx1) - fx0.max(cx0)).max(0);
            let v_ov = (fy1.min(cy1) - fy0.max(cy0)).max(0);
            let key = match dir {
                Dir::Up if cy1 <= fy0 => Some((h_ov == 0, fy0 - cy1, -h_ov, c.top)),
                Dir::Down if cy0 >= fy1 => Some((h_ov == 0, cy0 - fy1, -h_ov, c.top)),
                Dir::Left if c.space == f.space && cx1 <= fx0 => {
                    Some((v_ov == 0, fx0 - cx1, -v_ov, (cy0 - fy0).abs()))
                }
                Dir::Right if c.space == f.space && cx0 >= fx1 => {
                    Some((v_ov == 0, cx0 - fx1, -v_ov, (cy0 - fy0).abs()))
                }
                _ => None,
            };
            if let Some(k) = key {
                if best.map_or(true, |(bk, _)| k < bk) {
                    best = Some((k, c.id));
                }
            }
        }
        best.map(|(_, n)| n)
    }

    /// Wrap-around neighbor for overview arrow navigation.
    pub fn wrap_neighbor_of(&self, id: WindowId, dir: Dir) -> Option<WindowId> {
        let flow = self.flow();
        let f = flow.iter().find(|c| c.id == id && c.span != Span::Hidden)?;
        let (fx0, fx1) = f.x_range();
        let (fy0, fy1) = (f.top, f.bottom());
        let f_mid = (fy0 + fy1) / 2;
        let shown = || flow.iter().filter(|c| c.span != Span::Hidden);
        let first_top = shown().map(|c| c.top).min()?;
        let last_bottom = shown().map(|c| c.bottom()).max()?;
        let first_x = shown().map(|c| c.x_range().0).min()?;
        let last_x = shown().map(|c| c.x_range().1).max()?;

        let mut best: Option<((bool, i32, i32, i32), WindowId)> = None;
        for c in shown().filter(|c| c.id != id) {
            let (cx0, cx1) = c.x_range();
            let (cy0, cy1) = (c.top, c.bottom());
            let h_ov = (fx1.min(cx1) - fx0.max(cx0)).max(0);
            let v_ov = (fy1.min(cy1) - fy0.max(cy0)).max(0);
            let dy = ((cy0 + cy1) / 2 - f_mid).abs();
            let key = match dir {
                Dir::Up if cy0 >= fy1 => Some((h_ov == 0, last_bottom - cy1, -h_ov, cy0)),
                Dir::Down if cy1 <= fy0 => Some((h_ov == 0, cy0 - first_top, -h_ov, cy0)),
                Dir::Left if cx0 >= fx1 => Some((v_ov == 0, last_x - cx1, dy, cy0)),
                Dir::Right if cx1 <= fx0 => Some((v_ov == 0, cx0 - first_x, dy, cy0)),
                _ => None,
            };
            if let Some(k) = key {
                if best.map_or(true, |(bk, _)| k < bk) {
                    best = Some((k, c.id));
                }
            }
        }
        best.map(|(_, n)| n)
    }

    /// Next (or previous) window in reading order: space by space, top to bottom, left to right.
    pub fn cycle_from(&self, id: WindowId, forward: bool) -> Option<WindowId> {
        let mut order: Vec<(i32, i32, WindowId)> = self
            .flow()
            .iter()
            .filter(|f| f.span != Span::Hidden)
            .map(|f| (f.top, f.x_range().0, f.id))
            .collect();
        order.sort_by_key(|&(top, x, _)| (top, x));
        let n = order.len();
        let pos = order.iter().position(|&(_, _, w)| w == id)?;
        if n < 2 {
            return None;
        }
        let next = if forward {
            (pos + 1) % n
        } else {
            (pos + n - 1) % n
        };
        Some(order[next].2)
    }

    /// Map of the entire strip (including off-screen Spaces) for the overview.
    pub fn overview(&self) -> Overview {
        let c = self.content();
        let items: Vec<OverviewItem> = self
            .flow()
            .iter()
            .filter(|f| f.span != Span::Hidden)
            .map(|f| {
                let (x0, x1) = f.x_range();
                OverviewItem {
                    id: f.id,
                    rect: Rect {
                        x: x0,
                        y: f.top,
                        w: x1 - x0,
                        h: f.h,
                    },
                }
            })
            .collect();
        let space_count = self.visible_space_count();
        let height = (space_count as i32 * c.h).max(c.h);
        Overview {
            items,
            width: c.w,
            height,
            view_y: self.scroll_y,
            view_h: c.h,
            space_h: c.h,
            space_count,
        }
    }

    pub fn layout(&self) -> Vec<Placement> {
        let vp = self.viewport;
        let c = self.content();
        let g = self.gap / 2;
        self.flow()
            .into_iter()
            .filter(|f| f.span != Span::Hidden)
            .map(|f| {
                let top = c.y + f.top - self.scroll_y;
                let (x, w) = (c.x + f.x, f.w);
                let rect = Rect {
                    x: x + g,
                    y: top + g,
                    w: (w - 2 * g).max(1),
                    h: (f.h - 2 * g).max(1),
                };
                Placement {
                    id: f.id,
                    rect,
                    visible: rect.y + rect.h > vp.y && rect.y < vp.y + vp.h,
                }
            })
            .collect()
    }
}
