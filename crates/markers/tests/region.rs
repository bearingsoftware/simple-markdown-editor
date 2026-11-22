//! MDEDIT-207: the trait method takes only the region's text, and has to say what the free function
//! said about the same region of the whole source.

use mdedit_language::Language;
use mdedit_markers::{MARKDOWN, analyze, analyze_region};
use mdedit_offsets::{ByteOffset, ByteSpan};

#[test]
fn the_trait_on_a_slice_equals_the_free_function_on_the_whole_source() {
    let source =
        "# Title é\n\nSome *prose* here.\n\n- a\n- b\n\n```rust\nfn f() {}\n```\n\n> quote\n";
    let mut regions = vec![ByteSpan::new(
        ByteOffset::ZERO,
        ByteOffset::new(source.len()),
    )];
    regions.extend(
        analyze(source)
            .blocks
            .iter()
            .filter(|block| block.depth() == 0)
            .map(|block| ByteSpan::new(block.span().start(), ByteOffset::new(source.len()))),
    );
    assert!(
        regions.len() > 3,
        "the fixture should have several block starts"
    );
    for region in regions {
        assert_eq!(
            MARKDOWN.analyze_region(region.slice(source), region.start()),
            Some(analyze_region(source, region)),
            "region {region}"
        );
    }
}
