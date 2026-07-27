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

```bash
cargo test          # 42 tests
cargo clippy --all-targets --all-features -- -D warnings
cargo deny check
cargo bench -p mdedit-markers
```

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
