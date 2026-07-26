# 04 — Parsing benchmarks

**No public benchmark of Markdown incremental reparse latency existed**, so one
was built and run against the current crates: `tree-sitter` 0.26.11,
`tree-sitter-md` 0.5.3, `pulldown-cmark` 0.13.4.

> **Addendum, MDEDIT-3/4 (2026-07-26).** These figures measure the **parse only**. The editor also
> has to segment the result into marker and content spans. Measured with criterion,
> `crates/markers/benches/segment.rs`, reporting **overhead = segment − parse measured in the same
> run** (see the caveat below for why not absolute times):
>
> | Document | segmentation overhead, initial | after MDEDIT-4 |
> |---|---:|---:|
> | 6 KiB | 0.24 ms | **0.11 ms** |
> | 60 KiB | 2.69 ms | **1.03 ms** |
> | 310 KiB | 13.40 ms | **2.37 ms** |
>
> MDEDIT-4 fused three per-byte passes into one, replaced an `O(n)` bitmap with a merge walk, and
> reserved the collection capacities — the last of which removed superlinear scaling with document
> size. Criterion's paired comparison reported a significant improvement on total 60 KiB time in two
> independent runs (−38.7% and −25.4%, both p = 0.00).
>
> **Measurement caveat.** Absolute times on the development machine vary by more than 2× between
> runs — one run measured `segment` *faster* than `parse-only`, which is impossible. Only
> same-run overhead figures and criterion's paired verdicts are trustworthy here. Re-measure on a
> quiet machine before treating any absolute number as a budget.
>
> **What this does not change:** full re-parse per keystroke stays the v1 decision. What it changes
> is the *scope* — see [06](06-performance-targets.md#the-3-ms-budget-is-viewport-scoped). Parse plus
> segment of a whole 60 KB document cannot fit a 3 ms budget, because the parse alone does not. The
> keystroke path must segment the viewport, not the document.

## Conclusion first

**Skip incremental parsing entirely for v1. Full re-parse with `pulldown-cmark`
on every edit.**

| Document | pulldown-cmark **full parse** | tree-sitter-md **best-case incremental** | tree-sitter-md **as its README documents** |
|---|---:|---:|---:|
| 60 KB flat list | **2.99 ms** | 156.7 ms | 1075.0 ms |
| 49 KB prose | **3.56 ms** | 112.4 ms | 424.4 ms |
| 16 KB nested lists | **2.59 ms** | 3.6 ms | 133.4 ms |
| 310 KB flat list | **15.95 ms** | — | 2317.7 ms |

Re-parsing the entire document from scratch with `pulldown-cmark` is **~52×
faster than `tree-sitter-md`'s best-case incremental result**, and **~360× faster
than the approach its own README documents.**

At 60 KB you are at 3 ms — comfortably inside a 16 ms frame budget, on the main
thread, with no dirty-region logic whatsoever.

## Why tree-sitter incremental fails on Markdown

### The mechanism

Max Brunsfeld, tree-sitter's author, in
[tree-sitter-html#23](https://github.com/tree-sitter/tree-sitter-html/issues/23):

> "We **discard** a subtree if its preceding external scanner state is *different*
> from the preceding external scanner state on our parse stack."

Marijn Haverbeke filed that issue in Feb 2021 with this benchmark:

```
Parse from scratch   9.36/s
Opened new tag       9.31/s   <-- incremental parse gives ZERO speedup
Inserted new tag    49.65/s   <-- 5.3x speedup
```

> "Due to the way changes in the external scanner state prevent reuse of nodes
> created with another state, if you open a new tag … somewhere in a big document,
> **everything coming after that tag will be re-parsed**."

Brunsfeld confirmed the mechanism and proposed an `external_scanner_compare` hook.
**That issue is still open in 2026** and remains a *stretch goal* on the
[1.0 checklist](https://github.com/tree-sitter/tree-sitter/issues/930), which
explicitly cites "the **catastrophic node-reuse failures** seen in the HTML
parser."

**tree-sitter-markdown is strictly worse than HTML here**, because its serialized
scanner state includes `column` and `indentation` — values that change on nearly
every edit — on top of the open-block stack.

### Measured: incremental is often *worse than cold*

60 KB document (800-item flat list with bold/code/link per item), fresh `Parser`
per call, release build:

| Edit | Block parse | Speedup vs cold |
|---|---:|---:|
| cold | 199.4 ms | 1.00× |
| 1 char in a paragraph | 156.0 ms | 1.28× |
| open `> ` blockquote mid-doc | 248.3 ms | **0.80×** |
| open ` ``` ` fence mid-doc | 151.3 ms | 1.32× |
| open `> ` at doc **start** | 332.2 ms | **0.60×** |

**Typing `>`, `-`, `#` or a fence at a line start — i.e. what a Markdown editor
does constantly — makes incremental parsing actively harmful.** You pay
`Tree::edit` bookkeeping for negative return.

`changed_ranges` proves the invalidation:

| Edit | Changed ranges | Bytes | % of document |
|---|---:|---:|---:|
| 1 char mid-document | 5,585 | 9,575 | **15.4%** |
| open blockquote at start | 11,887 | — | **42.5%** |
| open blockquote at start (prose corpus) | — | — | **100.0%** |

That is the external-scanner-state discard rule, visible in output.

### The two-parser split makes it worse

`tree-sitter-md` splits into a block grammar and an inline grammar, combined via
`ts_parser_set_included_ranges`. Two strategies exist for the inline pass:

- **Strategy A** — one inline parse over all included ranges. *What the README
  prescribes.*
- **Strategy B** — one small tree per inline range. *The Helix/tree-house "layer"
  model.*

| Edit (60 KB, 800 inline ranges) | Inline A | Speedup | Inline B | Speedup |
|---|---:|---:|---:|---:|
| cold | 1059.9 ms | — | 624.1 ms | — |
| 1 char in a paragraph | 919.0 ms | **1.15×** | 0.72 ms | 871× |
| open `> ` mid-doc | 787.3 ms | 1.35× | 0.003 ms | — |
| open ` ``` ` mid-doc | 346.9 ms | 3.06× | 0.003 ms | — |
| open `> ` at doc start | 1287.5 ms | **0.82×** | 0.016 ms | — |

1. **Strategy A — the documented approach — is worthless.** A one-character edit
   yields 1.15×. Opening a blockquote at the top of the document is **0.82× —
   slower than parsing from scratch.**
2. **Strategy B is the only thing that works, and tree-sitter does not give it to
   you.** Per-range layer trees let you skip untouched layers entirely. But you
   must implement the layer manager, range-diffing and layer lifecycle yourself.

Which is exactly why Helix wrote its own bindings crate.

## Field corroboration

| Report | Content |
|---|---|
| [Helix #4139](https://github.com/helix-editor/helix/issues/4139) | *"if I hold down backspace and then let go, helix keeps deleting for roughly 0.5 seconds."* Persisted with LSP off; only fixed by disabling syntax highlighting entirely. Profiling showed markdown parsing far costlier than Rust. |
| [Helix #3072](https://github.com/helix-editor/helix/issues/3072) | "Very slow *editing* of large files when tree-sitter is used" |
| [Neovim #18107](https://github.com/neovim/neovim/issues/18107) | *"the LanguageTree itself is not incremental, i.e. the childs of the language tree are recreated each time"* → "catastrophic performance" with many injections |
| [Helix 25.07 notes](https://helix-editor.com/news/release-25-07-highlights/) | *"a very large list in Markdown means thousands of small injections"*; Helix had an **O(N²)** injection lookup, later replaced with a hashtable |

**Helix switched away from the upstream Rust bindings.**
[PR #12972](https://github.com/helix-editor/helix/pull/12972), merged 2025-05-13,
replaced `tree-sitter` with [`tree-house`](https://github.com/helix-editor/tree-house)
(MPL-2.0). Stated reasons: correctness bugs, unblocking the C library upgrade
0.22 → 0.25, and 5–10% throughput. The decisive gain is Strategy B:

> "editing within one list item in a large list will only cause the re-parsing and
> rerun of injections queries for root layer and the edited 'inline' layer —
> **the minimum amount of work required**."

**Read this correctly: Helix had to build and maintain its own bindings crate to
make Markdown editing acceptable.** That is the true cost of that path.

## The grammar itself is not fit for this purpose

`tree-sitter-md` 0.5.3, MIT, 1.03M downloads. Repo moved to
[tree-sitter-grammars/tree-sitter-markdown](https://github.com/tree-sitter-grammars/tree-sitter-markdown).

**Its own README disclaims correctness:**

> "It is **not recommended to use this parser where correctness is important**."
> … "There are still **lots of inaccuracies in the output**" … the purpose is
> "to provide syntactical information for **syntax highlighting**."

Open correctness issues:

| Area | Issue | Opened | State |
|---|---|---|---|
| Link reference definitions | [#5](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/5) | 2021-12-01 | **Open, 4.5 yrs** |
| Setext headings — `====` H1 not parsed at all | [#59](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/59) | 2022-09-04 | **Open, ~4 yrs** |
| HTML block type 4 **consumes the entire document** | [#233](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/233) | 2026-03-18 | Open |
| HTML tag parsed as code span | [#6](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/6) | 2021-12-01 | Open |
| "Emphasis does not always work as intended" | [#8](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/8) | 2021-12-01 | Open |
| Codespan parsed as autolink | [#7](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/7) | 2021-12-01 | Open |
| Tables — empty cells, whitespace, inconsistent pipe errors | [#230](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/230), [#241](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/241), [#242](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/242) | 2026 | Open |
| Single-tilde strikethrough pairs greedily across a paragraph | [#236](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/236) | 2025 | Open |
| 4-space nested list marker mis-handled | [#127](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/127) | 2023-11-29 | Open |
| Crashes on tree-sitter 0.26.0 above ~10K chars | [#251](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/251) | 2026-07-07 | Open |

**Link reference definitions cannot be resolved at all.** The grammar is
single-pass, so `[foo]` cannot know whether `[foo]: /url` appears 500 lines later.
For a WYSIWYG editor this is fatal on its own.

### A memory-safety bug that runs on every keystroke

[#243](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/243),
filed 2026-04-28 by the Emacs tree-sitter maintainer. **Open, zero comments.**

From `scanner.c`:

```c
static unsigned serialize(Scanner *s, char *buffer) {
    unsigned size = 0;
    buffer[size++] = (char)s->state;
    buffer[size++] = (char)s->matched;
    buffer[size++] = (char)s->indentation;
    buffer[size++] = (char)s->column;
    buffer[size++] = (char)s->fenced_code_block_delimiter_length;
    size_t blocks_count = s->open_blocks.size;
    if (blocks_count > 0) {
        memcpy(&buffer[size], s->open_blocks.items,
               blocks_count * sizeof(Block));   // <-- no bound vs 1024-byte buffer
        size += blocks_count * sizeof(Block);
    }
    return size;
}
```

No bounds check against `TREE_SITTER_SERIALIZATION_BUFFER_SIZE`. A deeply-nested
document overflows and corrupts the stack. **This function runs on every single
keystroke.**

### Maintenance

- **MDeiml's last commit: 2024-02-18.** Original author inactive ~2.5 years.
- Now maintained by `clason` and `lewis6991` — **Neovim core people**. Last
  substantive commit 2026-05-06; last release 2026-02-26. Recent pushes are
  dependabot CI bumps.
- **nvim-treesitter was archived 2026-04-03** after a maintainer conflict. Since
  the grammar's de-facto maintainers are the Neovim ecosystem, that is a direct
  bus-factor risk.

## Zed runs both parsers — the strongest available signal

`Cargo.toml:851`:

```toml
tree-sitter-md = { git = "https://github.com/zed-industries/tree-sitter-markdown", rev = "b596e73" }
# fork of 9a23c1a with serialize() buffer-overflow fix
# https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/243
```

`Cargo.toml:753`:

```toml
pulldown-cmark = { version = "0.13.0" }
```

used by `crates/markdown` and `crates/markdown_preview`.

**tree-sitter-md (forked, patched for memory safety) drives editor syntax
highlighting. `pulldown-cmark` does the actual Markdown semantics for rendering
and preview.**

A team with Max Brunsfeld on staff — the people who *invented* tree-sitter — chose
not to use tree-sitter for Markdown → rendered output. And they ship a fork
purely to patch a stack-corrupting overflow.

## Decision matrix

| Criterion | tree-sitter-md | pulldown-cmark |
|---|---|---|
| CommonMark correctness | Self-disclaimed: *"not recommended where correctness is important"* | CommonMark-conformant with a spec test suite |
| Link reference definitions | Cannot resolve ([#5](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/5), open 4.5 yrs) | Resolved correctly |
| Setext headings | [#59](https://github.com/tree-sitter-grammars/tree-sitter-markdown/issues/59) open ~4 yrs | Correct |
| Source positions | Byte ranges only; rebuild semantics yourself | Events carry byte offsets |
| Memory safety | Open stack-corrupting overflow; Zed ships a fork to patch it | Safe Rust |
| Incremental performance | 0.60×–2.3×; **often worse than cold** | N/A — full parse is 3 ms |
| Integration cost | Two grammars, two trees, layer manager, injection queries, range diffing | One iterator |
| Maintenance risk | Author gone since 2024; maintainers are nvim ecosystem, whose flagship plugin was just archived | 124M downloads; active (0.13.4, 2026-05-20) |
| Binary size | ~4.3 MB generated C (2.07 MB block + 2.26 MB inline `parser.c`) | Pure Rust, 440 KiB |

## Where tree-sitter would still win

**Syntax highlighting inside fenced code blocks**, scoped to fence contents with
the appropriate language grammar. That is exactly the split Zed arrived at.

Worth revisiting against syntect for that specific job — noting that
`tree-sitter-highlight` is itself not incremental (see
[03](03-architecture.md#syntax-highlighting-for-fenced-code)).

## Tree-sitter core, for the record

| Fact | Value |
|---|---|
| Latest | 0.26.11, 2026-07-12, MIT |
| Downloads | 29.70M total |
| Repo | 26,429★, 132 open issues, pushed 2026-07-25 |
| 1.0 status | [Checklist #930](https://github.com/tree-sitter/tree-sitter/issues/930) **open since 2021-02-20**. Still unchecked: formalizing the query spec. |
| Maintainer | **No handover announcement found.** Max Brunsfeld still committing (May 2026), at reduced cadence. Day-to-day load carried by `amaanq` and `ObserverOfTime`. Healthy but bus-factor thin. |

**Grammar/core ABI coupling is the dominant version-management cost**, not the
Rust API surface. Helix was stuck on C library v0.22 until it rewrote its
bindings to reach v0.25.

## Benchmark limitations

Stated plainly:

- Single machine (WSL2), synthetic documents
- Wall-clock timing without statistical rigour
- The Strategy-B layer manager is a simplification of what tree-house actually
  does
- Magnitudes are 50×–360×, far larger than that noise, so the *conclusions* hold —
  but treat precise figures as indicative

## What could not be determined

- No dedicated open issue for **entity references** or **lazy continuation** in
  tree-sitter-markdown. Absence of an issue is not evidence of correctness, given
  the README's blanket disclaimer.
- Whether tree-sitter preserves old-tree reuse across an `included_ranges` change
  — undocumented, and the mechanism the split Markdown grammar depends on. Only
  matters if the recommendation above is rejected.
- **Typora and iA Writer are closed source.** No architecture details found; no
  inference attempted.
- MarkText's docs do not state whether its lexer re-runs on the full document per
  keystroke.
