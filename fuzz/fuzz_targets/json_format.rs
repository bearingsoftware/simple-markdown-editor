//! Format Document, against the invariant it is the one sanctioned exception to.
//!
//! This is the only whole-document rewrite in the editor, and the decision permitting it was made
//! once, on MDEDIT-48, on the strength of exactly these two properties: the result is still valid
//! JSON, and every byte outside the whitespace is unchanged. `1.50` stays `1.50`, `1e3` stays `1e3`,
//! `-0` stays `-0`, and key order is key order.
//!
//! Differential, in the only sense available here: `validate` decides whether `format` is allowed to
//! run, and `format`'s output is put back through both `validate` and the same oracle the property
//! test uses. A document the validator passes and the formatter mangles is the worst outcome this
//! feature has, because it rewrites the user's file.
//!
//! The oracle lives in `mdedit_json` rather than here — see `meaningful_bytes`. An oracle kept in two
//! places is an oracle that drifts, and this one is already shared with `crates/json/tests/format.rs`.

#![no_main]

use libfuzzer_sys::fuzz_target;

// Raw bytes with a UTF-8 gate rather than an `Arbitrary` `&str`: a corpus file is then literally
// a document, so real ones can be dropped in — and invalid UTF-8 cannot reach these functions in
// the editor anyway, because the file layer has already decoded.
fuzz_target!(|data: &[u8]| {
    let Ok(source) = std::str::from_utf8(data) else {
        return;
    };

    let formatted = match mdedit_json::format(source) {
        Ok(formatted) => formatted,
        // Refusing is the documented answer for anything the validator objects to, and there is a
        // diagnostic to show for it. Nothing further to check.
        Err(_) => {
            assert!(
                !mdedit_json::validate(source).is_empty(),
                "refused to format a document it had no complaint about"
            );
            return;
        }
    };

    assert!(
        mdedit_json::validate(&formatted).is_empty(),
        "formatting produced JSON the validator rejects"
    );
    assert_eq!(
        mdedit_json::meaningful_bytes(&formatted),
        mdedit_json::meaningful_bytes(source),
        "formatting changed something other than whitespace"
    );

    // Pressing the menu item twice must not be two undo steps' worth of change.
    let again = mdedit_json::format(&formatted).expect("formatted output stays valid");
    assert_eq!(again, formatted, "formatting is not idempotent");
});
