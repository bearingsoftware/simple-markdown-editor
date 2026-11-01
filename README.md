# simple-markdown-editor

A native WYSIWYG Markdown editor in Rust. Windows first, portable later.

Not an Electron app, not a webview. The buffer holds literal Markdown and syntax is *concealed*
rather than converted, so opening a file and saving it returns the same bytes.

## State

No renderer yet. What exists is the document core, built and tested headlessly:

| Crate | What it does |
|---|---|
| [`mdedit-offsets`](crates/offsets) | Distinct types for the coordinate spaces the editor carries at once |
| [`mdedit-markers`](crates/markers) | Partitions Markdown source into syntax-marker and content spans |
| [`mdedit-document`](crates/document) | Rope buffer that keeps that partition current across edits, plus selection and the reveal-on-cursor predicate |
| [`mdedit-text`](crates/text) | The text-layout trait — what the editor asks for, independent of who shapes it |
| [`mdedit-text-dwrite`](crates/text-dwrite) | DirectWrite implementation of that trait. Windows only |
| [`mdedit-file`](crates/file) | Reads and writes files without changing a byte that was not edited |
| [`mdedit-session`](crates/session) | Tabs as lightweight handles, and the session file that restores them |
| [`mdedit-app`](crates/app) | The window: winit + wgpu, the decoration step, and the latency harness |

```bash
cargo run -p mdedit-app --release -- FILE.md   # the editor, Windows only
cargo test          # 148 tests (10 of them Windows-only)
cargo clippy --all-targets --all-features -- -D warnings
cargo deny check
cargo bench -p mdedit-markers
```

## Keystroke latency

Measured on this machine over 400 keystrokes into the sample document, reported as percentiles
rather than a mean, and taken under power-save as well as plugged in — `docs/research/06` is
explicit that a mean on AC is the least informative number available.

| | p50 | p90 | p99 | max |
|---|---|---|---|---|
| Balanced, AC | 1.91 ms | 2.36 ms | 2.73 ms | 6.17 ms |
| Power saver | 2.28 ms | 2.77 ms | 3.55 ms | 9.20 ms |

Budget is p50 <= 3 ms and p99 <= 8.3 ms. This measures keystroke to `present()` returning, which
excludes the input stack before winit sees the key and the compositor after — so it is a lower
bound on true keystroke-to-photon, not the whole path.

## The invariant everything rests on

Concatenating the segments reproduces the source **byte for byte** — proven over all 652 CommonMark
0.31.2 spec examples, and again after arbitrary sequences of insert, delete and replace.

That is what makes decorate-the-source safe: a renderer can conceal every marker span and still be
holding the user's exact bytes. If it ever fails, the architecture is wrong, and the tests are
positioned to say so before a renderer exists to throw away.

## Working on this

Read in order:

1. [`AGENTS.md`](AGENTS.md) — the project guide. Baseline standards, project rules, what blocks review.
2. [`docs/research/`](docs/research) — seven briefs behind the major decisions. Start with the
   [README](docs/research/README.md); the decisions are evidenced, so don't relitigate them without
   new evidence.

Work is tracked in Jira `MDEDIT`, with plans and findings in the Confluence space of the same key.
That tracker is private, so `MDEDIT-*` references in commits and docs will not resolve for outside readers.

## Moving to Windows

Everything above is platform-agnostic and runs anywhere. Two things are Windows-only:

**The font-fallback probe** ([`spikes/font-fallback`](spikes/font-fallback)) — confirmed that
`fontique` resolves `script = Hani` to a single fallback family and stops, so Simplified-Chinese-only
characters render as tofu even with SimSun installed
([parley#644](https://github.com/linebender/parley/issues/644)).
[parley#689](https://github.com/linebender/parley/pull/689) fixes it and has been verified against
this probe, but is unmerged and unreleased. Until it ships, **the text layer is DirectWrite-first** —
see `AGENTS.md`.

**IME verification** (MDEDIT-6) — blocked on a renderer existing to type into. A seven-step manual
script is in the spike's [README](spikes/font-fallback/README.md).

Neither can be exercised from WSL: `fontique` uses DirectWrite on Windows and fontconfig on Linux, so
a WSL run tests the code path that is not broken.

## Licence

MIT.
