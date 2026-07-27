# Spike: text layer — mixed-script fallback and Windows IME

MDEDIT-5. Decides whether the editor's text layer is **DirectWrite-first** or **parley-first**.

Deliberately outside the Cargo workspace (`exclude = ["spikes/*"]`) so it never enters CI. It is a
probe, not product code.

## Why

[`docs/research/02-gui-toolkits.md`](../../docs/research/02-gui-toolkits.md) recommends putting the
text layer behind a narrow trait and implementing it twice — DirectWrite now, parley later — partly
to de-risk [linebender/parley#644], *"Font Fallback not working for at least `汉` on Windows"*.

That issue is **open**, last touched 2026-07-14, with no maintainer response. Its comments identify
the cause as architectural rather than incidental:

> Am I understanding correctly that there is a fundamental issue with Fontique that it assumes that a
> given (script, locale) pair can be covered by a single fallback font, but that this isn't true for
> the `Hani` script on Windows where you actually need two fallback fonts?
> — nicoburns, confirmed by the reporter

A wrong assumption in the fallback model is not a one-line fix, and CJK is not an edge case for a
text editor. This probe confirms or refutes it on real hardware.

## Result so far

**Windows, 2026-07-27: parley#644 reproduces. Confirmed, not inferred.** 2 of 8 samples tofu'd:

```
TOFU Han (Simplified)   汉字编辑器
       fonts: Arial, Yu Gothic UI
       .notdef for: 汉, 编, 辑

Coverage audit — 232 font families installed:
  "汉" (U+6C49)
      covered by 10 installed families, e.g. SimSun, 新宋体, Microsoft JhengHei UI
      => the glyph was available; fallback did not reach it.
```

**SimSun — the default Simplified Chinese font on Windows — was installed and never consulted.** That
is what makes this conclusive rather than a missing-font artefact.

**Yu Gothic UI is a Japanese font.** fontique resolves `script = Hani` to one fallback family and
stops. `汉`, `编`, `辑` exist only in Simplified Chinese (`漢`, `編`, `輯` traditional) so they fall
through; `字` and `器` render because those forms are shared with Japanese.

Japanese, Korean, Arabic, Hebrew and emoji all pass — the proof rather than a mitigation. Fallback
works; it breaks only where one Unicode script spans locales with disjoint glyph coverage. Hani is
that case.

**Verdict: DirectWrite-first, high confidence.** `IDWriteFontFallback` does multi-font fallback
natively because it is what Windows itself uses. Re-run this probe when #644 closes; a green run is
the condition for reconsidering parley on Windows.

Full analysis on the Confluence page linked from MDEDIT-5.

## Testing the proposed fix (parley#689)

[linebender/parley#689] *"Fontique: 3-tier font fallback"* claims to fix #644. Its Windows tier
delegates to `IDWriteFontFallback::MapCharacters` — the same API the DirectWrite-first plan calls
directly — so the fix and our workaround converge, which is reassuring for both.

**Status as of 2026-07-27:** open, not merged, `mergeable_state: dirty` (conflicts with main), one
AI-generated commit, no approving review. The only maintainer response is supportive-with-caveats:
dfrg finds the char lookup and cache "iffy" and wants to avoid an exhaustive search, so the design is
not settled.

To test it, uncomment the `[patch.crates-io]` block in `Cargo.toml` **and raise `rust-version` to
1.88**, then:

```powershell
cargo update             # a stale Cargo.lock silently keeps the crates.io resolution
cargo tree -i fontique   # MUST show a github.com/nicoburns/parley source
cargo run --release
```

The MSRV bump is not optional. The PR branch requires Rust 1.88, and cargo's MSRV-aware resolver will
otherwise hold transitive dependencies at 1.85-compatible versions, producing:

```
error[E0015]: cannot call non-const associated function
              `PropertyNamesShort::<Script>::new` in statics
   --> parley_core/src/itemize.rs:216
```

That reads like a defect in the PR and is not one — it is an artefact of this crate's own MSRV pin.

**Confirm the second command before trusting the third.** A patched and an unpatched run look
identical in the probe's own output, so without checking provenance a "still fails" result is
unreadable — it cannot be told apart from the patch simply not having applied.

A clean PASS on Windows would be a useful datapoint for that PR; nobody has posted one.

## Running it

**On Windows** — this is the run that matters:

```powershell
cd spikes\font-fallback
cargo run --release
```

Exit code is 0 if every sample resolved to real glyphs, 1 if any produced tofu.

**On Linux** it currently does not build: fontique's fontconfig backend needs the development
headers, and the crate's `RUST_FONTCONFIG_DLOPEN` mode is incompatible with how fontique 0.11 imports
the symbols.

```bash
sudo apt install libfontconfig1-dev   # then cargo run --release
```

That is itself worth knowing — **parley adds a build-time system dependency on Linux**, which the
DirectWrite path would not have on Windows.

## What it measures

Lays out one line per script with parley and inspects the glyphs it chose. **Glyph id 0 is `.notdef`
in every OpenType font by definition**, so its presence means nothing in the fallback chain covered
that character — precisely the #644 failure. For each sample it prints the font families actually
selected and any characters that came back as tofu.

Samples: Latin (control), Han, Japanese, Korean, Arabic, Hebrew, emoji (colour, flag pair, ZWJ
sequence), and one mixed-script line — the real case, where fallback has to switch mid-run.

A missing font on the test machine and broken fallback look identical in the output. Check the
machine has CJK and Arabic fonts installed before reading a failure as a fontique defect; on a
default Windows install it will.

## The IME half — needs a human

**Not automatable, and not runnable from WSL.** IMM32 composition requires real keyboard input
through a real input method; there is no headless harness for it. Run this by hand on Windows with a
Japanese IME installed (Settings → Time & Language → Language → Japanese → Microsoft IME).

Against a text widget built on the candidate stack:

| # | Step | Pass condition |
|---|---|---|
| 1 | Switch to Japanese IME, type `nihongo` | Pre-edit text appears underlined *in the widget*, not in a floating box at the screen corner |
| 2 | Press space to open the candidate list | Candidate window appears **at the caret**, not at the window origin |
| 3 | Choose a candidate, press Enter | Committed text replaces the pre-edit in one edit, not two |
| 4 | Type `nihongo`, then click elsewhere mid-composition | Composition commits or cancels cleanly — **no dropped character** (this is the exact bug in slint#10861) |
| 5 | Type a long composition, press Escape | Pre-edit is removed, buffer unchanged |
| 6 | With composition active, press Ctrl+Z | Undo does not split the composition into fragments |
| 7 | Win+H voice typing, dictate a sentence | Text arrives. **Expected to fail on IMM32** — voice typing delivers via `ITextStoreACP::InsertTextAtSelection` and the IMM32↔TSF bridge does not bridge it ([wezterm#7791]) |

Step 4 is the one that most often fails and the one users notice, because it silently loses input.
Step 7 failing is expected and acceptable for v1 — it is why
[`AGENTS.md`](../../AGENTS.md) requires the buffer sit behind a trait that can later back an
`ITextStoreACP`.

[linebender/parley#644]: https://github.com/linebender/parley/issues/644
[wezterm#7791]: https://github.com/wezterm/wezterm/issues/7791
[linebender/parley#689]: https://github.com/linebender/parley/pull/689
