//! MDEDIT-5 — does parley/fontique resolve mixed-script text to real glyphs on this platform?
//!
//! [linebender/parley#644] reports that font fallback fails for at least `汉` on Windows. The
//! comments identify the cause as architectural: fontique assumes one fallback font covers a
//! `(script, locale)` pair, which does not hold for `Hani` on Windows where two or more are needed.
//! The issue is open with no maintainer response.
//!
//! That decides whether the editor's text layer is DirectWrite-first or parley-first, so it needs
//! confirming on Windows rather than inferring. This probe lays out one line per script and reports
//! the resolved font plus any `.notdef` glyphs — glyph id 0, which renders as tofu and is exactly
//! what broken fallback looks like.
//!
//! Run it on the target platform:
//!
//! ```text
//! cargo run --release
//! ```
//!
//! Exit status is 1 if any sample produced tofu, so it can gate CI later.
//!
//! [linebender/parley#644]: https://github.com/linebender/parley/issues/644

use std::collections::BTreeSet;
use std::process::ExitCode;

use parley::{Alignment, AlignmentOptions, FontContext, LayoutContext, PositionedLayoutItem};
use skrifa::MetadataProvider;

/// One script's worth of text, and what it is there to prove.
struct Sample {
    script: &'static str,
    text: &'static str,
    note: &'static str,
}

const SAMPLES: &[Sample] = &[
    Sample {
        script: "Latin",
        text: "The quick brown fox",
        note: "control — if this fails, the harness is wrong, not fontique",
    },
    Sample {
        script: "Han (Simplified)",
        text: "汉字编辑器",
        note: "the character named in parley#644",
    },
    Sample {
        script: "Japanese",
        text: "日本語のテキスト",
        note: "kanji + kana; a second family is often needed for the same run",
    },
    Sample {
        script: "Korean",
        text: "한국어 텍스트",
        note: "Hangul, usually a distinct family again",
    },
    Sample {
        script: "Arabic",
        text: "النص العربي",
        note: "right-to-left, and shaping must join the letters",
    },
    Sample {
        script: "Hebrew",
        text: "טקסט עברי",
        note: "right-to-left without joining",
    },
    Sample {
        script: "Emoji",
        text: "🎉 🇬🇧 👩‍💻",
        note: "colour, flag (regional indicator pair), and a ZWJ sequence",
    },
    Sample {
        script: "Mixed in one run",
        text: "Editor 编辑器 محرر 🎉",
        note: "the real case — one paragraph, four scripts, fallback must switch mid-run",
    },
];

fn main() -> ExitCode {
    println!("MDEDIT-5 font fallback probe");
    println!(
        "platform: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!(
        "parley 0.11 · fontique via parley::FontContext\n{}",
        "-".repeat(78)
    );

    let mut font_cx = FontContext::new();
    let mut layout_cx: LayoutContext<[u8; 4]> = LayoutContext::new();
    let mut failures = 0usize;

    for sample in SAMPLES {
        let report = probe(&mut font_cx, &mut layout_cx, sample.text);

        let verdict = if report.tofu.is_empty() {
            "OK  "
        } else {
            failures += 1;
            "TOFU"
        };
        println!("{verdict} {:<18} {}", sample.script, sample.text);
        println!("       fonts: {}", join(&report.fonts));
        if !report.tofu.is_empty() {
            println!("       .notdef for: {}", join(&report.tofu));
        }
        println!("       ({})", sample.note);
        println!();
    }

    println!("{}", "-".repeat(78));
    if failures == 0 {
        println!("PASS — every sample resolved to real glyphs on this platform.");
        ExitCode::SUCCESS
    } else {
        println!(
            "FAIL — {failures} of {} samples produced .notdef glyphs.",
            SAMPLES.len()
        );
        println!(
            "This is the parley#644 failure mode. See the spike write-up for what it implies."
        );
        ExitCode::FAILURE
    }
}

struct Report {
    /// Font families the layout actually selected, in no particular order.
    fonts: BTreeSet<String>,
    /// Characters that came back as glyph 0.
    tofu: BTreeSet<String>,
}

/// Lays the text out and inspects the glyphs parley chose.
///
/// Glyph id 0 is `.notdef` by definition in every OpenType font, so its presence means no font in
/// the fallback chain covered that character — the thing #644 describes.
fn probe(font_cx: &mut FontContext, layout_cx: &mut LayoutContext<[u8; 4]>, text: &str) -> Report {
    let builder = layout_cx.ranged_builder(font_cx, text, 1.0, true);
    let mut layout = builder.build(text);
    layout.break_all_lines(None);
    layout.align(Alignment::Start, AlignmentOptions::default());

    let mut fonts = BTreeSet::new();
    let mut tofu = BTreeSet::new();

    for line in layout.lines() {
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let run = glyph_run.run();
            let font = run.font();
            let name = skrifa::FontRef::from_index(font.data.as_ref(), font.index)
                .ok()
                .and_then(|f| family_name(&f))
                .unwrap_or_else(|| "<unnamed>".to_owned());
            fonts.insert(name);

            for glyph in glyph_run.glyphs() {
                if glyph.id == 0 {
                    // Map back to the source text so the report names the character, not an index.
                    let cluster = run
                        .clusters()
                        .find(|c| c.glyphs().any(|g| g.id == 0))
                        .map(|c| text[c.text_range()].to_owned())
                        .unwrap_or_else(|| "<unknown>".to_owned());
                    tofu.insert(cluster);
                }
            }
        }
    }

    Report { fonts, tofu }
}

fn family_name(font: &skrifa::FontRef<'_>) -> Option<String> {
    font.localized_strings(skrifa::string::StringId::FAMILY_NAME)
        .english_or_first()
        .map(|s| s.chars().collect())
}

fn join(set: &BTreeSet<String>) -> String {
    if set.is_empty() {
        "<none>".to_owned()
    } else {
        set.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}
