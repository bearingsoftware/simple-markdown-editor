//! What kind of file an open tab holds, as an icon (MDEDIT-174, MDEDIT-178).
//!
//! Shared by the file pane and the tab strip, which are two presentations of the same open files and
//! must agree about what each one is. Geometry is not here: each presentation owns the column the
//! icon sits in, because a 24 px row and a 30 px tab are not the same shape.

/// What an open file's icon says it is (MDEDIT-174).
///
/// **Three, not seven.** The icon font has no JSON glyph and no YAML one; its usable silhouettes are
/// a page, a page with lines, code brackets and three rules, and its `Document`, `Page` and
/// `MusicInfo` code points rasterise to one identical glyph. So the languages are grouped by what
/// they are *for* rather than given a shape apiece they cannot have. JSON and YAML look alike here,
/// and their names are what tell them apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    /// Prose to read: Markdown.
    Note,
    /// Structure to configure: JSON, YAML, XML, TOML, INI.
    Structured,
    /// Plain text, and anything with no language of its own.
    Text,
}

impl Icon {
    /// Which icon a language gets.
    ///
    /// **Keyed off the language rather than the extension**, so a new one gets an icon by being added
    /// to [`crate::detect`] and nowhere else — `.yml` and `.yaml` cannot drift apart, because they are
    /// one language by the time this is asked.
    #[must_use]
    pub fn of(language: &dyn mdedit_language::Language) -> Self {
        match language.name() {
            "Markdown" => Self::Note,
            "JSON" | "YAML" | "XML" | "TOML" | "INI" => Self::Structured,
            _ => Self::Text,
        }
    }

    /// The private-use code point this draws, from the icon font MDEDIT-92 already uses.
    ///
    /// **Rasterised and looked at, not taken from a reference page** — that is how the three
    /// identical "document" code points were found.
    #[must_use]
    pub fn glyph(self) -> char {
        match self {
            Self::Note => '\u{E9F9}',
            Self::Structured => '\u{E943}',
            Self::Text => '\u{E14C}',
        }
    }

    /// What to draw instead on a machine whose fonts have no such glyph.
    ///
    /// Short enough for the icon's column, because that column is all the room there is: a fallback
    /// that overran it would push into the name it is supposed to leave alone.
    #[must_use]
    pub fn badge(self) -> &'static str {
        match self {
            Self::Note => "MD",
            Self::Structured => "{}",
            Self::Text => "TXT",
        }
    }
}
