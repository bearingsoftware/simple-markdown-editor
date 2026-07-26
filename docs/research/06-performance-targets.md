# 06 — Performance targets

What "ridiculously fast" means in numbers, what the competition actually
measures at, and where the measurement literature is empty enough to be an
opportunity.

## The state of the evidence

**Almost nobody publishes real numbers for this app category.** There is no
benchmark suite, no comparative timing study, no reproducible measurements for
Markdown editors. The top layer of search results for "editor performance" is
AI-generated content citing fabricated figures — the widely-circulated Zed
"0.12 s startup / 222 MB / 2 ms latency" numbers are not Zed claims and should
not be used.

Zed's own [1.0 announcement](https://zed.dev/blog/zed-1-0) contains **no
performance numbers at all**, and Zed has never published p50/p99 input latency.

That gap is an opportunity — see [Publish your own numbers](#publish-your-own-numbers).

## Measured baselines

### Cold start and large-file open

The only credible multi-editor hands-on set is
[MakeUseOf 2026](https://www.makeuseof.com/ranked-fastest-code-editors-quickest-isnt-talking/)
(Linux Mint, 200 MB log file):

| Editor | Cold start | 200 MB file open | RSS |
|---|---|---|---|
| Vim | ~170 ms | <1 s | 240 MB (with file) |
| Helix | ~170 ms | <1 s | 371 MB |
| Zed (native) | 500–600 ms | 5.5 s | ~350 MB idle |
| Sublime Text | 1.38 s | ~9 s | 644 MB |
| VS Code | 5.5–6 s | 7.7 s | ~1.5 GB idle |
| Cursor | 2.7 s | **failed — killed past 4.4 GB** | ~500 MB idle |

**Sublime's large-file reputation is not supported here** — it was the slowest
that completed.

### Typing latency

[Pavel Fatin / Typometer](https://pavelfatin.com/typing-with-pleasure/), Windows,
empty file, software-only, mean (max) in ms:

| GVim | IDEA zero-latency | Notepad++ | Emacs | Sublime | Eclipse | IDEA default | Atom |
|---|---|---|---|---|---|---|---|
| **0.9** (1.2) | **2.9** (21.2) | **4.3** (5.9) | **5.3** (19.2) | **8.2** (35.2) | **10.1** (20.8) | **24.7** (83.7) | **49.4** (85.5) |

Modern Typometer run ([beuke.org 2024](https://beuke.org/terminal-latency/)):
**VS Code 1.87.2 = 31.2 ms mean**, chromium 19.6 ms, firefox 24.1 ms, gvim 8.0 ms.

Those browser-engine figures are why a webview-based editor cannot hit the target
— see [02](02-gui-toolkits.md).

### The finding almost nobody accounts for: power-save

Fatin's Windows XML numbers, on battery:

| Editor | On AC | On battery |
|---|---|---|
| Emacs | 5.3 ms | **50.5 ms** |
| IDEA default | 24.7 ms | **45.5 ms** (max 239) |
| Sublime | 8.2 ms | 12.6 ms |
| GVim | 0.9 ms | 1.4 ms |

**Every published benchmark is a mean on a plugged-in workstation — the least
informative condition available.** Hold your latency on battery and you win
outright, because nobody else measures it.

### Tails, not means

[Dan Luu's terminal work](https://danluu.com/term-latency/) found idle medians of
5–44 ms but **99.9th percentiles of 17–111 ms**. Tails are where perceived jank
lives.

For end-to-end context: [Luu's input lag work](https://danluu.com/input-lag/)
measured a 2014 desktop at 165 Hz at **50 ms**, and an **Apple IIe from 1983 at
30 ms**.

### Perception thresholds

- Users detect dragging latency down to **2.38 ms**, and *"noticeable improvement
  continued well below 10 ms"* ([Ng et al., UIST 2012](https://dl.acm.org/doi/10.1145/2380116.2380174)).
- Tapping latency becomes imperceptible ~24 ms; dragging performance degrades
  above 25 ms ([Jota et al., CHI 2013](https://www.tactuallabs.com/papers/howFastIsFastEnoughCHI13.pdf)).

**There is no perceptual floor at 10 ms.** Faster keeps being better.

### Zed's actual published numbers

Thinner than the discourse suggests: 8.33 ms frame budget at 120 Hz and
*"consistently under 4ms"* measured frame times
([zed.dev/blog/120fps](https://zed.dev/blog/120fps)), plus an unmethodologied
"<10 ms typing latency" on their comparison page. That is all.

## Competitive gifts

Failures in the incumbents that a viewport-virtualized design makes irrelevant:

- **Typora has a hard 2 MB render ceiling.** `MAX_FILE_SIZE = 2e6`, producing
  "The file is too large to render in Typora."
- **Typora typing lag on a 281k-char document vanishes when switching CSS theme**,
  same document, same machine
  ([typora-issues#6595](https://github.com/typora/typora-issues/issues/6595)) —
  it is CSS-driven relayout, not text-engine bound.
- **MarkText freezes 4–5 minutes on a ~300k-word file**, post-rewrite
  ([#4887](https://github.com/marktext/marktext/issues/4887)). Obsidian opens the
  same file in seconds.
- **Obsidian:** ~4 s between keystrokes in link autocomplete on large vaults; a
  measured **6-minute cold start** on a 43,049-file vault with all community
  plugins disabled.
- **Joplin:** ~1.5 GB idle, [closed as not planned](https://github.com/laurent22/joplin/issues/12354).
- **VS Code** hangs indefinitely on a 35 MB JSON with word wrap on
  ([#197715](https://github.com/microsoft/vscode/issues/197715)).
- **Notepad++** disables session snapshots entirely above its large-file
  threshold — "opens huge logs" and "never loses your work" are mutually
  exclusive there. See [05](05-windows-platform.md).

## Targets

| Metric | Target | Stretch | Basis |
|---|---|---|---|
| Cold start → interactive paint | **≤100 ms** | ≤50 ms | Beats Vim/Helix (170 ms), the fastest credibly measured; 5× better than Zed native |
| Warm start | ≤30 ms | ≤16 ms | Indistinguishable from an already-open window |
| Keystroke→pixel, software-only **p50** | **≤3 ms** | ≤1 ms | GVim proves 0.9 ms is achievable; VS Code's 31.2 ms is 10× the budget |
| Keystroke→pixel **p99** | **≤8.3 ms** | ≤6 ms | One 120 Hz frame. Budget the tail explicitly — Luu's 99.9p reached 111 ms |
| **p50 on battery** | **≤8 ms** | ≤5 ms | The discriminating test. Nobody benchmarks it |
| Frame render | ≤4 ms | ≤2 ms | Matches Zed's measured figure, with headroom for 240 Hz |
| File open → first paint | **O(1), ≤50 ms at any size** | ≤16 ms | mmap + parse only the visible viewport. This is the bet that makes competitors' numbers irrelevant |
| File open, fully indexed | ≤10 ms/MB | ≤5 ms/MB | Vim/Helix exceed 200 MB/s; Zed 36, VS Code 26, Sublime 22 |
| Max file size | **no limit below RAM** | — | **Typora dies at 2 MB** |
| Baseline RSS | **≤50 MB** | ≤25 MB | Vim's implied ~40 MB; VS Code idles at 30× that |
| Memory per open file | ≤1.2× file bytes | ≤1.05× | Vim: 240 MB for a 200 MB file |
| 100 open notes (~50 KB each) | ≤80 MB | ≤60 MB | **No published data exists** — you would set the benchmark |
| Markdown parse + segment, **viewport** | ≤1 ms | ≤0.5 ms | Measured at 6 KB: 0.42 ms total, 0.11 ms of it segmentation ([04](04-parsing-benchmarks.md)) |
| Markdown parse + segment, whole 60 KB document | ≤16 ms (one frame) | — | Background/indexing path, not the keystroke path — see below |

### The 3 ms budget is viewport-scoped

**Decision, MDEDIT-4 (2026-07-26).** The keystroke budget cannot be met by parsing and segmenting the
whole document, and no optimisation will change that: `pulldown-cmark` alone takes 1.6–3.6 ms on a
60 KB document, so the ≤3 ms p50 target is already spent before segmentation starts. MDEDIT-4 cut
segmentation overhead by ~60% and the arithmetic still does not work.

The target is not wrong. Its **scope** was.

- **Keystroke path — viewport only.** Parse and segment the visible region plus enough context to
  resolve open blocks, not the document. At 6 KB — a generous viewport — that is 0.42 ms total, well
  inside 3 ms with room for layout and paint.
- **Whole-document — background.** Full segmentation is an indexing concern (outline, search, link
  resolution), runs off the keystroke path, and gets a one-frame budget rather than a 3 ms one.

This is not a new constraint. [03](03-architecture.md#virtualization-is-non-negotiable) already
requires that a document is never materialised whole and that `file open → first paint` is O(1) via
viewport parsing. This decision just states the consequence for the *editing* path, which had been
left implicit — and which the measurements would otherwise appear to contradict.

**Consequence for MDEDIT-4's acceptance criterion,** which asked for parse + segment of 60 KB inside
3 ms: unachievable as written, closed via its own "or a written decision that the budget moves"
clause. This is that decision.

**What would change it:** if viewport-scoped segmentation turns out to be impractical — because open
blocks force too much context, say — the fallback is block-level incremental parsing, which
[03](03-architecture.md#block-level-incrementality) already sketches and
[04](04-parsing-benchmarks.md) deliberately deferred. Do not reach for it before the renderer exists
to prove the need.

### Two rules for using these

1. **Measure tails and battery**, not means on AC power. That is where every
   published benchmark is weakest and where users actually feel pain.
2. **File open must be O(1).** mmap plus viewport-only parse. Every large-file
   failure in the competitive gifts list above comes from materializing the whole
   document. See [03](03-architecture.md#virtualization-is-non-negotiable).

## Publish your own numbers

Nobody in this category publishes reproducible benchmarks with methodology. A
public results table plus the script that produced it would be genuinely
differentiating, and cheap to build:

- **Typometer is open source.** No measured typing-latency figure exists for
  Typora or Obsidian anywhere — that is a real hole in the literature.
- Binary size for Rust GUI toolkits is unpublished by everyone. Three
  hello-worlds and a `dir` is a ten-minute experiment.
- Per-document memory for parley vs cosmic-text is unpublished. Directly relevant
  to "many files open."

Ship the harness alongside the editor. Report p50 **and** p99, on AC **and** on
battery, and state the machine.
