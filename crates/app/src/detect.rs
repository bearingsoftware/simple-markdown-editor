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
//! # An unknown extension is plain text, and that reverses a decision
//!
//! This file used to say: *"An unknown extension, or none, is Markdown. This is a Markdown editor;
//! that is the right default and not an admission of defeat."* MDEDIT-93 reverses it, and the
//! reversal deserves to be as explicit as the original.
//!
//! **Guessing Markdown means styling a file that is not Markdown.** A `notes.txt` of shell commands
//! gets its `#` comments drawn as headings and its `*` globs as emphasis, and the concealment model
//! makes that worse than a wrong colour: `*.log` loses its asterisks. Defaulting to Markdown is a
//! confident answer to a question nobody asked.
//!
//! So the cue that a file *is* Markdown is the same cue that makes it Markdown when you save it —
//! its name. A new buffer is plain until then, and `.md` is spelled out here like every other
//! extension rather than being what is left over.

use std::path::Path;

use mdedit_language::Language;

/// The language to read `path` as.
#[must_use]
pub fn language_for(path: &Path) -> &'static dyn Language {
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return &mdedit_language::PLAIN;
    };

    // Lowercased because Windows filesystems are case-insensitive and `DATA.JSON` is a JSON file.
    match extension.to_ascii_lowercase().as_str() {
        // Both spellings, and `.markdown` because it is the one the format's own site uses.
        "md" | "markdown" => &mdedit_markers::MARKDOWN,
        "json" => &mdedit_json::JSON,
        // `.yml` is the older spelling and still the commoner in the wild.
        "yaml" | "yml" => &mdedit_yaml::YAML,
        // `.xml` alone, deliberately. `.svg`, `.xsd`, `.csproj` and the rest are XML too, and each
        // is a line here when somebody wants it rather than a guess made in advance.
        "xml" => &mdedit_xml::XML,
        "toml" => &mdedit_toml::TOML,
        // **`.cfg` is not a format, it is a file extension**, and mapping it here is a guess of the
        // same kind as the `data.txt` note above — Python's `setup.cfg` is INI and plenty of other
        // `.cfg` files are not. It is a better guess than plain text and it is still a guess, which
        // is why it is written down rather than left implied.
        "ini" | "cfg" => &mdedit_ini::INI,
        _ => &mdedit_language::PLAIN,
    }
}
