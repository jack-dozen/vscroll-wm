//! Unit tests for the Space-based layout engine.

use crate::*;

const A: WindowId = WindowId(1);
const B: WindowId = WindowId(2);
const C: WindowId = WindowId(3);
const D: WindowId = WindowId(4);

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rect {
    Rect { x, y, w, h }
}

fn strip() -> Strip {
    Strip::new(rect(0, 0, 1920, 1080))
}

fn at(s: &Strip, id: WindowId) -> Rect {
    s.layout()
        .into_iter()
        .find(|p| p.id == id)
        .expect("window in layout")
        .rect
}

fn vis(s: &Strip, id: WindowId) -> bool {
    s.layout()
        .into_iter()
        .find(|p| p.id == id)
        .expect("window in layout")
        .visible
}

#[test]
fn new_windows_create_separate_full_spaces_and_scroll_cleanly() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert_eq!(s.spaces().len(), 2);
    assert_eq!(s.focused(), Some(B));
    assert_eq!(at(&s, B), rect(0, 0, 1920, 1080));
    assert!(vis(&s, B));
    assert!(!vis(&s, A));
    assert_eq!(at(&s, A), rect(0, -1080, 1920, 1080));

    // Focus up scrolls cleanly back to Space 0.
    s.focus(Dir::Up);
    assert_eq!(s.focused(), Some(A));
    assert_eq!(at(&s, A), rect(0, 0, 1920, 1080));
    assert!(vis(&s, A));
    assert!(!vis(&s, B));
    assert_eq!(at(&s, B), rect(0, 1080, 1920, 1080));
}

#[test]
fn gap_is_relative_to_space_so_adjacent_spaces_never_peek() {
    let mut s = strip();
    s.set_gap(16);
    s.add_window(A);
    s.add_window(B);
    s.focus(Dir::Up); // Viewing Space 0 (A)

    let ra = at(&s, A);
    let rb = at(&s, B);
    assert_eq!(ra, rect(16, 16, 1920 - 32, 1080 - 32));
    assert!(vis(&s, A));
    // Space 1 starts at y = 1080 (exact bottom edge of monitor): 0 pixels peeking.
    assert_eq!(rb.y, 1080);
    assert!(!vis(&s, B));

    // Scroll to Space 1 (B): Space 0 ends at y = 0 (exact top edge of monitor).
    s.focus(Dir::Down);
    let ra2 = at(&s, A);
    assert_eq!(ra2.y + ra2.h, 0);
    assert!(!vis(&s, A));
    assert_eq!(at(&s, B), rect(16, 16, 1920 - 32, 1080 - 32));
    assert!(vis(&s, B));
}

#[test]
fn snap_2d_state_machine_supports_empty_top_and_quarters() {
    let mut s = strip();
    s.add_window(A);

    // Full -> Win+Down -> half-height at bottom (top half empty!)
    s.snap(Dir::Down);
    assert_eq!(at(&s, A), rect(0, 540, 1920, 540));

    // Bottom half -> Win+Up -> Full again
    s.snap(Dir::Up);
    assert_eq!(at(&s, A), rect(0, 0, 1920, 1080));

    // Full -> Win+Down -> Win+Left -> bottom-left quarter
    s.snap(Dir::Down);
    s.snap(Dir::Left);
    assert_eq!(at(&s, A), rect(0, 540, 960, 540));

    // Bottom-left quarter -> Win+Up -> left half-width full-height
    s.snap(Dir::Up);
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));

    // Left full-height -> Win+Up -> top-left quarter
    s.snap(Dir::Up);
    assert_eq!(at(&s, A), rect(0, 0, 960, 540));

    // Top-left quarter -> Win+Right -> top half-height full-width
    s.snap(Dir::Right);
    assert_eq!(at(&s, A), rect(0, 0, 1920, 540));
}

#[test]
fn strict_merge_horizontal_and_vertical_into_space_above() {
    // 1. Horizontal merge: Space 0 has A(Left, Full), Space 1 has B(Full) -> Win+Right merges.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    assert_eq!(s.spaces().len(), 2);
    s.snap(Dir::Right);
    assert_eq!(s.spaces().len(), 1);
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(960, 0, 960, 1080));

    // 2. Vertical merge: Space 0 has A(Full, Top), Space 1 has B(Full) -> Win+Down merges.
    let mut s2 = strip();
    s2.add_window(A);
    s2.snap(Dir::Up);
    s2.add_window(B);
    assert_eq!(s2.spaces().len(), 2);
    s2.snap(Dir::Down);
    assert_eq!(s2.spaces().len(), 1);
    assert_eq!(at(&s2, A), rect(0, 0, 1920, 540));
    assert_eq!(at(&s2, B), rect(0, 540, 1920, 540));
}

#[test]
fn strict_merge_waits_until_size_fits_empty_slot() {
    // Space 0: A(Left, Full) + B(Right, Top). Only (Right, Bottom) 1/4 is free.
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right); // merges into Space 0 at (Right, Full)
    s.snap(Dir::Up); // shrinks to (Right, Top)
    assert_eq!(s.spaces().len(), 1);

    // Space 1: C(Full). Win+Right makes C (Right, Full) -> does NOT fit 1/4 slot -> stays in Space 1.
    s.add_window(C);
    assert_eq!(s.spaces().len(), 2);
    s.snap(Dir::Right);
    assert_eq!(s.spaces().len(), 2);
    assert_eq!(at(&s, C), rect(960, 0, 960, 1080));

    // Now Win+Down on C -> (Right, Bottom) -> matches empty 1/4 slot in Space 0 -> merges!
    s.snap(Dir::Down);
    assert_eq!(s.spaces().len(), 1);
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(960, 0, 960, 540));
    assert_eq!(at(&s, C), rect(960, 540, 960, 540));
}

#[test]
fn collision_shifts_to_opposite_side_when_free() {
    // Space 0: B(Right, Top) + C(Right, Bottom). Left side is empty!
    let mut s = strip();
    s.add_window(B);
    s.snap(Dir::Right);
    s.snap(Dir::Up);
    s.add_window(C);
    s.snap(Dir::Right);
    s.snap(Dir::Down); // merges into Space 0 at (Right, Bottom)
    assert_eq!(s.spaces().len(), 1);

    // Focus B and press Win+Down -> B becomes (Right, Full), C shifts to (Left, Bottom)!
    s.set_focus(B);
    s.snap(Dir::Down);
    assert_eq!(s.spaces().len(), 1);
    assert_eq!(at(&s, B), rect(960, 0, 960, 1080));
    assert_eq!(at(&s, C), rect(0, 540, 960, 540));
}

#[test]
fn collision_evicts_to_new_space_when_opposite_side_is_occupied() {
    // Space 0: A(Left, Full), B(Right, Top), C(Right, Bottom).
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right);
    s.snap(Dir::Up);
    s.add_window(C);
    s.snap(Dir::Right);
    s.snap(Dir::Down);
    assert_eq!(s.spaces().len(), 1);

    // Focus B and expand down -> (Right, Full). Left has A, so C is evicted to Space 1.
    s.set_focus(B);
    s.snap(Dir::Down);
    assert_eq!(s.spaces().len(), 2);
    assert_eq!(s.spaces()[0].items.len(), 2);
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(960, 0, 960, 1080));
    assert_eq!(s.spaces()[1].items.len(), 1);
    assert_eq!(s.spaces()[1].items[0].id, C);
}

#[test]
fn expanding_to_full_via_win_arrow_evicts_others_together_into_one_space() {
    // Space 0: A(Left, Full), B(Right, Top), C(Right, Bottom).
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right);
    s.snap(Dir::Up);
    s.add_window(C);
    s.snap(Dir::Right);
    s.snap(Dir::Down);

    // Focus A and press Win+Right -> A becomes Full, B & C move together to Space 1!
    s.set_focus(A);
    s.snap(Dir::Right);
    assert_eq!(s.spaces().len(), 2);
    assert_eq!(s.spaces()[0].items.len(), 1);
    assert_eq!(at(&s, A), rect(0, 0, 1920, 1080));
    assert_eq!(s.spaces()[1].items.len(), 2);
    assert_eq!(s.spaces()[1].items[0].id, B);
    assert_eq!(s.spaces()[1].items[1].id, C);
    assert_eq!(s.spaces()[1].items[0].top, 0);
    assert_eq!(s.spaces()[1].items[1].top, 500);
}

#[test]
fn maximize_explodes_all_windows_in_space_into_individual_full_spaces() {
    // Space 0: A(Left, Full), B(Right, Top), C(Right, Bottom). Space 1: D(Full).
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right);
    s.snap(Dir::Up);
    s.add_window(C);
    s.snap(Dir::Right);
    s.snap(Dir::Down);
    s.add_window(D);
    assert_eq!(s.spaces().len(), 2);

    // Win+Enter on B -> A, B, C each become their own Full Space; D stays below.
    s.set_focus(B);
    s.maximize();
    assert_eq!(s.spaces().len(), 4);
    assert_eq!(s.focused(), Some(B));
    for (idx, expected) in [A, B, C, D].into_iter().enumerate() {
        assert_eq!(s.spaces()[idx].items.len(), 1);
        assert_eq!(s.spaces()[idx].items[0].id, expected);
        assert!(s.spaces()[idx].items[0].is_full_space());
    }
    assert_eq!(at(&s, B), rect(0, 0, 1920, 1080));
}

#[test]
fn adjust_and_reset_split_affect_only_current_space() {
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right);
    s.add_window(C);
    s.snap(Dir::Left); // Space 1 (A+B in Space 0 are full, so C stays in Space 1)
    s.add_window(D);
    s.snap(Dir::Right); // merges into Space 1

    s.set_focus(A);
    assert!(s.adjust_split(192)); // +10% -> 600
    assert_eq!(s.spaces()[0].split, 600);
    assert_eq!(s.spaces()[1].split, 500);

    assert!(s.reset_split());
    assert_eq!(s.spaces()[0].split, 500);
}

#[test]
fn overview_reports_space_height_and_count() {
    let mut s = strip();
    s.set_gap(16);
    s.add_window(A);
    s.add_window(B);
    let ov = s.overview();
    assert_eq!(ov.space_count, 2);
    assert_eq!(ov.space_h, 1080 - 16);
    assert_eq!(ov.height, 2 * (1080 - 16));
    assert_eq!(ov.view_h, ov.space_h);
    assert_eq!(ov.view_y, ov.space_h); // B is focused (Space 1)
}

#[test]
fn move_window_swaps_slots_and_sets_cue_for_full_spaces() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    s.move_window(Dir::Up);
    assert_eq!(s.spaces()[0].items[0].id, B);
    assert_eq!(s.spaces()[1].items[0].id, A);
    assert_eq!(s.take_move_cue(), Some((B, Dir::Up)));
    assert_eq!(s.take_move_cue(), None);
}

#[test]
fn evicted_window_auto_merges_back_when_slot_is_freed() {
    // Space 0: A(Right, Full), B(Left, Top), C(Left, Bottom).
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Right);
    s.add_window(B);
    s.snap(Dir::Left);
    s.snap(Dir::Up); // merges into Space 0 at (Left, Top)
    s.add_window(C);
    s.snap(Dir::Left);
    s.snap(Dir::Down); // merges into Space 0 at (Left, Bottom)
    assert_eq!(s.spaces().len(), 1);

    // Focus B and press Win+Right -> B becomes (Full, Top), A is evicted to Space 1.
    s.set_focus(B);
    s.snap(Dir::Right);
    assert_eq!(s.spaces().len(), 2);
    assert_eq!(at(&s, B), rect(0, 0, 1920, 540));
    assert_eq!(s.spaces()[1].items[0].id, A);

    // Pull B back to (Left, Top) via Win+Left -> right half is free -> A auto-merges back!
    s.snap(Dir::Left);
    assert_eq!(s.spaces().len(), 1);
    assert_eq!(at(&s, A), rect(960, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(0, 0, 960, 540));
    assert_eq!(at(&s, C), rect(0, 540, 960, 540));
}

#[test]
fn full_eviction_group_auto_merges_back_when_shrunk_back() {
    // Space 0: A(Left, Full), B(Right, Top), C(Right, Bottom).
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right);
    s.snap(Dir::Up);
    s.add_window(C);
    s.snap(Dir::Right);
    s.snap(Dir::Down);
    assert_eq!(s.spaces().len(), 1);

    // Expand A to Full -> B & C evicted together to Space 1.
    s.set_focus(A);
    s.snap(Dir::Right);
    assert_eq!(s.spaces().len(), 2);

    // Shrink A back to Left -> B & C auto-merge back into Space 0!
    s.snap(Dir::Left);
    assert_eq!(s.spaces().len(), 1);
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(960, 0, 960, 540));
    assert_eq!(at(&s, C), rect(960, 540, 960, 540));
}

#[test]
fn authority_swap_flips_lanes_when_focused_on_larger_window_and_swaps_slot_otherwise() {
    // Space 0: A(Right, Full), B(Left, Top), C(Left, Bottom).
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Right);
    s.add_window(B);
    s.snap(Dir::Left);
    s.snap(Dir::Up);
    s.add_window(C);
    s.snap(Dir::Left);
    s.snap(Dir::Down);
    assert_eq!(s.spaces().len(), 1);

    // Focus A (higher authority) + Win+Alt+Left -> A moves to Left Full, B & C flip to Right!
    s.set_focus(A);
    s.move_window(Dir::Left);
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(960, 0, 960, 540));
    assert_eq!(at(&s, C), rect(960, 540, 960, 540));

    // Focus B (lower authority) + Win+Alt+Left -> B swaps slot with A (B becomes Left Full, A becomes Right Top).
    s.set_focus(B);
    s.move_window(Dir::Left);
    assert_eq!(at(&s, B), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, A), rect(960, 0, 960, 540));
    assert_eq!(at(&s, C), rect(960, 540, 960, 540));
}

#[test]
fn authority_swap_also_works_vertically_for_top_split() {
    // Space 0: A(Full, Top), B(Left, Bottom), C(Right, Bottom).
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.add_window(B);
    s.snap(Dir::Left);
    s.snap(Dir::Down);
    s.add_window(C);
    s.snap(Dir::Right);
    s.snap(Dir::Down);
    assert_eq!(s.spaces().len(), 1);

    // Focus A (higher authority) + Win+Alt+Down -> A moves to Bottom Full, B & C flip to Top!
    s.set_focus(A);
    s.move_window(Dir::Down);
    assert_eq!(at(&s, A), rect(0, 540, 1920, 540));
    assert_eq!(at(&s, B), rect(0, 0, 960, 540));
    assert_eq!(at(&s, C), rect(960, 0, 960, 540));
}

#[test]
fn directional_eviction_pushes_up_from_bottom_and_auto_merges_back() {
    // 1. Vertical push up: A(Full, Top), B(Left, Bottom), C(Right, Bottom).
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Up);
    s.add_window(B);
    s.snap(Dir::Left);
    s.snap(Dir::Down);
    s.add_window(C);
    s.snap(Dir::Right);
    s.snap(Dir::Down);
    assert_eq!(s.spaces().len(), 1);

    // Focus B and Win+Up -> B grows up to (Left, Full), A is pushed to Space ABOVE (Space 0)!
    s.set_focus(B);
    s.snap(Dir::Up);
    assert_eq!(s.spaces().len(), 2);
    assert_eq!(s.spaces()[0].items[0].id, A); // Space above
    assert_eq!(s.spaces()[1].items.len(), 2); // B + C stay in active Space (Space 1)
    assert_eq!(at(&s, B), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, A).y, -1080); // Above screen

    // Shrink B back down -> A auto-merges back from Space above!
    s.snap(Dir::Down);
    assert_eq!(s.spaces().len(), 1);
    assert_eq!(at(&s, A), rect(0, 0, 1920, 540));
    assert_eq!(at(&s, B), rect(0, 540, 960, 540));
    assert_eq!(at(&s, C), rect(960, 540, 960, 540));

    // 2. Horizontal push from top vs bottom: A(Left, Full), B(Right, Top), C(Right, Bottom).
    let mut s2 = strip();
    s2.add_window(A);
    s2.snap(Dir::Left);
    s2.add_window(B);
    s2.snap(Dir::Right);
    s2.snap(Dir::Up);
    s2.add_window(C);
    s2.snap(Dir::Right);
    s2.snap(Dir::Down);
    assert_eq!(s2.spaces().len(), 1);

    // B (top) extends left -> A pushed BELOW (Space 1).
    s2.set_focus(B);
    s2.snap(Dir::Left);
    assert_eq!(s2.spaces().len(), 2);
    assert_eq!(s2.spaces()[1].items[0].id, A);
    s2.snap(Dir::Right); // A merges back
    assert_eq!(s2.spaces().len(), 1);

    // C (bottom) extends left -> A pushed ABOVE (Space 0)!
    s2.set_focus(C);
    s2.snap(Dir::Left);
    assert_eq!(s2.spaces().len(), 2);
    assert_eq!(s2.spaces()[0].items[0].id, A); // Space above
    assert_eq!(at(&s2, A).y, -1080);
    s2.snap(Dir::Right); // A merges back from Space above!
    assert_eq!(s2.spaces().len(), 1);
    assert_eq!(at(&s2, A), rect(0, 0, 960, 1080));
}

#[test]
fn vertical_edge_resize_moves_shared_divider_and_resizes_into_empty_space() {
    // 1. Shared vertical divider: A(Left, Full), B(Right, Top), C(Right, Bottom).
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right);
    s.snap(Dir::Up);
    s.add_window(C);
    s.snap(Dir::Right);
    s.snap(Dir::Down);

    // Drag B's bottom edge down by 108 px (+10%): B becomes 60%, C becomes 40%.
    assert!(s.adjust_vertical_edge(B, false, 108));
    assert_eq!(at(&s, B), rect(960, 0, 960, 648));
    assert_eq!(at(&s, C), rect(960, 648, 960, 432));

    // Drag C's top edge up by 108 px (-10%): back to 50/50.
    assert!(s.adjust_vertical_edge(C, true, -108));
    assert_eq!(at(&s, B), rect(960, 0, 960, 540));
    assert_eq!(at(&s, C), rect(960, 540, 960, 540));

    // 2. Resize into empty space: single window at (Right, Bottom), drag top edge up.
    let mut s2 = strip();
    s2.add_window(A);
    s2.snap(Dir::Right);
    s2.snap(Dir::Down); // 540..1080
    assert!(s2.adjust_vertical_edge(A, true, -216)); // -20% -> top=300, h=700
    assert_eq!(at(&s2, A), rect(960, 324, 960, 756));
}

#[test]
fn slot_steal_replaces_slot_moves_old_opposite_direction_and_leaves_donor_slot_empty() {
    // Space 0: A(Left, Full), B(Right, Full)
    // Space 1: C(Left, Full), D(Right, Full)
    let mut s = strip();
    s.add_window(A);
    s.snap(Dir::Left);
    s.add_window(B);
    s.snap(Dir::Right);
    s.add_window(C);
    s.snap(Dir::Left);
    s.add_window(D);
    s.snap(Dir::Right);
    assert_eq!(s.spaces().len(), 2);

    // Focus B (Space 0, Right). Candidates should exclude A (same Space) and include C, D.
    s.set_focus(B);
    assert_eq!(s.selector_candidates(), vec![C, D]);

    // Steal D (from Space 1, BELOW) into B's slot -> B moves to new Full Space ABOVE!
    // Space 0: B(Full)
    // Space 1: A(Left), D(Right)  <-- active
    // Space 2: C(Left)            <-- D's old right slot is empty
    assert!(s.steal_into_slot(D));
    assert_eq!(s.spaces().len(), 3);
    assert_eq!(s.focused(), Some(D));
    assert_eq!(s.spaces()[0].items[0].id, B);
    assert!(s.spaces()[0].items[0].is_full_space());
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, D), rect(960, 0, 960, 1080));
    assert_eq!(s.spaces()[2].items.len(), 1);
    assert_eq!(s.spaces()[2].items[0].id, C);
    assert_eq!(s.spaces()[2].items[0].span, Span::Left);

    // Now steal B (from Space 0, ABOVE) into D's slot -> D moves to new Full Space BELOW!
    assert!(s.steal_into_slot(B));
    assert_eq!(s.focused(), Some(B));
    assert_eq!(at(&s, A), rect(0, 0, 960, 1080));
    assert_eq!(at(&s, B), rect(960, 0, 960, 1080));
    assert_eq!(s.spaces()[1].items[0].id, D);
    assert!(s.spaces()[1].items[0].is_full_space());
}

#[test]
fn slot_steal_includes_minimized_windows_and_works_on_full_space() {
    let mut s = strip();
    s.add_window(A);
    s.add_window(B);
    assert!(s.set_hidden(B, true)); // B is minimized in Space 1
    s.set_focus(A);
    assert_eq!(s.selector_candidates(), vec![B]);

    // Stealing hidden B into Full Space A unhides B and moves A to Space above.
    assert!(s.steal_into_slot(B));
    assert_eq!(s.focused(), Some(B));
    assert_eq!(at(&s, B), rect(0, 0, 1920, 1080));
    assert_eq!(s.spaces()[0].items[0].id, A);
}
