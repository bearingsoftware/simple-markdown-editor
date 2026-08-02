//! Searching, and the bar's arithmetic — everything about find that needs no window.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::cast_precision_loss, clippy::float_cmp)]

use mdedit_app::field::Press;
use mdedit_app::find::{self, Bar, Button};
use mdedit_offsets::{ByteOffset, ByteSpan};

fn spans(text: &str, needle: &str, case_sensitive: bool) -> Vec<(usize, usize)> {
    find::matches(text, needle, case_sensitive)
        .iter()
        .map(|span| (span.start().get(), span.end().get()))
        .collect()
}

fn span(from: usize, to: usize) -> ByteSpan {
    ByteSpan::from(from..to)
}

#[test]
fn an_empty_needle_matches_nothing() {
    assert!(spans("anything at all", "", false).is_empty());
}

#[test]
fn matches_are_found_in_order() {
    assert_eq!(spans("a b a b a", "a", true), [(0, 1), (4, 5), (8, 9)]);
}

#[test]
fn matches_do_not_overlap() {
    // "aaa" holds two overlapping "aa"; the second starts where the first ended, not one on.
    assert_eq!(spans("aaa", "aa", true), [(0, 2)]);
}

#[test]
fn case_matters_when_it_is_asked_to() {
    assert_eq!(spans("Cat cat CAT", "cat", true), [(4, 7)]);
}

#[test]
fn case_does_not_matter_by_default() {
    assert_eq!(
        spans("Cat cat CAT", "cat", false),
        [(0, 3), (4, 7), (8, 11)]
    );
}

#[test]
fn folding_finds_accented_text_in_either_case() {
    assert_eq!(spans("CAFÉ", "café", false), [(0, "CAFÉ".len())]);
}

#[test]
fn a_span_is_a_length_in_the_original_not_in_a_lowercased_copy() {
    // `É` is two bytes and `é` is two bytes, so this one is easy. `İ` is not: it is two bytes and
    // lowercases to three, and a search that reported lengths from a folded copy would return a
    // span that does not lie on a character boundary in the buffer.
    let text = "aİb";
    let found = find::matches(text, "İ", false);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].slice(text), "İ");
}

#[test]
fn a_match_never_ends_half_way_through_a_folded_character() {
    // `İ` folds to `i` plus a combining dot. Searching for the `i` alone must not claim the first
    // half of it, because the span would then split a character in the buffer.
    let found = find::matches("İ", "i", false);
    assert!(found.is_empty(), "matched {found:?}");
}

#[test]
fn markers_are_searchable_because_the_source_is_what_is_searched() {
    assert_eq!(spans("a **bold** word", "**", true), [(2, 4), (8, 10)]);
}

#[test]
fn a_multi_byte_needle_lands_on_boundaries() {
    let text = "汉字编辑器";
    let found = find::matches(text, "编辑", false);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].slice(text), "编辑");
}

#[test]
fn next_finds_the_first_match_at_or_after_the_offset() {
    let found = [span(0, 1), span(4, 5), span(8, 9)];
    assert_eq!(find::next(&found, ByteOffset::new(1)), Some(1));
    assert_eq!(find::next(&found, ByteOffset::new(4)), Some(1));
    assert_eq!(find::next(&found, ByteOffset::new(5)), Some(2));
}

#[test]
fn next_wraps_past_the_last_match() {
    let found = [span(0, 1), span(4, 5)];
    assert_eq!(find::next(&found, ByteOffset::new(9)), Some(0));
}

#[test]
fn previous_finds_the_last_match_ending_at_or_before_the_offset() {
    let found = [span(0, 1), span(4, 5), span(8, 9)];
    assert_eq!(find::previous(&found, ByteOffset::new(8)), Some(1));
    assert_eq!(find::previous(&found, ByteOffset::new(5)), Some(1));
    assert_eq!(find::previous(&found, ByteOffset::new(4)), Some(0));
}

#[test]
fn previous_wraps_before_the_first_match() {
    let found = [span(0, 1), span(4, 5)];
    assert_eq!(find::previous(&found, ByteOffset::ZERO), Some(1));
}

#[test]
fn navigating_an_empty_list_goes_nowhere() {
    assert_eq!(find::next(&[], ByteOffset::ZERO), None);
    assert_eq!(find::previous(&[], ByteOffset::ZERO), None);
}

#[test]
fn index_of_is_exact() {
    let found = [span(0, 3), span(10, 13)];
    assert_eq!(find::index_of(&found, span(10, 13)), Some(1));
    // Overlapping a match is not being on it: the user has typed since landing there.
    assert_eq!(find::index_of(&found, span(10, 12)), None);
    assert_eq!(find::index_of(&found, span(5, 8)), None);
}

#[test]
fn within_slices_out_the_matches_a_chunk_covers() {
    let found = [span(0, 2), span(10, 12), span(20, 22), span(30, 32)];
    let inside = find::within(&found, span(10, 25));
    assert_eq!(inside, [span(10, 12), span(20, 22)]);
}

#[test]
fn within_excludes_a_match_that_merely_abuts_the_chunk() {
    let found = [span(0, 5), span(10, 15)];
    assert!(find::within(&found, span(5, 10)).is_empty());
}

#[test]
fn the_bar_rescans_only_when_something_it_depends_on_moves() {
    let mut bar = Bar::default();
    bar.field.insert("a");
    assert!(bar.refresh("a b a", 0, 1), "the first look must scan");
    assert!(!bar.refresh("a b a", 0, 1), "nothing moved");
    assert!(bar.refresh("a b a b", 0, 2), "the document changed");
    assert!(!bar.refresh("a b a b", 0, 2));

    bar.case_sensitive = true;
    assert!(bar.refresh("a b a b", 0, 2), "the case flag changed");

    bar.field.insert("b");
    assert!(bar.refresh("a b a b", 0, 2), "the needle changed");
}

#[test]
fn switching_document_rescans_even_at_the_same_revision() {
    // The case the document is in the key for. Two tabs opened and not yet typed in are both at
    // revision zero, so without it the second would keep the first one's offsets and light bytes
    // that have nothing to do with the search.
    let mut bar = Bar::default();
    bar.field.insert("a");
    assert!(bar.refresh("a b a", 0, 0));
    assert_eq!(bar.matches().len(), 2);
    assert!(bar.refresh("a", 1, 0), "a different document must rescan");
    assert_eq!(bar.matches().len(), 1);
}

#[test]
fn the_count_says_nothing_until_something_is_typed() {
    let mut bar = Bar::default();
    bar.refresh("a b a", 0, 1);
    assert_eq!(bar.count_label(ByteSpan::empty_at(ByteOffset::ZERO)), None);
}

#[test]
fn the_count_says_which_match_you_are_on() {
    let mut bar = Bar::default();
    bar.field.insert("a");
    bar.refresh("a b a b a", 0, 1);
    assert_eq!(bar.count_label(span(4, 5)).as_deref(), Some("2 of 3"));
}

#[test]
fn the_count_says_how_many_when_you_are_on_none_of_them() {
    let mut bar = Bar::default();
    bar.field.insert("a");
    bar.refresh("a b a", 0, 1);
    assert_eq!(
        bar.count_label(ByteSpan::empty_at(ByteOffset::new(2)))
            .as_deref(),
        Some("2 matches")
    );
}

#[test]
fn the_count_says_so_when_there_are_none() {
    let mut bar = Bar::default();
    bar.field.insert("zzz");
    bar.refresh("a b a", 0, 1);
    assert_eq!(
        bar.count_label(ByteSpan::empty_at(ByteOffset::ZERO))
            .as_deref(),
        Some("No results")
    );
}

// ---- geometry ----

/// Symbol buttons measure about a symbol wide; `Replace` and `All` are words and measure wider. The
/// numbers do not have to be right, only proportionate — `layout` pads and floors them.
fn widths(mode: find::Mode) -> Vec<f32> {
    find::buttons(mode)
        .iter()
        .map(|button| match button.glyph().len() {
            1..=2 => 12.0,
            length => length as f32 * 6.0,
        })
        .collect()
}

fn laid_out(mode: find::Mode, width: f32, count: f32) -> find::Layout {
    find::layout(mode, width, count, &widths(mode))
}

#[test]
fn find_is_one_row_and_replace_is_two() {
    assert_eq!(find::height(find::Mode::Find), find::HEIGHT);
    assert_eq!(find::height(find::Mode::GoTo), find::HEIGHT);
    assert_eq!(find::height(find::Mode::Replace), find::HEIGHT * 2.0);
}

#[test]
fn the_buttons_are_packed_against_the_right_edge() {
    let layout = laid_out(find::Mode::Find, 800.0, 50.0);
    let last = layout.buttons.last().expect("four buttons");
    assert!(
        (last.x + last.width - 794.0).abs() < 0.01,
        "the close button ends at the padding, not at {}",
        last.x + last.width
    );
    assert_eq!(layout.buttons.len(), 4);
}

#[test]
fn the_field_takes_what_the_buttons_and_the_count_leave() {
    let layout = laid_out(find::Mode::Find, 800.0, 50.0);
    assert!(layout.field.0 > 0.0);
    assert!(layout.field.0 + layout.field.1 <= layout.count);
}

#[test]
fn a_narrow_window_keeps_the_field_and_drops_the_count() {
    let layout = laid_out(find::Mode::Find, 160.0, 50.0);
    // The field runs up to the buttons instead of stopping short for a count there is no room for.
    let first = layout.buttons.first().expect("four buttons");
    assert!(
        layout.field.1 > 40.0,
        "the field collapsed to {}",
        layout.field.1
    );
    assert!(layout.field.0 + layout.field.1 <= first.x);
}

#[test]
fn a_click_lands_on_the_button_under_it() {
    let layout = laid_out(find::Mode::Find, 800.0, 50.0);
    for rect in &layout.buttons {
        let middle = rect.x + rect.width / 2.0;
        assert_eq!(
            layout.hit_test(middle, find::HEIGHT / 2.0),
            Some(rect.button)
        );
    }
}

#[test]
fn a_click_above_or_below_the_bar_hits_nothing() {
    let layout = laid_out(find::Mode::Find, 800.0, 50.0);
    let middle = layout.buttons[0].x + 1.0;
    assert_eq!(layout.hit_test(middle, -1.0), None);
    assert_eq!(layout.hit_test(middle, find::HEIGHT), None);
}

#[test]
fn a_click_in_the_text_box_is_not_a_button() {
    let layout = laid_out(find::Mode::Find, 800.0, 50.0);
    let inside = layout.field.0 + 1.0;
    assert_eq!(
        layout.over_field(inside, find::HEIGHT / 2.0),
        Some(find::Focus::Primary)
    );
    assert_eq!(layout.hit_test(inside, find::HEIGHT / 2.0), None);
}

#[test]
fn every_button_draws_something() {
    for mode in [find::Mode::Find, find::Mode::Replace, find::Mode::GoTo] {
        for button in find::buttons(mode) {
            assert!(!button.glyph().is_empty());
        }
    }
    assert_eq!(Button::Case.glyph(), "Aa");
}

// ---- the second row ----

#[test]
fn replace_puts_its_two_buttons_on_the_second_row() {
    let layout = laid_out(find::Mode::Replace, 800.0, 50.0);
    let second: Vec<Button> = layout
        .buttons
        .iter()
        .filter(|rect| rect.y > 0.0)
        .map(|rect| rect.button)
        .collect();
    assert_eq!(second, [Button::Replace, Button::ReplaceAll]);
}

/// The two word buttons are measured rather than given the symbols' width, or "Replace" would be a
/// button with three letters showing.
#[test]
fn a_word_button_is_wider_than_a_symbol_one() {
    let layout = laid_out(find::Mode::Replace, 800.0, 50.0);
    let width_of = |button| {
        layout
            .buttons
            .iter()
            .find(|rect| rect.button == button)
            .map(|rect| rect.width)
            .expect("on the bar")
    };
    assert!(width_of(Button::Replace) > width_of(Button::Close));
}

#[test]
fn the_second_row_gets_its_own_field() {
    let layout = laid_out(find::Mode::Replace, 800.0, 50.0);
    let replace = layout.replace.expect("a replacement box");
    assert!(replace.1 > 0.0);
    assert_eq!(
        layout.over_field(replace.0 + 1.0, find::HEIGHT + 4.0),
        Some(find::Focus::Secondary)
    );
    // And the row above it is still the search term, not the replacement.
    assert_eq!(
        layout.over_field(replace.0 + 1.0, 4.0),
        Some(find::Focus::Primary)
    );
}

#[test]
fn only_replace_has_a_second_field() {
    assert!(laid_out(find::Mode::Find, 800.0, 50.0).replace.is_none());
    assert!(laid_out(find::Mode::GoTo, 800.0, 50.0).replace.is_none());
}

#[test]
fn a_second_row_button_is_not_hit_from_the_first() {
    let layout = laid_out(find::Mode::Replace, 800.0, 50.0);
    let rect = layout
        .buttons
        .iter()
        .find(|rect| rect.button == Button::ReplaceAll)
        .expect("on the bar");
    let middle = rect.x + rect.width / 2.0;
    assert_eq!(
        layout.hit_test(middle, find::HEIGHT + 4.0),
        Some(Button::ReplaceAll)
    );
    assert_ne!(layout.hit_test(middle, 4.0), Some(Button::ReplaceAll));
}

#[test]
fn go_to_line_keeps_only_the_close_button() {
    let layout = laid_out(find::Mode::GoTo, 800.0, 50.0);
    let shown: Vec<Button> = layout.buttons.iter().map(|rect| rect.button).collect();
    assert_eq!(shown, [Button::Close]);
}

// ---- the bar's three fields ----

#[test]
fn keys_go_to_the_focused_field() {
    let mut bar = Bar::default();
    bar.mode = find::Mode::Replace;
    bar.press(&Press::Text("old".to_owned()), false);
    bar.next_field();
    bar.press(&Press::Text("new".to_owned()), false);

    assert_eq!(bar.field.text(), "old");
    assert_eq!(bar.replace.text(), "new");
}

#[test]
fn tab_does_nothing_outside_replace() {
    let mut bar = Bar::default();
    assert!(!bar.next_field(), "find has one field");
    bar.mode = find::Mode::GoTo;
    assert!(!bar.next_field());
    bar.mode = find::Mode::Replace;
    assert!(bar.next_field());
    assert_eq!(bar.focus, find::Focus::Secondary);
    assert!(bar.next_field());
    assert_eq!(bar.focus, find::Focus::Primary, "and back again");
}

#[test]
fn go_to_line_takes_digits_only() {
    let mut bar = Bar::default();
    bar.mode = find::Mode::GoTo;
    bar.press(&Press::Text("4".to_owned()), false);
    bar.press(&Press::Text("x".to_owned()), false);
    bar.press(&Press::Text("2".to_owned()), false);
    assert_eq!(bar.line.text(), "42");
    assert_eq!(bar.line_number(), Some(42));
}

/// A paste is filtered the same way a keystroke is, which is why the rule lives in `press` rather
/// than in the key handler.
#[test]
fn a_paste_into_go_to_line_keeps_only_its_digits() {
    let mut bar = Bar::default();
    bar.mode = find::Mode::GoTo;
    bar.press(&Press::Text("line 17 of the file".to_owned()), false);
    assert_eq!(bar.line.text(), "17");
}

#[test]
fn backspace_still_works_in_go_to_line() {
    let mut bar = Bar::default();
    bar.mode = find::Mode::GoTo;
    bar.press(&Press::Text("123".to_owned()), false);
    bar.press(&Press::Backspace, false);
    assert_eq!(bar.line.text(), "12");
}

#[test]
fn a_line_number_of_nothing_or_zero_is_no_line() {
    let mut bar = Bar::default();
    bar.mode = find::Mode::GoTo;
    assert_eq!(bar.line_number(), None);
    bar.press(&Press::Text("0".to_owned()), false);
    assert_eq!(bar.line_number(), None, "lines count from one");
}

/// Go to Line is not searching, so it must not keep lighting the matches of a search the user has
/// moved on from.
#[test]
fn go_to_line_has_no_matches() {
    let mut bar = Bar::default();
    bar.field.insert("a");
    bar.refresh("a b a", 0, 1);
    assert_eq!(bar.matches().len(), 2);

    bar.mode = find::Mode::GoTo;
    assert!(bar.refresh("a b a", 0, 1), "the mode changed");
    assert!(bar.matches().is_empty());
    assert_eq!(bar.count_label(ByteSpan::empty_at(ByteOffset::ZERO)), None);
}

#[test]
fn the_search_term_survives_a_trip_through_go_to_line() {
    let mut bar = Bar::default();
    bar.field.insert("alpha");
    bar.mode = find::Mode::GoTo;
    bar.press(&Press::Text("12".to_owned()), false);
    bar.mode = find::Mode::Find;
    assert_eq!(bar.field.text(), "alpha");
}
