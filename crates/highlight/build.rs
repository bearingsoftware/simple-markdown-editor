//! Builds the syntax set the highlighter loads (MDEDIT-201): syntect's bundled sublimehq set plus the
//! syntaxes vendored under `syntaxes/`, dumped to `OUT_DIR`. Doing it here keeps the YAML loader out
//! of the binary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stdout)]

use std::env;
use std::path::PathBuf;

use syntect::dumps::dump_to_uncompressed_file;
use syntect::parsing::SyntaxSet;

const VENDORED: &[&str] = &["TOML", "Dockerfile", "Kotlin", "PowerShell"];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=syntaxes");

    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets this"));
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    for name in VENDORED {
        let dir = root.join("syntaxes").join(name);
        builder
            .add_from_folder(&dir, true)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    }

    let out =
        PathBuf::from(env::var_os("OUT_DIR").expect("cargo sets this")).join("syntaxes.packdump");
    dump_to_uncompressed_file(&builder.build(), &out).expect("write the syntax dump");
}
