//! Overview controller (application side; rendering is in overview.rs).

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use layout::{Dir, InsertPosition, WindowId};

#[allow(unused_imports)]
use windows::Win32::UI::Input::KeyboardAndMouse::*;
#[allow(unused_imports)]
use windows::Win32::UI::WindowsAndMessaging::*;

use super::workspace::{slide_dir, Slide};
use super::{App, OverviewSession};
use crate::consts::{DIRS, EVENT_DEBOUNCE_MS};
use crate::hook::{OV_MOVED, OV_OPEN};
use crate::winutil::{title, wid, work_area};
use crate::{config, overview, rules, safety};

impl App {
    /// Win+Alt+Space: open / close the overview.
    pub(crate) unsafe fn toggle_overview(&mut self) {
        if self.overview.is_some() {
            self.close_overview(false);
        } else {
            self.open_overview();
        }
    }

    pub(crate) unsafe fn open_overview(&mut self) {
        if !self.try_open_overview() {
            // Failed to open: undo the "open" state that the hook may have set.
            OV_OPEN.store(false, Ordering::Relaxed);
            OV_MOVED.store(false, Ordering::Relaxed);
        }
    }

    /// All workspaces with visible windows + one empty workspace at the end (so a
    /// window can always be dragged to a new workspace, even when only one is occupied).
    fn overview_maps(&self) -> Vec<(usize, layout::Overview)> {
        let mut maps = self.ws.overviews();
        let active = self.ws.active();
        if !maps.iter().any(|(n, _)| *n == active) {
            if let Some(st) = self.ws.strip(active) {
                maps.push((active, st.overview()));
            }
        }
        let last = self.ws.count().saturating_sub(1);
        if !maps.iter().any(|(n, _)| *n == last) {
            if let Some(st) = self.ws.strip(last) {
                maps.push((last, st.overview()));
            }
        }
        maps.sort_by_key(|(n, _)| *n);
        maps
    }

    /// Daftar window floating / sticky untuk baris pill di overview: `(id, label, sticky)`.
    unsafe fn overview_floating_list(&self) -> Vec<(u64, String, bool)> {
        let mode = self.cfg.overview_label;
        let mut out: Vec<(u64, String, bool)> = self
            .floating
            .iter()
            .filter(|(_, &h)| safety::alive(h))
            .map(|(&id, &h)| {
                let t = title(h);
                let exe = if mode == config::LabelMode::App {
                    rules::exe_name(h)
                } else {
                    None
                };
                let lbl = config::label(mode, &t, exe.as_deref(), &self.cfg.aliases);
                (id.0, lbl, self.sticky.contains_key(&id))
            })
            .collect();
        out.sort_by(|a, b| (!a.2).cmp(&(!b.2)).then_with(|| a.1.cmp(&b.1)));
        out
    }

    pub(crate) unsafe fn try_open_overview(&mut self) -> bool {
        if !overview::available() {
            return false;
        }
        self.snap_reset(true); // Empty slots must not appear in the overview map.
        let maps = self.overview_maps();
        if maps.is_empty() {
            return false;
        }
        // Center column = active workspace (even if empty).
        let active = self.ws.active();
        let center = maps
            .iter()
            .position(|(n, _)| *n == active)
            .unwrap_or(0);
        let selected = self.ws.representative(maps[center].0);
        let mode = self.cfg.overview_label;
        let all = || maps.iter().flat_map(|(_, o)| o.items.iter());
        let titles: HashMap<u64, String> = all()
            .filter_map(|it| {
                self.map.get(&it.id).map(|&h| {
                    let t = title(h);
                    let exe = if mode == config::LabelMode::App {
                        rules::exe_name(h)
                    } else {
                        None
                    };
                    (
                        it.id.0,
                        config::label(mode, &t, exe.as_deref(), &self.cfg.aliases),
                    )
                })
            })
            .collect();
        let sources: HashMap<u64, isize> = all()
            .filter_map(|it| self.map.get(&it.id).map(|&h| (it.id.0, h.0 as isize)))
            .collect();
        let focused = self.ws.cur().focused().map_or(0, |f| f.0);
        let (first, visible) = overview::window_for(maps.len(), center);
        let model = overview::Model {
            maps,
            first,
            visible,
            big: center,
            active_ws: active,
        };
        let floating = self.overview_floating_list();
        let mut draw = overview::build(
            &model,
            &titles,
            &sources,
            selected.map_or(0, |w| w.0),
            focused,
            work_area(),
            &self.overview_options(),
        );
        overview::attach_floating(&mut draw, &floating);
        self.overview = Some(OverviewSession {
            selected,
            prev_fg: GetForegroundWindow(),
            model,
            titles,
            sources,
            focused,
        });
        overview::show(draw, self.cfg.overview_thumbnails);
        OV_MOVED.store(false, Ordering::Relaxed);
        OV_OPEN.store(true, Ordering::Relaxed);
        true
    }

    fn overview_options(&self) -> overview::Options {
        overview::Options {
            hint: self.cfg.overview_hint,
            minimap: self.cfg.overview_minimap,
            size_pct: self.cfg.overview_size,
            colors: self.cfg.overview_colors,
        }
    }

    /// Move the cursor to `n`. If it is in another workspace, make that column the
    /// large one and shift columns as needed (redraw the panel); otherwise just highlight it.
    unsafe fn overview_select(&mut self, n: WindowId) {
        let opts = self.overview_options();
        let thumbs = self.cfg.overview_thumbnails;
        let Some(s) = self.overview.as_mut() else {
            return;
        };
        s.selected = Some(n);
        let col = s
            .model
            .maps
            .iter()
            .position(|(_, o)| o.items.iter().any(|it| it.id == n));
        let Some(col) = col else {
            overview::set_selected(n.0);
            return;
        };
        let first = overview::scroll(s.model.first, s.model.visible, col);
        if col == s.model.big && first == s.model.first {
            overview::set_selected(n.0);
            return;
        }
        s.model.big = col;
        s.model.first = first;
        let mut draw = overview::build(
            &s.model,
            &s.titles,
            &s.sources,
            n.0,
            s.focused,
            work_area(),
            &opts,
        );
        let floating = self.overview_floating_list();
        overview::attach_floating(&mut draw, &floating);
        overview::show(draw, thumbs);
    }

    /// Tab in the overview: move the cursor to the next / previous window, only
    /// within the workspace of the currently selected window (reading order).
    pub(crate) unsafe fn overview_cycle(&mut self, forward: bool) {
        let Some(sel) = self.overview.as_ref().and_then(|s| s.selected) else {
            return;
        };
        let next = self
            .ws
            .workspace_of(sel)
            .and_then(|n| self.ws.strip(n))
            .and_then(|st| st.cycle_from(sel, forward));
        if let Some(n) = next {
            self.overview_select(n);
        }
    }

    /// Arrows in the overview: move the cursor to a neighbor in the same workspace
    /// (the window does not take focus yet). Left/Right at a workspace edge continue
    /// into the neighboring workspace, entering from the opposite edge at the window
    /// nearest the cursor's height. If at an endpoint and `overview_wrap` is enabled,
    /// wrap to the opposite end (first / last workspace for Left/Right; bottom / top
    /// row for Up/Down; if there is only one workspace, Left/Right wrap within it).
    pub(crate) unsafe fn overview_move(&mut self, dir_index: usize) {
        let Some(dir) = DIRS.get(dir_index).map(|d| d.1) else {
            return;
        };
        let Some((sel_opt, cur_big, map_len)) = self
            .overview
            .as_ref()
            .map(|s| (s.selected, s.model.big, s.model.maps.len()))
        else {
            return;
        };

        // Sedang di workspace kosong: Kiri/Kanan langsung pindah ke kolom sebelah.
        let Some(sel) = sel_opt else {
            if matches!(dir, Dir::Left | Dir::Right) && map_len > 1 {
                let forward = dir == Dir::Right;
                let next_col = if forward {
                    if cur_big + 1 < map_len {
                        Some(cur_big + 1)
                    } else if self.cfg.overview_wrap {
                        Some(0)
                    } else {
                        None
                    }
                } else if cur_big > 0 {
                    Some(cur_big - 1)
                } else if self.cfg.overview_wrap {
                    Some(map_len - 1)
                } else {
                    None
                };
                if let Some(col) = next_col {
                    if let Some((ws_num, _)) = self
                        .overview
                        .as_ref()
                        .and_then(|s| s.model.maps.get(col))
                    {
                        self.overview_workspace(2 + *ws_num);
                    }
                }
            }
            return;
        };

        let Some(from) = self.ws.workspace_of(sel) else {
            return;
        };
        if let Some(n) = self.ws.strip(from).and_then(|st| st.neighbor_of(sel, dir)) {
            self.overview_select(n);
            return;
        }
        // Di tepi kiri/kanan: seberangi ke kolom sebelah di overview (termasuk workspace kosong).
        if matches!(dir, Dir::Left | Dir::Right) && map_len > 1 {
            let forward = dir == Dir::Right;
            let next_col = if forward {
                if cur_big + 1 < map_len {
                    Some(cur_big + 1)
                } else if self.cfg.overview_wrap {
                    Some(0)
                } else {
                    None
                }
            } else if cur_big > 0 {
                Some(cur_big - 1)
            } else if self.cfg.overview_wrap {
                Some(map_len - 1)
            } else {
                None
            };
            if let Some(col) = next_col {
                let target_ws = self
                    .overview
                    .as_ref()
                    .and_then(|s| s.model.maps.get(col))
                    .map(|(n, _)| *n);
                if let Some(ws_num) = target_ws {
                    let y = self
                        .ws
                        .strip(from)
                        .and_then(|st| {
                            st.overview()
                                .items
                                .into_iter()
                                .find(|it| it.id == sel)
                                .map(|it| it.rect.y + it.rect.h / 2)
                        })
                        .unwrap_or(0);
                    if let Some(w) = self.ws.entry_window(ws_num, forward, y) {
                        self.overview_select(w);
                    } else {
                        self.overview_workspace(2 + ws_num); // workspace kosong!
                    }
                    return;
                }
            }
        }
        if self.cfg.overview_wrap {
            if let Some(n) = self
                .ws
                .strip(from)
                .and_then(|st| st.wrap_neighbor_of(sel, dir))
            {
                self.overview_select(n);
            }
        }
    }

    /// Win+Ctrl+Left/Right, number keys, dock click, or dock hover during a drag:
    /// switch the stage to another workspace (including the empty one at the end).
    /// `code`: 0 previous, 1 next, n >= 2 workspace number n - 2 (zero-based).
    pub(crate) unsafe fn overview_workspace(&mut self, code: usize) {
        let Some(cur_big) = self.overview.as_ref().map(|s| s.model.big) else {
            return;
        };
        let target_col = {
            let Some(s) = self.overview.as_ref() else {
                return;
            };
            match code {
                0 => cur_big.checked_sub(1),
                1 => (cur_big + 1 < s.model.maps.len()).then_some(cur_big + 1),
                n => {
                    let ws_num = n - 2;
                    s.model.maps.iter().position(|(ws, _)| *ws == ws_num)
                }
            }
        };
        let Some(col) = target_col else {
            return;
        };
        let ws_num = match self.overview.as_ref().and_then(|s| s.model.maps.get(col)) {
            Some((n, _)) => *n,
            None => return,
        };
        if let Some(w) = self.ws.representative(ws_num) {
            self.overview_select(w);
        } else {
            // Empty workspace: show its empty stage (can be entered with Enter / click or used as drop target).
            let opts = self.overview_options();
            let thumbs = self.cfg.overview_thumbnails;
            let floating = self.overview_floating_list();
            let Some(s) = self.overview.as_mut() else {
                return;
            };
            s.selected = None;
            s.model.big = col;
            s.model.first = overview::scroll(s.model.first, s.model.visible, col);
            let mut draw = overview::build(
                &s.model,
                &s.titles,
                &s.sources,
                0,
                s.focused,
                work_area(),
                &opts,
            );
            overview::attach_floating(&mut draw, &floating);
            overview::show(draw, thumbs);
        }
    }

    /// Stage zoom changed (Ctrl+Wheel / +/-/0): rebuild the panel at the new scale
    /// while preserving the current scroll position.
    pub(crate) unsafe fn overview_zoom(&mut self) {
        let opts = self.overview_options();
        let thumbs = self.cfg.overview_thumbnails;
        let frac = overview::current_scroll_frac();
        let floating = self.overview_floating_list();
        let Some(s) = self.overview.as_ref() else {
            return;
        };
        let mut draw = overview::build(
            &s.model,
            &s.titles,
            &s.sources,
            s.selected.map_or(0, |w| w.0),
            s.focused,
            work_area(),
            &opts,
        );
        overview::attach_floating(&mut draw, &floating);
        if let Some(f) = frac {
            draw.apply_scroll_frac(f);
        }
        overview::show(draw, thumbs);
    }

    /// A window is dragged and dropped in the overview (see `overview::take_drop`):
    ///  - in any workspace (including its original one), move it to the marked target:
    ///    split the window under the cursor (full-width -> side-by-side, half-width ->
    ///    stacked), fill the empty column beside a half-width window, or place it
    ///    full-width above / below a block (creating a new block separator).
    /// The actual tiling is updated; redraw the panel and keep it open.
    pub(crate) unsafe fn overview_drop(&mut self) {
        let Some((src, slot)) = overview::take_drop() else {
            return;
        };
        if self.overview.is_none() {
            return;
        }
        let src = WindowId(src);
        let Some(from) = self.ws.workspace_of(src) else {
            return;
        };
        let to = slot.number.saturating_sub(1);
        let changed = match slot.kind {
            overview::SlotKind::Workspace => {
                if to != from {
                    self.ws.move_window_to(src, to)
                } else {
                    false
                }
            }
            other => {
                let at = match other {
                    overview::SlotKind::Beside(t) => InsertPosition::Beside(WindowId(t)),
                    overview::SlotKind::Before(t) => InsertPosition::FullBefore(WindowId(t)),
                    overview::SlotKind::After(t) => InsertPosition::FullAfter(WindowId(t)),
                    overview::SlotKind::Split(t, first) => InsertPosition::Split(WindowId(t), first),
                    overview::SlotKind::Workspace => unreachable!(),
                };
                if to != from {
                    self.ws.move_window_at(src, to, at)
                } else {
                    self.ws.reposition_window(src, at)
                }
            }
        };
        if changed {
            self.apply(); // Park / arrange according to the new layout.
            self.overview_rebuild(src);
        }
    }

    /// Rebuild the panel from the current layout (after moving the window), with the cursor at
    /// `selected`.
    unsafe fn overview_rebuild(&mut self, selected: WindowId) {
        let opts = self.overview_options();
        let thumbs = self.cfg.overview_thumbnails;
        let active = self.ws.active();
        let maps = self.overview_maps();
        if maps.is_empty() {
            return;
        }
        let col = maps
            .iter()
            .position(|(_, o)| o.items.iter().any(|it| it.id == selected))
            .unwrap_or(0);
        let (first, visible) = overview::window_for(maps.len(), col);
        let floating = self.overview_floating_list();
        let Some(s) = self.overview.as_mut() else {
            return;
        };
        s.selected = Some(selected);
        s.model = overview::Model {
            maps,
            first,
            visible,
            big: col,
            active_ws: active,
        };
        let mut draw = overview::build(
            &s.model,
            &s.titles,
            &s.sources,
            selected.0,
            s.focused,
            work_area(),
            &opts,
        );
        overview::attach_floating(&mut draw, &floating);
        overview::show(draw, thumbs);
    }

    pub(crate) unsafe fn overview_pick(&mut self, id: u64) {
        if let Some(s) = self.overview.as_mut() {
            s.selected = Some(WindowId(id));
        }
        self.close_overview(true);
    }

    /// Close the overview. `accept` = jump to the selected window (or empty workspace);
    /// otherwise, restore focus to the previous window.
    pub(crate) unsafe fn close_overview(&mut self, accept: bool) {
        OV_OPEN.store(false, Ordering::Relaxed);
        OV_MOVED.store(false, Ordering::Relaxed);
        let Some(s) = self.overview.take() else {
            return;
        };
        overview::hide();
        if accept {
            let Some(sel) = s.selected else {
                // Workspace kosong terpilih: langsung pindah ke workspace itu.
                if let Some(&(ws_num, _)) = s.model.maps.get(s.model.big) {
                    self.switch_workspace(ws_num);
                }
                return;
            };
            if let Some(&h) = self.floating.get(&sel) {
                if IsIconic(h).as_bool() {
                    let _ = ShowWindow(h, SW_RESTORE);
                }
                let _ = SetForegroundWindow(h);
                self.last_fg = Some(wid(h));
                self.schedule_sync(EVENT_DEBOUNCE_MS);
                return;
            }
            let old = self.ws.active();
            if self.ws.focus_window(sel) {
                // The window is in another workspace: switch with a slide animation.
                self.slide = Some(Slide::switching(slide_dir(old, self.ws.active())));
                self.update_workspace_tip();
            }
            self.apply();
            if let Some(&h) = self.map.get(&sel) {
                let _ = SetForegroundWindow(h);
                self.last_fg = Some(wid(h));
            }
            self.schedule_sync(EVENT_DEBOUNCE_MS); // Update borders and layout after activation.
        } else if safety::alive(s.prev_fg) {
            let _ = SetForegroundWindow(s.prev_fg);
        }
    }
}
