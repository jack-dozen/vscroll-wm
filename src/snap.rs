//! Snap layouts: presets arranging several zones inside one `Space`
//! (like Snap Layouts in Windows 11). Pure logic, without Win32.

use crate::types::*;

/// Lower bound for slot IDs. Original window IDs come from HWNDs (32 significant bits),
/// so they can never be this large.
const SLOT_BASE: u64 = 1 << 60;

impl WindowId {
    /// Fake ID for the unfilled zone `n` of a preset.
    pub fn slot(n: usize) -> WindowId {
        WindowId(SLOT_BASE + n as u64)
    }

    /// True if this is a slot ID (not an original window).
    pub fn is_slot(self) -> bool {
        self.0 >= SLOT_BASE
    }
}

/// Maximum number of zones in one preset.
pub const MAX_ZONES: usize = 8;

/// Minimum height of a zone (thousandths), equal to the `Height::Custom` limit.
const MIN_PERMIL: i32 = 150;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapZone {
    /// `Full`, `Left`, or `Right`.
    pub span: Span,
    pub height: Height,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapPreset {
    pub name: String,
    /// Left lane width in thousandths (500 = 50%), like `Space::split`.
    pub split: i32,
    /// Ordered zones (reading order: top to bottom, left to right).
    pub zones: Vec<SnapZone>,
}

/// Built-in preset: (name, specification in the same format as the configuration file).
pub const BUILTIN: &[(&str, &str)] = &[
    ("full", "F"),
    ("halves", "L, R"),
    ("wide-left", "split=67, L, R"),
    ("wide-right", "split=33, L, R"),
    ("main-stack", "L, R:1/2, R:1/2"),
    ("stack-main", "L:1/2, L:1/2, R"),
    ("quarters", "L:1/2, R:1/2, L:1/2, R:1/2"),
    ("top-split", "F:1/2, L:1/2, R:1/2"),
    ("rows", "F:1/2, F:1/2"),
    ("thirds", "F:1/3, F:1/3, F:1/3"),
];

/// All built-in presets, in the same order as `BUILTIN`.
pub fn builtin_presets() -> Vec<SnapPreset> {
    BUILTIN
        .iter()
        .filter_map(|(name, spec)| SnapPreset::parse(name, spec).ok())
        .collect()
}

fn height_from_permil(p: i32) -> Height {
    Height::from_permil(p)
}

/// "full", "1/2", "40%", or "0.4" -> thousandths of the space content height.
fn parse_height(s: &str) -> Result<Height, String> {
    let s = s.trim();
    let bad = || format!("bad height '{s}' (use full, 1/2, 1/3, 2/3 or 40%)");
    let frac: f64 = if s == "full" {
        1.0
    } else if let Some(p) = s.strip_suffix('%') {
        p.trim().parse::<f64>().map_err(|_| bad())? / 100.0
    } else if let Some((a, b)) = s.split_once('/') {
        let a = a.trim().parse::<f64>().map_err(|_| bad())?;
        let b = b.trim().parse::<f64>().map_err(|_| bad())?;
        if b <= 0.0 {
            return Err(bad());
        }
        a / b
    } else {
        s.parse::<f64>().map_err(|_| bad())?
    };
    if !frac.is_finite() {
        return Err(bad());
    }
    let permil = (frac * 1000.0).round() as i32;
    if !(MIN_PERMIL..=1000).contains(&permil) {
        return Err(format!("height '{s}' must be between 15% and 100%"));
    }
    Ok(height_from_permil(permil))
}

/// "L", "r:1/2", "full:1/3" -> zone.
fn parse_zone(tok: &str) -> Result<SnapZone, String> {
    let (side, height) = match tok.split_once(':') {
        Some((s, h)) => (s.trim(), Some(h)),
        None => (tok.trim(), None),
    };
    let span = match side {
        "l" | "left" => Span::Left,
        "r" | "right" => Span::Right,
        "f" | "full" => Span::Full,
        other => {
            return Err(format!(
                "unknown zone '{other}' (use L, R or F, optionally with :height)"
            ))
        }
    };
    let height = match height {
        Some(h) => parse_height(h)?,
        None => Height::Full,
    };
    Ok(SnapZone { span, height })
}

/// "67" / "66.7%" -> thousandths of the left lane width (15%-85%).
fn parse_split(v: &str) -> Result<i32, String> {
    let v = v.trim().trim_end_matches('%').trim();
    let pct: f64 = v
        .parse()
        .map_err(|_| format!("bad split '{v}' (use a percentage such as split=67)"))?;
    if !pct.is_finite() || !(15.0..=85.0).contains(&pct) {
        return Err(format!("split '{v}' must be between 15 and 85"));
    }
    Ok((pct * 10.0).round() as i32)
}

impl SnapPreset {
    /// Parse a text specification: comma-separated list, ordered top-to-bottom.
    ///   `split=67`        left lane width (percent, 15-85; default 50)
    ///   `L` `R` `F`       zone in the left / right / full-width lane
    ///   `L:1/2`           zone height: full (default), 1/2, 1/3, 2/3, 40%
    pub fn parse(name: &str, spec: &str) -> Result<SnapPreset, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a layout needs a name".to_string());
        }
        let mut split = 500;
        let mut zones = Vec::new();
        for tok in spec.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let lower = tok.to_lowercase();
            if let Some(rest) = lower.strip_prefix("split") {
                let value = rest
                    .trim_start()
                    .strip_prefix('=')
                    .ok_or_else(|| "expected split=NN".to_string())?;
                split = parse_split(value)?;
                continue;
            }
            zones.push(parse_zone(&lower)?);
        }
        if zones.is_empty() {
            return Err("no zones (write e.g. L, R)".to_string());
        }
        if zones.len() > MAX_ZONES {
            return Err(format!("at most {MAX_ZONES} zones per layout"));
        }
        Ok(SnapPreset {
            name: name.to_string(),
            split,
            zones,
        })
    }

    /// Compute `(top, height_permil)` for each zone within one Space, normalizing
    /// vertically if the preset's zones sum to more than 1000‰.
    fn placed_zones(&self) -> Vec<(Span, i32, i32)> {
        let mut y_full = 0i32;
        let mut y_left = 0i32;
        let mut y_right = 0i32;
        let mut raw: Vec<(Span, i32, i32)> = Vec::with_capacity(self.zones.len());
        for z in &self.zones {
            let h = z.height.permil();
            match z.span {
                Span::Full => {
                    let top = y_full.max(y_left).max(y_right);
                    raw.push((Span::Full, top, h));
                    let bot = top + h;
                    y_full = bot;
                    y_left = bot;
                    y_right = bot;
                }
                Span::Left => {
                    let top = y_left;
                    raw.push((Span::Left, top, h));
                    y_left = top + h;
                }
                Span::Right => {
                    let top = y_right;
                    raw.push((Span::Right, top, h));
                    y_right = top + h;
                }
                Span::Hidden => {}
            }
        }
        let max_y = y_full.max(y_left).max(y_right);
        if max_y > 1000 {
            raw.into_iter()
                .map(|(sp, top, h)| {
                    let y0 = (top as i64 * 1000 / max_y as i64) as i32;
                    let y1 = ((top + h) as i64 * 1000 / max_y as i64) as i32;
                    (sp, y0, (y1 - y0).max(1))
                })
                .collect()
        } else {
            raw
        }
    }

    /// Preview of zones in a `w` x `h` box (without gaps).
    pub fn preview(&self, w: i32, h: i32) -> Vec<Rect> {
        let (w, h) = (w.max(1), h.max(1));
        let lw = (w as i64 * self.split as i64 / 1000) as i32;
        self.placed_zones()
            .into_iter()
            .map(|(sp, top, hp)| {
                let (x, rw) = match sp {
                    Span::Full => (0, w),
                    Span::Left => (0, lw),
                    Span::Right => (lw, w - lw),
                    Span::Hidden => (0, 0),
                };
                let y0 = (top as i64 * h as i64 / 1000) as i32;
                let y1 = if top + hp >= 998 {
                    h
                } else {
                    ((top + hp) as i64 * h as i64 / 1000) as i32
                };
                Rect {
                    x,
                    y: y0,
                    w: rw.max(1),
                    h: (y1 - y0).max(1),
                }
            })
            .collect()
    }
}

impl Strip {
    /// IDs of slots still in this strip, ordered by position.
    pub fn slots(&self) -> Vec<WindowId> {
        self.spaces
            .iter()
            .flat_map(|sp| sp.items.iter())
            .filter(|it| it.id.is_slot())
            .map(|it| it.id)
            .collect()
    }

    /// Remove all slots (unfilled zones). True if any were removed.
    pub fn clear_slots(&mut self) -> bool {
        let mut removed = false;
        for sp in &mut self.spaces {
            let before = sp.items.len();
            sp.items.retain(|it| !it.id.is_slot());
            removed |= sp.items.len() != before;
        }
        if !removed {
            return false;
        }
        self.cleanup_empty_spaces();
        if self.focused.is_some_and(|f| f.is_slot()) {
            self.focused = self
                .spaces
                .iter()
                .flat_map(|sp| sp.items.iter())
                .find(|it| it.span != Span::Hidden)
                .map(|it| it.id);
        }
        self.ensure_visible();
        true
    }

    /// Apply `preset` with `id` in zone `zone` inside a single `Space`.
    /// Any other windows that shared `id`'s Space become Full Spaces directly below.
    pub fn apply_snap(&mut self, id: WindowId, preset: &SnapPreset, zone: usize) -> Vec<WindowId> {
        if id.is_slot() || zone >= preset.zones.len() {
            return Vec::new();
        }
        self.clear_slots();

        let at = match self.find_item(id) {
            Some((si, _)) => {
                let old = self.spaces.remove(si);
                let rest: Vec<Space> = old
                    .items
                    .into_iter()
                    .filter(|it| it.id != id)
                    .map(|it| {
                        if it.span == Span::Hidden {
                            let mut h = Item::full(it.id);
                            h.span = Span::Hidden;
                            h.restore = Span::Full;
                            Space {
                                items: vec![h],
                                split: 500,
                            }
                        } else {
                            Space::new_full(it.id)
                        }
                    })
                    .collect();
                for (k, sp) in rest.into_iter().enumerate() {
                    self.spaces.insert(si + k, sp);
                }
                si
            }
            None => self
                .focused
                .and_then(|f| self.find_item(f))
                .map_or(self.spaces.len(), |(si, _)| si + 1),
        };

        self.min_heights.remove(&id);
        self.min_widths.remove(&id);

        let mut slots = Vec::new();
        let mut items = Vec::new();
        for (k, (span, top, hp)) in preset.placed_zones().into_iter().enumerate() {
            let item_id = if k == zone {
                id
            } else {
                let s = WindowId::slot(k);
                slots.push(s);
                s
            };
            items.push(Item {
                id: item_id,
                span,
                top,
                height: Height::from_permil(hp),
                restore: span,
                split: preset.split,
            });
        }
        self.spaces.insert(
            at,
            Space {
                items,
                split: preset.split,
            },
        );
        self.focused = Some(id);
        self.ensure_visible();
        slots
    }

    /// Fill slot `slot` with window `id`, which is not in this strip.
    pub fn fill_slot(&mut self, slot: WindowId, id: WindowId) -> bool {
        if !slot.is_slot() || id.is_slot() || self.find_item(id).is_some() {
            return false;
        }
        let Some((si, ii)) = self.find_item(slot) else {
            return false;
        };
        self.spaces[si].items[ii].id = id;
        self.min_heights.remove(&id);
        self.min_widths.remove(&id);
        self.focused = Some(id);
        self.ensure_visible();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspaces::Workspaces;

    const A: WindowId = WindowId(1);
    const C: WindowId = WindowId(3);
    const W: WindowId = WindowId(9);

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }

    fn strip() -> Strip {
        Strip::new(rect(0, 0, 1000, 600))
    }

    fn preset(spec: &str) -> SnapPreset {
        SnapPreset::parse("t", spec).unwrap()
    }

    fn placed(s: &Strip, id: WindowId) -> Rect {
        s.layout().iter().find(|p| p.id == id).unwrap().rect
    }

    #[test]
    fn slot_ids_are_distinct_from_window_handles() {
        assert!(WindowId::slot(0).is_slot());
        assert!(WindowId::slot(7).is_slot());
        assert!(!WindowId(0x0004_05A2).is_slot());
        assert_ne!(WindowId::slot(0), WindowId::slot(1));
    }

    #[test]
    fn parses_zones_heights_and_split() {
        let p = SnapPreset::parse("x", "split=67, L, R:1/2, F:1/3, l:2/3, r:40%").unwrap();
        assert_eq!(p.name, "x");
        assert_eq!(p.split, 670);
        let z: Vec<(Span, Height)> = p.zones.iter().map(|z| (z.span, z.height)).collect();
        assert_eq!(
            z,
            vec![
                (Span::Left, Height::Full),
                (Span::Right, Height::Half),
                (Span::Full, Height::Third),
                (Span::Left, Height::Custom(667)),
                (Span::Right, Height::Custom(400)),
            ]
        );
    }

    #[test]
    fn rejects_bad_specs() {
        assert!(SnapPreset::parse("x", "").is_err());
        assert!(SnapPreset::parse("x", "split=60").is_err());
        assert!(SnapPreset::parse("x", "Q").is_err());
        assert!(SnapPreset::parse("x", "L:1/0").is_err());
        assert!(SnapPreset::parse("x", "L:5%").is_err());
        assert!(SnapPreset::parse("x", "L:3/2").is_err());
        assert!(SnapPreset::parse("x", "split=99, L").is_err());
        assert!(SnapPreset::parse("", "L").is_err());
        assert!(SnapPreset::parse("x", "L,L,L,L,L,L,L,L,L").is_err());
    }

    #[test]
    fn every_builtin_parses_and_names_are_unique() {
        let all = builtin_presets();
        assert_eq!(all.len(), BUILTIN.len());
        for (i, p) in all.iter().enumerate() {
            assert!(!p.zones.is_empty());
            assert!(all[i + 1..].iter().all(|q| q.name != p.name), "{}", p.name);
        }
    }

    #[test]
    fn preview_uses_space_math() {
        let halves = preset("L, R").preview(1000, 600);
        assert_eq!(halves, vec![rect(0, 0, 500, 600), rect(500, 0, 500, 600)]);

        let quarters = preset("L:1/2, R:1/2, L:1/2, R:1/2").preview(1000, 600);
        assert_eq!(
            quarters,
            vec![
                rect(0, 0, 500, 300),
                rect(500, 0, 500, 300),
                rect(0, 300, 500, 300),
                rect(500, 300, 500, 300),
            ]
        );

        let top_split = preset("F:1/2, L:1/2, R:1/2").preview(1000, 600);
        assert_eq!(
            top_split,
            vec![
                rect(0, 0, 1000, 300),
                rect(0, 300, 500, 300),
                rect(500, 300, 500, 300),
            ]
        );

        let wide = preset("split=67, L, R").preview(1000, 600);
        assert_eq!(wide, vec![rect(0, 0, 670, 600), rect(670, 0, 330, 600)]);

        let thirds = preset("F:1/3, F:1/3, F:1/3").preview(1000, 600);
        assert_eq!(
            thirds,
            vec![
                rect(0, 0, 1000, 199),
                rect(0, 199, 1000, 200),
                rect(0, 399, 1000, 201),
            ]
        );
    }

    #[test]
    fn snapping_the_only_window_reserves_the_other_zones_in_one_space() {
        let mut s = strip();
        s.add_window(W);
        let slots = s.apply_snap(W, &preset("L, R"), 0);
        assert_eq!(slots, vec![WindowId::slot(1)]);
        assert_eq!(s.spaces().len(), 1);
        assert_eq!(placed(&s, W), rect(0, 0, 500, 600));
        assert_eq!(placed(&s, WindowId::slot(1)), rect(500, 0, 500, 600));
    }

    #[test]
    fn other_windows_in_same_space_become_full_spaces_below() {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Left);
        s.add_window(W);
        s.snap(Dir::Right); // A and W share Space 0
        s.add_window(C); // Space 1
        s.set_focus(W);
        s.apply_snap(W, &preset("L, R"), 1);
        assert_eq!(s.spaces().len(), 3);
        assert_eq!(s.spaces()[0].items.len(), 2); // slot(0) + W
        assert_eq!(s.spaces()[1].items[0].id, A);
        assert!(s.spaces()[1].items[0].is_full_space());
        assert_eq!(s.spaces()[2].items[0].id, C);
    }

    #[test]
    fn filling_a_slot_moves_window_from_another_workspace() {
        let mut w = Workspaces::new(rect(0, 0, 1000, 600));
        w.add_window(A);
        assert!(w.switch_to(1));
        w.add_window(W);
        assert_eq!(w.apply_snap(W, &preset("L, R"), 0).len(), 1);
        assert!(w.fill_slot(WindowId::slot(1), A));
        assert_eq!(w.workspace_of(A), Some(w.active()));
        assert_eq!(w.workspace_of(W), Some(w.active()));
        assert!(!w.has_slots());
    }

    #[test]
    fn clear_slots_keeps_filled_zones() {
        let mut s = strip();
        s.add_window(W);
        s.apply_snap(W, &preset("L, R:1/2, R:1/2"), 0);
        assert!(s.clear_slots());
        assert!(!s.clear_slots());
        assert_eq!(s.spaces().len(), 1);
        assert_eq!(s.spaces()[0].items.len(), 1);
        assert_eq!(s.spaces()[0].items[0].id, W);
        assert_eq!(s.spaces()[0].items[0].span, Span::Left);
    }
}
