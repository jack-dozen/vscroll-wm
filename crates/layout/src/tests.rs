use super::*;

const A: WindowId = WindowId(1);
const B: WindowId = WindowId(2);
const C: WindowId = WindowId(3);
const X: WindowId = WindowId(4);
const D: WindowId = WindowId(5);

fn strip() -> Strip {
    Strip::new(Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    })
}

fn r(s: &Strip, id: WindowId) -> Rect {
    s.layout().into_iter().find(|p| p.id == id).unwrap().rect
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect { x, y, w, h }
}

#[test]
fn new_window_goes_below_and_scrolls() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert_eq!(s.focused(), Some(B));
    assert_eq!(r(&s, B).y, 0);
    assert_eq!(r(&s, A).y, -1080);
    let a = s.layout().into_iter().find(|p| p.id == A).unwrap();
    assert!(!a.visible);
}

#[test]
fn up_shrinks_and_down_grows() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    assert_eq!(r(&s, A).h, 540);
    s.snap(Dir::Up);
    assert_eq!(r(&s, A).h, 360);
    s.snap(Dir::Down);
    s.snap(Dir::Down);
    assert_eq!(r(&s, A).h, 1080);
}

#[test]
fn snap_left_gives_half_width_and_leaves_right_free() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    assert_eq!(r(&s, A), rect(0, 0, 960, 540));
}

#[test]
fn snapping_same_side_again_is_noop() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    let before = s.items().to_vec();
    s.snap(Dir::Left);
    assert_eq!(s.items(), &before[..]);
}

#[test]
fn opposite_side_goes_full_width_first_then_other_side() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.snap(Dir::Right); // Full width first; keep the height.
    assert_eq!(r(&s, A), rect(0, 0, 1920, 540));
    s.snap(Dir::Right); // Then move to the right.
    assert_eq!(r(&s, A), rect(960, 0, 960, 540));
}

#[test]
fn heights_are_independent_per_window() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right); // B stays full-height.
    assert_eq!(r(&s, A), rect(0, 0, 960, 540));
    assert_eq!(r(&s, B), rect(960, 0, 960, 1080));
}

#[test]
fn swap_when_opposite_lane_is_free() {
    // A is in the top-left quarter, with the right side empty. Snap B left -> B top-left, A top-right.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Left);
    assert_eq!(r(&s, B), rect(0, 0, 960, 1080));
    assert_eq!(r(&s, A), rect(960, 0, 960, 540));
}

/// Y: right half of the screen (full height). X: top-left quarter.
fn y_right_x_top_left() -> Strip {
    let mut s = strip();
    s.add_window(A); // A = Y.
    s.snap(Dir::Right);
    s.add_window(X);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s
}

#[test]
fn fills_gap_below_occupant_without_moving_it() {
    let mut s = y_right_x_top_left();
    assert_eq!(r(&s, X), rect(0, 0, 960, 540));
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    // B fills the empty space below X; X and A (full-height on the right) do not move.
    assert_eq!(r(&s, X), rect(0, 0, 960, 540));
    assert_eq!(r(&s, B), rect(0, 540, 960, 540));
    assert_eq!(r(&s, A), rect(960, 0, 960, 1080));
}

/// Position in strip coordinates (without scrolling).
fn at(s: &Strip, id: WindowId) -> Rect {
    s.overview().items.into_iter().find(|i| i.id == id).unwrap().rect
}

/// A is full-height on the left; B is half-height at the top right (empty space below B).
fn a_left_full_b_right_half() -> Strip {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s
}

#[test]
fn full_window_snapped_right_goes_below_shorter_occupant() {
    // A is full-height on the left; B is half-height at the top right; C is full-width below the block.
    let mut s = a_left_full_b_right_half();
    s.add_window(C);
    s.snap(Dir::Right);
    // A and B do not move; C is below B and stays full-height.
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(960, 0, 960, 540));
    assert_eq!(at(&s, C), rect(960, 540, 960, 1080));
}

#[test]
fn full_window_snapped_left_goes_below_shorter_occupant_mirrored() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Right);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(C);
    s.snap(Dir::Left);
    assert_eq!(at(&s, A), rect(960, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(0, 0, 960, 540));
    assert_eq!(at(&s, C), rect(0, 540, 960, 1080));
}

#[test]
fn window_shorter_than_the_gap_keeps_its_height() {
    let mut s = a_left_full_b_right_half();
    s.add_window(C);
    s.snap(Dir::Up);
    s.snap(Dir::Up); // C: 1/3.
    s.snap(Dir::Right);
    assert_eq!(at(&s, B), rect(960, 0, 960, 540));
    assert_eq!(at(&s, C), rect(960, 540, 960, 360));
}

#[test]
fn gap_fill_goes_below_every_window_of_the_lane() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Up);
    s.snap(Dir::Right); // B: right, 1/3.
    s.add_window(D);
    s.snap(Dir::Up);
    s.snap(Dir::Up);
    s.snap(Dir::Right); // D: 1/3 below B.
    assert_eq!(at(&s, D), rect(960, 360, 960, 360));
    s.add_window(C);
    s.snap(Dir::Right);
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(960, 0, 960, 360));
    assert_eq!(at(&s, D), rect(960, 360, 960, 360));
    assert_eq!(at(&s, C), rect(960, 720, 960, 1080));
}

#[test]
fn gap_fill_ignores_hidden_windows_in_the_lane() {
    let mut s = a_left_full_b_right_half();
    s.add_window(D);
    s.snap(Dir::Up);
    s.snap(Dir::Right); // D below B.
    assert!(s.set_hidden(D, true));
    s.set_focus(B);
    s.add_window(C);
    s.snap(Dir::Right);
    assert_eq!(at(&s, B), rect(960, 0, 960, 540));
    assert_eq!(at(&s, C), rect(960, 540, 960, 1080));
}

#[test]
fn gap_fill_works_with_a_gap_setting() {
    let mut s = a_left_full_b_right_half();
    s.set_gap(10);
    s.add_window(C);
    s.snap(Dir::Right);
    let (a, b, c) = (r(&s, A), r(&s, B), r(&s, C));
    assert_eq!(b.y, a.y);
    assert_eq!(c.x, b.x);
    assert_eq!(c.y, b.y + b.h + 10); // Directly below B, separated by the gap.
    assert_eq!(c.h, a.h); // Remains full-height.
}

#[test]
fn maximize_restores_full_size() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.maximize();
    assert_eq!(r(&s, A), rect(0, 0, 1920, 1080));
}

#[test]
fn maximize_resets_the_lane_ratio_so_snapping_again_gives_half() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    assert!(s.adjust_split(96));
    assert_eq!(r(&s, A).w, 1056); // Divider has moved from 1/2.
    s.maximize();
    assert_eq!(r(&s, A), rect(0, 0, 1920, 1080)); // Back to the original layout.
    s.snap(Dir::Left);
    assert_eq!(r(&s, A).w, 960); // Back to 1/2, not the previous ratio.
}

#[test]
fn rejoining_a_block_after_maximize_still_follows_that_blocks_ratio() {
    let mut s = two_half_lanes();
    s.set_focus(A);
    assert!(s.adjust_split(96)); // Block A+B: left 1056, right 864.
    s.set_focus(B);
    s.maximize();
    assert_eq!(r(&s, B).w, 1920);
    s.snap(Dir::Right); // Re-enter a block with a divider: use its ratio.
    assert_eq!(r(&s, B).w, 864);
    assert_eq!(r(&s, A).w, 1056);
}

#[test]
fn moving_a_full_width_window_leaves_a_cue_for_the_app() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B); // B is focused, below A (both are full-width).
    s.move_window(Dir::Up);
    assert_eq!(s.take_move_cue(), Some((B, Dir::Up)));
    assert_eq!(s.take_move_cue(), None); // One-shot.
}

#[test]
fn lane_swaps_and_failed_moves_leave_no_cue() {
    let mut s = two_half_lanes();
    s.set_focus(A);
    s.move_window(Dir::Right); // Swap sides: already visible on screen.
    assert_eq!(s.take_move_cue(), None);
    let mut s = strip();
    s.add_window(A);
    s.move_window(Dir::Down); // No neighbor.
    assert_eq!(s.take_move_cue(), None);
}

#[test]
fn min_height_stretches_aligned_neighbor_to_keep_edges_even() {
    // A and B are both half-height and side by side. B has a minimum height of 600:
    // stretch A to 600 so their bottom edges align.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    assert!(s.set_min_height(B, 600));
    assert_eq!(r(&s, B).h, 600);
    assert_eq!(r(&s, A).h, 600);
    assert!(!s.set_min_height(B, 500));
}

#[test]
fn quarter_grid_rows_stay_aligned_with_min_height() {
    // 2x2: A B / C D. B has a minimum height -> both rows stay aligned.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(C);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(D);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.set_min_height(B, 600);
    let (a, b, c, d) = (r(&s, A), r(&s, B), r(&s, C), r(&s, D));
    assert_eq!(a.y, b.y);
    assert_eq!(a.h, b.h);
    assert_eq!(c.y, d.y);
    assert_eq!(c.y - a.y, 600);
    assert_eq!(c.y + c.h, d.y + d.h);
}

#[test]
fn new_window_goes_below_whole_block_without_pushing_neighbors() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.set_focus(A);
    s.add_window(C);
    assert_eq!(r(&s, B).y, r(&s, A).y); // B does not move down.
    assert_eq!(r(&s, C).y - r(&s, A).y, 540);
}

#[test]
fn min_height_pushes_window_below_in_same_lane() {
    let mut s = y_right_x_top_left();
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Left); // Left: X (top), B (bottom).
    s.set_focus(X);
    s.set_min_height(X, 700);
    assert_eq!(r(&s, X).h, 700);
    assert_eq!(r(&s, B).y, r(&s, X).y + 700);
}

#[test]
fn min_height_is_capped_to_viewport() {
    let mut s = strip();
    s.add_window(A);
    s.set_min_height(A, 5000);
    assert_eq!(r(&s, A).h, 1080);
}

#[test]
fn focus_follows_screen_geometry() {
    let mut s = y_right_x_top_left();
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Left); // Left: X (top), B (bottom). Right: A.
    assert_eq!(s.focused(), Some(B));
    s.focus(Dir::Up);
    assert_eq!(s.focused(), Some(X));
    s.focus(Dir::Down);
    assert_eq!(s.focused(), Some(B));
    s.focus(Dir::Right);
    assert_eq!(s.focused(), Some(A));
    s.focus(Dir::Left); // Two candidates are equally close: choose the top one.
    assert_eq!(s.focused(), Some(X));
}

#[test]
fn remove_moves_focus_and_clamps_scroll() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    s.add_window(C);
    s.remove_window(C);
    assert_eq!(s.items().len(), 2);
    assert_eq!(s.focused(), Some(B));
    assert_eq!(r(&s, B).y, 0);
}

#[test]
fn vertical_focus_scrolls_back_up() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    s.focus(Dir::Up);
    assert_eq!(s.focused(), Some(A));
    assert_eq!(r(&s, A).y, 0);
}

#[test]
fn full_width_goes_below_window_beside_it() {
    // A top-left, B top-right. Make A full-width -> move it below B.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.set_focus(A);
    s.snap(Dir::Right);
    assert_eq!(r(&s, A), rect(0, 540, 1920, 540));
    assert_eq!(r(&s, B), rect(960, 0, 960, 540));
}

#[test]
fn goes_below_when_quarter_row_is_complete() {
    // A top-left, B top-right (quarter row is full). Put C in the left quarter -> below A.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(C);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    assert_eq!(r(&s, A), rect(0, 0, 960, 540));
    assert_eq!(r(&s, B), rect(960, 0, 960, 540));
    assert_eq!(r(&s, C), rect(0, 540, 960, 540));
}

#[test]
fn maximize_goes_below_neighbors_without_dragging_them() {
    // Left: B (top), A (bottom). Right: C, half-screen wide and full-height.
    let mut s = strip();
    s.add_window(C);
    s.snap(Dir::Right);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left); // A fills the space below B.
    s.set_focus(A);
    s.maximize();
    assert_eq!(r(&s, A).w, 1920);
    assert_eq!(r(&s, A).h, 1080);
    assert_eq!(r(&s, A).y - r(&s, C).y, 1080); // Below A and C.
    assert_eq!(r(&s, C).y, r(&s, B).y); // C does not move down.
}

#[test]
fn move_window_with_full_width_neighbor_keeps_sizes() {
    // A is full-width and half-height; B is below it on the left, C on the right.
    // Move B up: B stays a quarter, A stays full-width, and C moves below A.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(C);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.set_focus(B);
    s.move_window(Dir::Up);
    assert_eq!(s.focused(), Some(B));
    assert_eq!(r(&s, B), rect(0, 0, 960, 540));
    assert_eq!(r(&s, A), rect(0, 540, 1920, 540));
    assert_eq!(r(&s, C).y, 1080);
}

#[test]
fn move_window_in_same_lane_swaps_positions_not_sizes() {
    // Left: W (full-height) at the top, X (half-height) below. Right: Y.
    let mut s = strip();
    s.add_window(A); // A = Y.
    s.snap(Dir::Right);
    s.add_window(B); // B = W.
    s.snap(Dir::Left); // Left lane is still empty: full-height.
    s.add_window(X);
    s.snap(Dir::Up);
    s.snap(Dir::Left); // No empty space: below W.
    s.set_focus(X);
    let (wh, xh) = (r(&s, B).h, r(&s, X).h);
    assert_eq!((wh, xh), (1080, 540));
    s.move_window(Dir::Up);
    assert_eq!(r(&s, X).h, 540);
    assert_eq!(r(&s, B).h, 1080);
    assert_eq!(r(&s, B).y - r(&s, X).y, 540); // X is now above B.
    assert_eq!(r(&s, X).y, r(&s, A).y);
}

#[test]
fn move_window_across_lanes_swaps_sides_not_heights() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Up);
    s.snap(Dir::Right); // B is one-third width on the right.
    s.set_focus(A);
    s.move_window(Dir::Right);
    assert_eq!(r(&s, A), rect(960, 0, 960, 540));
    assert_eq!(r(&s, B), rect(0, 0, 960, 360));
}

#[test]
fn changing_height_forgets_measured_min_height() {
    let mut s = strip();
    s.add_window(A);
    s.set_min_height(A, 600);
    assert_eq!(r(&s, A).h, 1080);
    s.snap(Dir::Up);
    assert_eq!(r(&s, A).h, 540); // Forget the old minimum and remeasure.
}

#[test]
fn gap_around_single_window() {
    let mut s = strip();
    s.set_gap(20);
    s.add_window(A);
    assert_eq!(r(&s, A), rect(20, 20, 1880, 1040));
}

#[test]
fn gap_between_lanes_equals_gap_setting() {
    let mut s = strip();
    s.set_gap(20);
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    let (a, b) = (r(&s, A), r(&s, B));
    assert_eq!(b.x - (a.x + a.w), 20);
    assert_eq!(a.x, 20); // Distance to the edge equals the gap.
    assert_eq!(1920 - (b.x + b.w), 20);
}

#[test]
fn gap_between_rows_equals_gap_setting() {
    let mut s = strip();
    s.set_gap(20);
    s.add_window(A);
    s.snap(Dir::Up);
    s.add_window(B);
    s.snap(Dir::Up);
    let (a, b) = (r(&s, A), r(&s, B));
    assert_eq!(b.y - (a.y + a.h), 20);
}

#[test]
fn min_height_accounts_for_gap() {
    let mut s = strip();
    s.set_gap(20);
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Up);
    s.set_min_height(A, 600);
    assert_eq!(r(&s, A).h, 600);
}

#[test]
fn min_height_mismatch_is_evened_out_before_full_window() {
    // A is half-height on the left, B half-height on the right, and C full-width below.
    // B has a larger minimum height -> stretch A so their bottom edges align.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(C);
    s.set_min_height(B, 600);
    assert_eq!(r(&s, B).h, 600);
    assert_eq!(r(&s, A).h, 600);
    assert_eq!(r(&s, C).y - r(&s, B).y, 600);
}

#[test]
fn intentionally_unaligned_lanes_are_not_stretched() {
    // A is half-height on the left, B one-third height on the right: they are not aligned.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(C);
    assert_eq!(r(&s, A).h, 540);
    assert_eq!(r(&s, B).h, 360);
}

#[test]
fn empty_lane_is_not_filled() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(C);
    assert_eq!(r(&s, A).h, 540);
    assert_eq!(r(&s, C).y - r(&s, A).y, 540);
}

#[test]
fn hidden_window_frees_its_space_and_returns_to_same_slot() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    assert_eq!(r(&s, B), rect(960, 0, 960, 540));
    assert!(s.set_hidden(B, true));
    assert_eq!(s.layout().len(), 1);
    assert_eq!(s.focused(), Some(A));
    assert!(s.set_hidden(B, false));
    assert_eq!(r(&s, B), rect(960, 0, 960, 540));
    assert!(!s.set_hidden(B, false)); // Already visible.
}

#[test]
fn hiding_everything_then_showing_restores_the_whole_layout() {
    // 2x2 plus one full-width window below, then "Win+D" and restore.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(C);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(D);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(X);
    let ids = [A, B, C, D, X];
    let shape = |s: &Strip| -> Vec<(i32, i32, i32, i32)> {
        let ay = r(s, A).y;
        ids.iter()
            .map(|&i| {
                let q = r(s, i);
                (q.x, q.y - ay, q.w, q.h)
            })
            .collect()
    };
    let before = shape(&s);
    for &i in &ids {
        s.set_hidden(i, true);
    }
    assert!(s.layout().is_empty());
    assert_eq!(s.focused(), None);
    for &i in ids.iter().rev() {
        s.set_hidden(i, false); // Windows do not have to reappear in the same order.
    }
    assert_eq!(shape(&s), before);
    assert!(s.focused().is_some());
}

#[test]
fn hidden_window_is_skipped_by_focus_and_removal() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    s.add_window(C);
    s.set_hidden(B, true);
    s.remove_window(C);
    assert_eq!(s.focused(), Some(A)); // B is hidden and is not selected.
    s.focus(Dir::Down);
    assert_eq!(s.focused(), Some(A)); // There is no visible neighbor.
    s.set_focus(B); // Ignored while hidden.
    assert_eq!(s.focused(), Some(A));
}

#[test]
fn overview_describes_whole_strip_including_offscreen_part() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(C);
    s.snap(Dir::Up);
    let ov = s.overview();
    let at = |id| ov.items.iter().find(|i| i.id == id).unwrap().rect;
    assert_eq!((ov.width, ov.height), (1920, 1080));
    assert_eq!(at(A), rect(0, 0, 960, 540));
    assert_eq!(at(B), rect(960, 0, 960, 540));
    assert_eq!(at(C), rect(0, 540, 1920, 540));

    s.add_window(D); // Another full screen below.
    let ov = s.overview();
    assert_eq!(ov.height, 2160);
    assert_eq!(ov.view_y, 1080); // Only D is visible.
    assert_eq!(ov.view_h, 1080);
}

#[test]
fn overview_skips_hidden_windows() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    s.set_hidden(B, true);
    assert_eq!(s.overview().items.len(), 1);
}

#[test]
fn neighbor_of_does_not_change_focus() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    assert_eq!(s.focused(), Some(B));
    assert_eq!(s.neighbor_of(A, Dir::Right), Some(B));
    assert_eq!(s.neighbor_of(B, Dir::Left), Some(A));
    assert_eq!(s.neighbor_of(A, Dir::Up), None);
    assert_eq!(s.focused(), Some(B));
}

#[test]
fn cycle_follows_reading_order_and_wraps() {
    // A top-left, B top-right, C full-width below.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(C);
    s.snap(Dir::Up);
    assert_eq!(s.cycle_from(A, true), Some(B));
    assert_eq!(s.cycle_from(B, true), Some(C));
    assert_eq!(s.cycle_from(C, true), Some(A)); // Wraps around.
    assert_eq!(s.cycle_from(A, false), Some(C));
    assert_eq!(s.cycle_from(C, false), Some(B));
    assert_eq!(s.focused(), Some(C)); // Focus does not change.
}

#[test]
fn cycle_skips_hidden_and_needs_two_windows() {
    let mut s = strip();
    s.add_window(A);
    assert_eq!(s.cycle_from(A, true), None);
    s.add_window(B);
    s.add_window(C);
    s.set_hidden(B, true);
    assert_eq!(s.cycle_from(A, true), Some(C));
    assert_eq!(s.cycle_from(B, true), None); // Hidden.
}

/// [A(left), B(full), C(left)]: two blocks separated by B.
fn blocks() -> Strip {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.add_window(C);
    s.snap(Dir::Left);
    s
}

fn order(s: &Strip) -> Vec<(WindowId, Span)> {
    s.items().iter().map(|i| (i.id, i.span)).collect()
}

#[test]
fn insert_beside_takes_the_opposite_lane_of_that_block() {
    let mut s = blocks();
    assert_eq!(
        order(&s),
        vec![(A, Span::Left), (B, Span::Full), (C, Span::Left)]
    );
    assert!(s.insert_window(D, InsertPosition::Beside(A)));
    assert_eq!(
        order(&s),
        vec![(A, Span::Left), (D, Span::Right), (B, Span::Full), (C, Span::Left)]
    );
    assert_eq!(s.focused(), Some(D));

    // Lower block: beside C.
    let mut s = blocks();
    assert!(s.insert_window(D, InsertPosition::Beside(C)));
    assert_eq!(
        order(&s),
        vec![(A, Span::Left), (B, Span::Full), (C, Span::Left), (D, Span::Right)]
    );
}

#[test]
fn insert_beside_matches_the_row_when_the_other_lane_has_windows() {
    // Left: A, A2; right: E. Beside A2 -> second in the right lane (below E).
    let make_strip = || {
        let mut s = strip();
        s.add_window(A);
        s.snap(Dir::Left);
        assert!(s.insert_window(B, InsertPosition::Split(A, false)));
        assert!(s.insert_window(C, InsertPosition::Beside(A)));
        s
    };
    let mut s = make_strip();
    assert!(s.insert_window(D, InsertPosition::Beside(B)));
    let right: Vec<WindowId> = s
        .items()
        .iter()
        .filter(|i| i.span == Span::Right)
        .map(|i| i.id)
        .collect();
    assert_eq!(right, vec![C, D]);
    // Beside A -> first in order, above C.
    let mut s2 = make_strip();
    assert!(s2.insert_window(D, InsertPosition::Beside(A)));
    let right: Vec<WindowId> = s2
        .items()
        .iter()
        .filter(|i| i.span == Span::Right)
        .map(|i| i.id)
        .collect();
    assert_eq!(right, vec![D, C]);
}

#[test]
fn insert_full_goes_to_block_boundaries_without_splitting_blocks() {
    // Above the first block / below the last block.
    let mut s = blocks();
    assert!(s.insert_window(D, InsertPosition::FullBefore(A)));
    assert_eq!(s.items()[0].id, D);
    assert_eq!(s.items()[0].span, Span::Full);
    let mut s = blocks();
    assert!(s.insert_window(D, InsertPosition::FullAfter(C)));
    assert_eq!(s.items()[3].id, D);
    assert_eq!(s.items()[3].span, Span::Full);
    // Below A's block (half-width window): before B, not in the middle of the block.
    let mut s = blocks();
    assert!(s.insert_window(D, InsertPosition::FullAfter(A)));
    assert_eq!(
        order(&s),
        vec![(A, Span::Left), (D, Span::Full), (B, Span::Full), (C, Span::Left)]
    );
    // Relative to a full-width window.
    let mut s = blocks();
    assert!(s.insert_window(D, InsertPosition::FullBefore(B)));
    assert_eq!(s.items()[1].id, D);
    let mut s = blocks();
    assert!(s.insert_window(D, InsertPosition::FullAfter(B)));
    assert_eq!(s.items()[2].id, D);
}

#[test]
fn split_halves_a_full_window_then_stacks_in_the_lane() {
    // One full-width window: drop above it -> side by side, each half-width.
    let mut s = strip();
    s.add_window(A);
    assert!(s.insert_window(B, InsertPosition::Split(A, false)));
    assert_eq!(order(&s), vec![(A, Span::Left), (B, Span::Right)]);
    assert_eq!(s.items()[0].height, Height::Full);
    assert_eq!(s.items()[1].height, Height::Full);

    // Drop on the right side (B): B and the new window are stacked, each half-height.
    assert!(s.insert_window(C, InsertPosition::Split(B, false)));
    assert_eq!(
        order(&s),
        vec![(A, Span::Left), (B, Span::Right), (C, Span::Right)]
    );
    assert_eq!(s.items()[1].height, Height::Half);
    assert_eq!(s.items()[2].height, Height::Half);

    // Drop on A: four equal-sized windows.
    assert!(s.insert_window(D, InsertPosition::Split(A, false)));
    assert_eq!(
        order(&s),
        vec![
            (A, Span::Left),
            (D, Span::Left),
            (B, Span::Right),
            (C, Span::Right)
        ]
    );
    assert!(s.items().iter().all(|i| i.height == Height::Half));
}

#[test]
fn split_first_puts_the_new_window_left_or_above() {
    let mut s = strip();
    s.add_window(A);
    assert!(s.insert_window(B, InsertPosition::Split(A, true)));
    assert_eq!(order(&s), vec![(A, Span::Right), (B, Span::Left)]);

    // On the top half of a half-width window: place the new window above it.
    let mut s = strip();
    s.add_window(A);
    assert!(s.insert_window(B, InsertPosition::Split(A, false))); // A left, B right.
    assert!(s.insert_window(C, InsertPosition::Split(B, true)));
    let right: Vec<WindowId> = s
        .items()
        .iter()
        .filter(|i| i.span == Span::Right)
        .map(|i| i.id)
        .collect();
    assert_eq!(right, vec![C, B]); // C above B.
}

#[test]
fn halving_goes_below_half_for_nested_splits() {
    let mut s = strip();
    s.add_window(A);
    assert!(s.insert_window(B, InsertPosition::Split(A, false)));
    assert!(s.insert_window(C, InsertPosition::Split(B, false))); // B, C: 1/2.
    assert!(s.insert_window(D, InsertPosition::Split(C, false))); // C, D: 1/4.
    let h = |id| s.items().iter().find(|i| i.id == id).unwrap().height;
    assert_eq!(h(B), Height::Half);
    assert_eq!(h(C), Height::Custom(250));
    assert_eq!(h(D), Height::Custom(250));
}

#[test]
fn split_rejects_unknown_and_hidden_targets() {
    let mut s = strip();
    s.add_window(A);
    assert!(!s.insert_window(B, InsertPosition::Split(WindowId(99), false)));
    assert!(!s.insert_window(A, InsertPosition::Split(A, false))); // Already exists.
    s.add_window(C);
    s.set_hidden(C, true);
    assert!(!s.insert_window(B, InsertPosition::Split(C, false)));
    assert_eq!(s.items().len(), 2);
}

#[test]
fn insert_rejects_invalid_requests() {
    let mut s = blocks();
    assert!(!s.insert_window(D, InsertPosition::Beside(B))); // B is full-width.
    assert!(!s.insert_window(D, InsertPosition::Beside(WindowId(99)))); // Unknown.
    assert!(!s.insert_window(A, InsertPosition::FullAfter(C))); // Already exists.
    assert_eq!(s.items().len(), 3);
}

#[test]
fn swap_windows_exchanges_order_or_sides() {
    // One full lane: swap order.
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    s.add_window(C);
    assert!(s.swap_windows(A, C));
    let order: Vec<WindowId> = s.items().iter().map(|i| i.id).collect();
    assert_eq!(order, vec![C, B, A]);
    assert!(!s.swap_windows(A, A));
    assert!(!s.swap_windows(A, D)); // Unknown.

    // Different lanes: swap sides, keep order.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right);
    assert!(s.swap_windows(A, B));
    assert_eq!(s.items()[0].id, A);
    assert_eq!(s.items()[0].span, Span::Right);
    assert_eq!(s.items()[1].span, Span::Left);

    // Hidden windows are excluded.
    s.set_hidden(B, true);
    assert!(!s.swap_windows(A, B));
}

#[test]
fn wrap_neighbor_jumps_to_the_opposite_end() {
    // A top-left, B top-right, C full-width below.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(C);
    s.snap(Dir::Up);
    // Up from the top row -> bottom row.
    assert_eq!(s.wrap_neighbor_of(A, Dir::Up), Some(C));
    assert_eq!(s.wrap_neighbor_of(B, Dir::Up), Some(C));
    // Down from the bottom row -> top row (A or B; both span C's width).
    assert!(matches!(s.wrap_neighbor_of(C, Dir::Down), Some(w) if w == A || w == B));
    // Left from the left edge -> right lane, same row; vice versa for Right.
    assert_eq!(s.wrap_neighbor_of(A, Dir::Left), Some(B));
    assert_eq!(s.wrap_neighbor_of(B, Dir::Right), Some(A));
    // A full-width window has no "opposite side" horizontally.
    assert_eq!(s.wrap_neighbor_of(C, Dir::Left), None);
    assert_eq!(s.wrap_neighbor_of(C, Dir::Right), None);
    assert_eq!(s.focused(), Some(C)); // Focus does not change.
}

#[test]
fn wrap_neighbor_in_a_single_column_matches_reading_order() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    s.add_window(C); // Three full-width windows stacked.
    assert_eq!(s.wrap_neighbor_of(A, Dir::Up), Some(C));
    assert_eq!(s.wrap_neighbor_of(C, Dir::Down), Some(A));
    assert_eq!(s.wrap_neighbor_of(B, Dir::Left), None);
    assert_eq!(s.wrap_neighbor_of(B, Dir::Right), None);
}

#[test]
fn wrap_neighbor_needs_two_visible_windows() {
    let mut s = strip();
    s.add_window(A);
    for d in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
        assert_eq!(s.wrap_neighbor_of(A, d), None);
    }
    s.add_window(B);
    s.set_hidden(B, true);
    assert_eq!(s.wrap_neighbor_of(A, Dir::Up), None); // B is hidden.
    assert_eq!(s.wrap_neighbor_of(B, Dir::Up), None); // ID is hidden.
    assert_eq!(s.wrap_neighbor_of(D, Dir::Up), None); // Unknown ID.
}

fn two_half_lanes() -> Strip {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s
}

#[test]
fn adjust_split_moves_the_lane_divider() {
    let mut s = two_half_lanes();
    s.set_focus(A);
    assert!(s.adjust_split(96));
    assert_eq!(r(&s, A), rect(0, 0, 1056, 540));
    assert_eq!(r(&s, B), rect(1056, 0, 864, 540));
    s.set_focus(B);
    assert!(s.adjust_split(-192)); // From 1056 to 864.
    assert_eq!(r(&s, A).w, 864);
    assert_eq!(r(&s, B), rect(864, 0, 1056, 540));
}

#[test]
fn divider_stops_at_the_global_limits_without_piling_up() {
    let mut s = two_half_lanes();
    s.set_focus(A);
    assert!(s.adjust_split(500));
    assert!(s.adjust_split(500));
    assert_eq!(r(&s, A).w, 1632); // 85%.
    assert!(!s.adjust_split(500)); // Already at the limit.
    assert!(s.adjust_split(-96)); // Can immediately move back.
    assert_eq!(r(&s, A).w, 1536);
}

#[test]
fn divider_belongs_to_the_lane_so_stacked_windows_follow() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(C);
    s.snap(Dir::Up);
    s.snap(Dir::Left); // C below A, in the left lane.
    s.set_focus(A);
    assert!(s.adjust_split(96));
    assert_eq!(r(&s, A).w, 1056);
    assert_eq!(r(&s, C).w, 1056);
    assert_eq!(r(&s, B).w, 864);
}

#[test]
fn divider_is_stored_per_block() {
    // Block 1: A B. Full-width window C. Block 2: D X.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.add_window(C);
    s.snap(Dir::Up);
    s.add_window(D);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(X);
    s.snap(Dir::Up);
    s.snap(Dir::Right);
    s.set_focus(A);
    assert!(s.adjust_split(96));
    assert_eq!(r(&s, A).w, 1056);
    assert_eq!((r(&s, D).w, r(&s, X).w), (960, 960)); // Block 2 is unaffected.
    s.set_focus(D);
    assert!(s.adjust_split(-96));
    assert_eq!(r(&s, D).w, 864);
    assert_eq!(r(&s, A).w, 1056); // Block 1 is unaffected.
    assert_eq!(r(&s, C).w, 1920); // Full-width windows have no divider.
}

#[test]
fn full_width_window_has_no_divider() {
    let mut s = strip();
    s.add_window(A);
    assert!(!s.adjust_split(96));
    assert!(!s.reset_split());
}

#[test]
fn reset_returns_to_even_split() {
    let mut s = two_half_lanes();
    s.set_focus(A);
    s.adjust_split(300);
    assert!(r(&s, A).w > 960);
    assert!(s.reset_split());
    assert_eq!(r(&s, A).w, 960);
    assert!(!s.reset_split()); // Already 50/50.
}

#[test]
fn app_minimum_width_pins_the_divider() {
    let mut s = two_half_lanes();
    s.set_focus(A);
    assert!(s.set_min_width(B, 1000));
    assert_eq!((r(&s, A).w, r(&s, B).w, r(&s, B).x), (920, 1000, 920));
    assert!(s.adjust_split(100)); // Remeasure the right lane.
    assert!(r(&s, B).w < 1000);
    assert!(s.set_min_width(B, 1000)); // After remeasurement, the app still refuses.
    assert_eq!(r(&s, A).w, 920);
    assert!(!s.adjust_split(100)); // Does not move or accumulate.
}

#[test]
fn min_width_is_capped_to_the_content_width() {
    let mut s = two_half_lanes();
    assert!(s.set_min_width(B, 99_999));
    assert!(r(&s, B).w <= 1920);
}

#[test]
fn divider_shows_up_in_overview_and_neighbor_lookup() {
    let mut s = two_half_lanes();
    s.set_focus(A);
    s.adjust_split(96);
    let ov = s.overview();
    let at = |id| ov.items.iter().find(|i| i.id == id).unwrap().rect;
    assert_eq!(at(A), rect(0, 0, 1056, 540));
    assert_eq!(at(B), rect(1056, 0, 864, 540));
    assert_eq!(s.neighbor_of(A, Dir::Right), Some(B));
    assert_eq!(s.neighbor_of(B, Dir::Left), Some(A));
}

#[test]
fn gap_between_unequal_lanes_stays_constant() {
    let mut s = two_half_lanes();
    s.set_gap(20);
    s.set_focus(A);
    s.adjust_split(96);
    let (a, b) = (r(&s, A), r(&s, B));
    assert_eq!(b.x - (a.x + a.w), 20);
    assert_eq!(a.x, 20);
    assert_eq!(1920 - (b.x + b.w), 20);
}

#[test]
fn step_height_moves_by_pixels() {
    let mut s = strip();
    assert!(!s.adjust_height(-80)); // No windows yet.
    s.add_window(A);
    assert!(s.adjust_height(-100));
    assert!((r(&s, A).h - 980).abs() <= 1);
    assert_eq!(r(&s, A).w, 1920); // Still full-width.
}

#[test]
fn step_height_magnets_to_presets() {
    let mut s = strip();
    s.add_window(A);
    assert!(s.adjust_height(-500)); // 1080 -> 580, near 1/2.
    assert_eq!(s.items()[0].height, Height::Half);
    // Decrease by 80 px from full height; eventually lands exactly at 1/2.
    let mut s = strip();
    s.add_window(A);
    let mut n = 0;
    while s.items()[0].height != Height::Half && n < 20 {
        s.adjust_height(-80);
        n += 1;
    }
    assert_eq!(s.items()[0].height, Height::Half);
    assert_eq!(r(&s, A).h, 540);
}

#[test]
fn step_height_always_leaves_the_current_preset() {
    for step in [10, 80, 400] {
        for ups in 0..3 {
            let mut s = strip();
            s.add_window(A);
            for _ in 0..ups {
                s.snap(Dir::Up);
            }
            let before = s.items()[0].height;
            assert!(s.adjust_height(-step), "step {step}, ups {ups}");
            assert_ne!(s.items()[0].height, before);
        }
    }
}

#[test]
fn step_height_is_clamped_to_15_percent_and_full() {
    let mut s = strip();
    s.add_window(A);
    for _ in 0..30 {
        s.adjust_height(-300);
    }
    assert_eq!(r(&s, A).h, 162);
    assert!(!s.adjust_height(-300));
    assert!(s.adjust_height(10_000));
    assert_eq!(s.items()[0].height, Height::Full);
    assert!(!s.adjust_height(100));
}

#[test]
fn step_height_does_not_pile_up_against_app_minimum() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up); // 1/2 = 540.
    s.set_min_height(A, 600);
    assert_eq!(r(&s, A).h, 600);
    for _ in 0..5 {
        s.adjust_height(-80); // Start from the actual 600, not 540.
        s.set_min_height(A, 600); // The app still refuses anything below 600.
        assert_eq!(r(&s, A).h, 600);
    }
    assert!(matches!(
        s.items()[0].height,
        Height::Half | Height::Custom(481)
    ));
}

#[test]
fn custom_height_snaps_to_next_preset_and_scales_with_viewport() {
    let mut s = strip();
    s.add_window(A);
    s.adjust_height(-80); // Custom(925).
    s.set_viewport(rect(0, 0, 1920, 540));
    assert!((r(&s, A).h - 500).abs() <= 1); // Proportional, not a fixed pixel value.
    s.set_viewport(rect(0, 0, 1920, 1080));
    s.snap(Dir::Up);
    assert_eq!(s.items()[0].height, Height::Half);

    let mut s = strip();
    s.add_window(A);
    s.adjust_height(-80);
    s.snap(Dir::Down);
    assert_eq!(s.items()[0].height, Height::Full);
}

#[test]
fn join_full_windows_splits_side_by_side_in_reading_order() {
    // T above (Up) -> focus on the right.
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert!(s.join(Dir::Up));
    assert_eq!(s.focused(), Some(B));
    assert_eq!(r(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(r(&s, B), rect(960, 0, 960, 1080));

    // T below (Down) -> focus on the left.
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    s.set_focus(A);
    assert!(s.join(Dir::Down));
    assert_eq!(s.focused(), Some(A));
    assert_eq!(r(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(r(&s, B), rect(960, 0, 960, 1080));
}

#[test]
fn join_third_window_into_half_width_stacks_in_lane() {
    // A left, B right; C full-width below -> join Up stacks below A (stack-main).
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert!(s.join(Dir::Up));
    s.add_window(C);
    assert!(s.join(Dir::Up));
    assert_eq!(s.focused(), Some(C));
    assert_eq!(r(&s, A), rect(0, 0, 960, 540));
    assert_eq!(r(&s, C), rect(0, 540, 960, 540));
    assert_eq!(r(&s, B), rect(960, 0, 960, 1080));

    // Different lanes: join B (right) to the left -> stack above A (same top).
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert!(s.join(Dir::Up));
    s.set_focus(B);
    assert!(s.join(Dir::Left));
    assert_eq!(r(&s, B), rect(0, 0, 960, 540));
    assert_eq!(r(&s, A), rect(0, 540, 960, 540));
}

#[test]
fn join_into_lane_with_empty_space_fills_gap_without_halving() {
    // C is full-width at the top; below it is a block: A half-height on the left (empty space below), B full-height on the right.
    let mut s = strip();
    s.add_window(C);
    s.add_window(A);
    s.snap(Dir::Up);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right);
    s.set_focus(C);

    assert!(s.join(Dir::Down));
    // A is not split; C fills the space below A.
    assert_eq!(at(&s, A), rect(0, 0, 960, 540));
    assert_eq!(at(&s, C), rect(0, 540, 960, 1080));
    assert_eq!(at(&s, B), rect(960, 0, 960, 1080));
}

#[test]
fn join_in_same_lane_is_rejected() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert!(s.join(Dir::Up));
    s.add_window(C);
    assert!(s.join(Dir::Up)); // A (top) and C (bottom) in the left lane.
    let before = s.items().to_vec();

    s.set_focus(C);
    assert!(!s.join(Dir::Up));
    assert_eq!(s.items(), &before[..]);

    s.set_focus(A);
    assert!(!s.join(Dir::Down));
    assert_eq!(s.items(), &before[..]);
}

#[test]
fn join_rejects_at_minimum_height_and_preserves_layout() {
    // Target is already 1/4 (Custom(250): another split would exceed the 15% limit -> reject.
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert!(s.join(Dir::Up));
    s.set_focus(A);
    s.snap(Dir::Up); // Half.
    s.set_focus(B);
    s.snap(Dir::Up); // Half.
    // Split A and B so both lanes contain 1/4 + 1/4 (with no empty space).
    s.add_window(X);
    assert!(s.reinsert_window(X, InsertPosition::Split(A, false)));
    s.add_window(D);
    assert!(s.reinsert_window(D, InsertPosition::Split(B, false)));
    let target_w = WindowId(99);
    s.add_window(target_w);
    let before = s.items().to_vec();
    assert!(!s.join(Dir::Up));
    assert_eq!(s.items(), &before[..]);
}

#[test]
fn join_rejects_when_min_height_does_not_fit() {
    // Target A has min_height 600 (> 540 = half the screen): reject; preserve the layout.
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert!(s.join(Dir::Up));
    s.set_min_height(A, 600);
    s.add_window(C);
    let before = s.items().to_vec();
    assert!(!s.join(Dir::Up));
    assert_eq!(s.items(), &before[..]);

    // The focused window (C) has min_height 600: reject this too.
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert!(s.join(Dir::Up));
    s.add_window(C);
    s.set_min_height(C, 600);
    let before = s.items().to_vec();
    assert!(!s.join(Dir::Up));
    assert_eq!(s.items(), &before[..]);
}

#[test]
fn join_without_neighbor_returns_false() {
    let mut s = strip();
    s.add_window(A);
    for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
        assert!(!s.join(dir));
    }
    s.add_window(B);
    assert!(!s.join(Dir::Left));
    assert!(!s.join(Dir::Right));
    assert!(!s.join(Dir::Down));
}

#[test]
fn join_into_can_target_non_adjacent_window_and_pick_right_lane() {
    // A, B, C, D are full-width: join D (bottommost) directly to A (topmost),
    // skipping B and C without changing them.
    let mut s = strip();
    for w in [A, B, C, D] {
        s.add_window(w);
    }
    assert!(s.join_into(D, A));
    assert_eq!(s.focused(), Some(D));
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, D), rect(960, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(0, 1080, 1920, 1080));
    assert_eq!(at(&s, C), rect(0, 2160, 1920, 1080));

    // Aim C directly at the RIGHT lane (D), not the left (A).
    assert!(s.join_into(C, D));
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, D), rect(960, 0, 960, 540));
    assert_eq!(at(&s, C), rect(960, 540, 960, 540));
    assert_eq!(at(&s, B), rect(0, 1080, 1920, 1080));

    // Join to itself -> reject.
    let before = s.items().to_vec();
    assert!(!s.join_into(C, C));
    assert_eq!(s.items(), &before[..]);
}

#[test]
fn join_into_allows_same_side_across_different_blocks() {
    // Block 1: A (left), B (right). Separator: X (full-width). Block 2: C (left), D (right).
    // C (left lane of block 2) can be joined to A (left lane of block 1).
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert!(s.join(Dir::Up));
    s.add_window(X);
    s.add_window(C);
    s.add_window(D);
    assert!(s.join(Dir::Up));

    assert!(s.join_into(C, A));
    assert_eq!(at(&s, A), rect(0, 0, 960, 540));
    assert_eq!(at(&s, C), rect(0, 540, 960, 540));
    assert_eq!(at(&s, B), rect(960, 0, 960, 1080));
}
