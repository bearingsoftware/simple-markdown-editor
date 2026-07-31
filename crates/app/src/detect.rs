//! Which language a file is in.
//!
//! # By extension, and dully
//!
//! `AGENTS.md` names the 2023 Notepad++ CVEs, which were all heap overflows in hand-rolled encoding
//! detection — the class that comes from inspecting a file's *contents* to decide what it is. So
//! nothing here reads a byte of the file. It looks at the extension, lowercased, and that is the
//! whole algorithm.
//!
//! The cost is that a JSON file named `data.txt` opens as Markdown. That is a worse guess and a much
//! smaller problem than a parser reached by a crafted file, and the fix when someone wants it is a
//! language picker — a UI decision, not a sniffing one.
//!
//! An unknown extension, or none, is Markdown. This is a Markdown editor; that is the right default
//! and not an admission of defeat.

use std::path::Path;

use mdedit_language::Language;

/// The language to read `path` as.
#[must_use]
pub fn language_for(path: &Path) -> &'static dyn Language {
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return &mdedit_markers::MARKDOWN;
    };

    // Lowercased because Windows filesystems are case-insensitive and `DATA.JSON` is a JSON file.
    match extension.to_ascii_lowercase().as_str() {
        "json" => &mdedit_json::JSON,
        // Both spellings. `.yml` is the older one and still the commoner in the wild.
        "yaml" | "yml" => &mdedit_yaml::YAML,
        _ => &mdedit_markers::MARKDOWN,
    }
}
