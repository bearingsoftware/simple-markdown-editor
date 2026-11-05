//! Laziness (MDEDIT-190). **One test, in a binary of its own**, so nothing else in the process can
//! have loaded the syntax set first: `cargo test` shares a process between a file's tests, and a
//! check that the set is not loaded is only real if it runs before anything loads it.

use mdedit_highlight::{highlight, is_loaded};

#[test]
fn the_syntax_set_loads_on_the_first_named_language_and_not_before() {
    assert!(!is_loaded());
    assert!(highlight("", "{\"a\": 1}\n").is_none());
    assert!(highlight("  \t", "{\"a\": 1}\n").is_none());
    assert!(!is_loaded(), "an empty info string must not load anything");

    assert!(highlight("rust", "fn main() {}\n").is_some());
    assert!(is_loaded());
}
