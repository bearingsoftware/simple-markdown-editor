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
| Cold start → interactive paint | **~250 ms on this hardware, recorded not budgeted** | — | Revised by MDEDIT-20 — see below. winit plus wgpu plus driver; creating a window alone exceeds 50 ms |
| Warm start | ≤30 ms | ≤16 ms | Indistinguishable from an already-open window |
| Keystroke→pixel, software-only **p50** | **≤3 ms** | ≤1 ms | GVim proves 0.9 ms is achievable; VS Code's 31.2 ms is 10× the budget |
| Keystroke→pixel **p99** | **≤8.3 ms** | ≤6 ms | One 120 Hz frame. Budget the tail explicitly — Luu's 99.9p reached 111 ms |
| **p50 on battery** | **≤8 ms** | ≤5 ms | The discriminating test. Nobody benchmarks it |
| Frame render | ≤4 ms | ≤2 ms | Matches Zed's measured figure, with headroom for 240 Hz |
| File open → first paint, **process already running** | **O(1), ≤50 ms at any size** | ≤16 ms | Scoped by MDEDIT-20 — see below, and **met**: 2.4 ms at 245 KB, 3.0 ms at 357 KB. The bet that makes competitors' numbers irrelevant |
| File open, fully indexed | ≤10 ms/MB | ≤5 ms/MB | Vim/Helix exceed 200 MB/s; Zed 36, VS Code 26, Sublime 22 |
| Max file size | **no limit below RAM** | — | **Typora dies at 2 MB** |
| Baseline RSS, editor's own | **≤40 MB over the graphics floor** | ≤20 MB | Revised by MDEDIT-19 — see below. The original ≤50 MB total is unreachable: an empty wgpu window costs 124 MB here |
| Baseline RSS, total | **≤170 MB on this hardware** | — | Floor plus our budget. Re-measure per machine; the floor is not ours to move |
| Memory per open file | ≤1.2× file bytes | ≤1.05× | Vim: 240 MB for a 200 MB file |
| 100 open notes (~50 KB each) | ≤80 MB | ≤60 MB | **No published data exists** — you would set the benchmark |
| Markdown parse + segment, **viewport** | ≤1 ms | ≤0.5 ms | Measured at 6 KB: 0.42 ms total, 0.11 ms of it segmentation ([04](04-parsing-benchmarks.md)) |
| Markdown parse + segment, whole 60 KB document | ≤16 ms (one frame) | — | Background/indexing path, not the keystroke path — see below |


### The 50 MB baseline was measuring the wrong thing

**Decision, MDEDIT-19 (2026-07-29).** The ≤50 MB baseline RSS target is unreachable on this stack,
and no amount of care in this codebase will reach it. Measured on Windows 11, Intel integrated
graphics, release build, 900×700 window:

| Process | Working set | Private |
|---|---:|---:|
| Empty winit + wgpu window, all backends | 346.3 MB | 251.7 MB |
| Empty winit + wgpu window, **DX12 only** | **124.0 MB** | 100.5 MB |
| Editor, all backends | 345.8 MB | 260.5 MB |
| Editor, **DX12 only** | **134.4 MB** | 117.5 MB |

The probe is `spikes/gpu-baseline` — a window, a surface, a device, and a render pass that clears to
a colour and draws nothing. It is re-runnable on other hardware, and it should be re-run before
anyone treats these numbers as universal.

Two things follow.

**Backend selection was costing 211 MB.** `wgpu::Instance::default()` enables every backend it was
compiled with, and each initialises its own driver stack — Vulkan and OpenGL were being loaded and
paid for although nothing here would ever use them. Restricting the instance to DX12 on Windows cut
the editor from 345.8 MB to 134.4 MB, a 61% reduction, with the hardware adapter unchanged and
`vulkan-1.dll` no longer loaded at all. That is now what the app does.

**The remaining 124 MB is not ours.** An empty window costs within 1 MB of the entire editor before
the change, and 124 MB after it. The editor's own contribution is about 10 MB — a 2048² glyph
atlas held CPU-side and as a texture, an instance buffer, and the document. Targeting 50 MB total
would mean not using wgpu, which is a decision `02-gui-toolkits.md` already made on other grounds
and which this does not reopen.

So the target splits: **the editor's own footprint gets a budget we can actually enforce**, and the
floor gets recorded rather than pretended away. This is the same move MDEDIT-4 made when the
keystroke budget turned out to be scoped wrongly — the target was not wrong, what it was measuring
was.

**What would change it:** a lower-level backend, or a wgpu release that stops eagerly initialising
unused backends. Neither is worth pursuing now. If the floor matters more later — a memory-
constrained target, or a competitive claim — re-run the probe first, because it answers the question
in about a minute and costs nothing.


### The ≤50 ms file-open target measures two different things

**Decision, MDEDIT-20 (2026-07-31).** `file open → first paint ≤50 ms at any size` is unreachable
from a cold process on this stack, and comfortably met once a process exists. It was one target
covering both, and it splits.

MDEDIT-18 measured 313 ms for a five-byte file and 571 ms for a 245 KB one, of which loading and
parsing the document was 9–10 ms. So the document was never what made opening slow. `spikes/
startup-attribution` brings up the same window and the same pipeline the editor does and times each
step. On Windows 11, Intel integrated graphics, release build, DX12 only — which is what the editor
already restricts to, for the memory reason above:

| Step | Typical | What it is |
|---|---:|---|
| `EventLoop::new` | 12 ms | winit |
| `create_window` | 60–79 ms | winit, and Win32 under it |
| `Instance::new` | 16–26 ms | wgpu, DX12 only |
| `create_surface` | 0.2 ms | — |
| `request_adapter` | 70–100 ms | DXGI enumeration and the driver |
| `request_device` | 62–72 ms | the driver |
| `surface.configure` | 39–54 ms | swapchain creation |
| `create_shader_module` | **0.3 ms** | — |
| `create_render_pipeline` | **0.5–11 ms** | — |
| First frame | 4–6 ms | ours |
| **Total** | **235–340 ms** | |

**Two hypotheses on the ticket died.** Shader compilation is 0.3 ms and pipeline creation under
11 ms, so moving either off the critical path buys nothing — there was no reason to think otherwise
except that it is the usual answer elsewhere. And `PowerPreference` makes no measurable difference
to `request_adapter`: `LowPower` 97–106 ms, `None` 117–124, `HighPerformance` 94–101, which is noise
either side of 100.

**Backend restriction was already banked.** With wgpu's default backends `Instance::new` costs
126–383 ms rather than 16–26. The editor has restricted to DX12 since MDEDIT-19, for memory, and got
this for nothing.

**What is left is not ours.** Creating a window alone exceeds the 50 ms target before any GPU work
starts. Everything above it is winit, DXGI, or the display driver. There is no arrangement of this
code that reaches 50 ms from a cold process, and the only thing that would change it is not using
wgpu — a decision `02-gui-toolkits.md` made on other grounds and this does not reopen.

**So the target splits, the way the RSS one did:**

| Target | Value | What it means |
|---|---|---|
| Cold start → first paint | **~250 ms on this hardware, recorded rather than budgeted** | winit plus wgpu plus driver. Re-measure per machine; it is not ours to move |
| File open → first paint, process already running | **≤50 ms at any size — met, with room** | Measured 2.4 ms at 245 KB and 3.0 ms at 357 KB, against a 50 ms budget |

That second row is the one that matters for the product, and it is met with room to spare: opening a
file into a running editor and drawing a frame takes **2.4 ms at 245 KB and 3.0 ms at 357 KB**,
against a 50 ms budget. `AGENTS.md` already requires that opening N files from Explorer reuses one
process, via a `DropTarget` verb — so for every file after the first, the 250 ms does not recur and
this is what the user experiences. The cold-start figure is what somebody pays once.

Splitting the target is not a way of declaring victory over the hard half. The 250 ms is real, it is
what someone sees when they double-click a `.md` file with no editor running, and it is recorded in
the table above rather than hidden here.

**What would change it:** a wgpu release that defers device creation, or showing the window before
the device exists. The second is possible and deliberately not done — it would mean presenting a
blank window for 200 ms, which makes the number look better and the editor feel worse. If it is ever
wanted, it is a product decision rather than a performance one.

**Now measured.** Cold start and file open → first paint were the last unmeasured targets; MDEDIT-20
attributed both, above. The 1176 ms that MDEDIT-17 measured incidentally at 239 KiB was mostly the
document, and MDEDIT-17 and MDEDIT-18 removed that part — what is left is the ~250 ms of window and
device bring-up, which is not the document's and not ours.

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

## What synchronous I/O on the UI thread actually costs

**Measured, MDEDIT-151 (2026-09-05), release build, Windows 11, i7-1270P, 32 GB, on AC.** `AGENTS.md`
claimed I/O went to a worker thread. It does not — there is no worker anywhere in `crates/*/src`, and
MDEDIT-146 inherited the claim as a constraint it did not have. Before correcting the sentence, the
question was whether the worker was worth building instead. These are the numbers that answered it.

**Read the best column and treat the worst as a warning, because five runs did not converge.** This
section began with one column of single figures. Re-running disagreed, so it became a range; the range
was then broken by the fourth run and broken again by the fifth. What the same 4K decode measured, run
by run: **65, 84, 70, 159, 130 ms.** What the same 16-megapixel decode measured: **110, 130, 132,
2543, 252 ms.**

The `1270P` is a thermally-throttled laptop part and the machine was doing other work by the fourth
run — that one overlapped a workspace build, which is where the 2.5 s comes from. That is not a
defect in the editor and not a number to quote; it is what this hardware does to any measurement taken
carelessly, and the reason the harnesses print both ends. **Run them on an idle, cold machine, twice,
and believe the smaller answer.** The power state was checked afterwards rather than per run, which
the targets above say to do and this did not.

So the tables give the **best** seen across every run — the most favourable honest reading — and the
worst seen on an *idle* machine beside it.

`mdedit-image`, decode, gradient content so the decompressor does real work:

| PNG | best | worst, idle | RGBA surface |
|---|---:|---:|---:|
| 640×480, a screenshot | 2.6 ms | 5.6 ms | 1.2 MB |
| 1920×1080 | 16.2 ms | 32.3 ms | 7.9 MB |
| 3840×2160, a 4K screenshot | **65.3 ms** | 150.5 ms | 31.6 MB |
| 4000×4000, `MAX_PIXELS` | **110.0 ms** | 278.6 ms | 61.0 MB |

`mdedit-file`, load and save:

| Markdown | load, best | load, worst | save, best | save, worst |
|---|---:|---:|---:|---:|
| 245 KB | 14.6 ms | 43.4 ms | 2.4 ms | 4.7 ms |
| 1 MB | 15.9 ms | 24.5 ms | 3.2 ms | 5.4 ms |
| 4 MB | 19.2 ms | 33.8 ms | 4.7 ms | 10.2 ms |
| 16 MB | 39.8 ms | 98.7 ms | 15.1 ms | 31.0 ms |

**The decode is the one that matters, and that reversed what MDEDIT-151 assumed.** The ticket said a
worker built for images alone would be "a worker for the least important of the four". It is the most
expensive by a wide margin in the case people actually hit, and this is the conclusion the drift cannot
touch: **the best 4K decode ever measured, 65 ms, is already over the ≤50 ms file-open target**, and
every other run was worse — twice over on a warm machine. It is eight times the 8.3 ms p99 keystroke
budget at best and twenty times at worst. Nothing about how this was measured makes that go away, and
it is the one number here that bought a worker thread (MDEDIT-156, below).

**Loading is where the drift bites, and it is an open question rather than a result.** 16 MB spans
40–99 ms: under the target on the best run and twice it on the worst. Neither "loading is fine" nor
"loading breaches the target above ~10 MB" is supported. What can be said is that no *realistic*
Markdown file gets near 50 ms, and 16 MB of Markdown is not a file anybody has. Saving never
approaches the target at any size.

**Save is one column because the two save paths measured the same.** `mdedit_file::save` writes the
original bytes back when the buffer still equals what was read, and encodes the buffer when it does
not — the fast path exists for byte preservation, not for speed. Timed apart, saving an edited 16 MB
buffer came in at 21–29 ms against 21–31 ms unedited, inside the noise. Encoding UTF-8 that was read
as UTF-8 is a copy; the fast path is not what makes saving cheap.

Three artefacts worth knowing before quoting the load column. The 245 KB row is the first size the
harness touches, so its worst iteration is paying for whatever the process and page cache had not
warmed yet — that is where the 43 ms comes from, not from the size. Every load reads a file written
moments earlier, so this measures a warm cache throughout; a genuinely cold open off disk is not
covered here at all. And nothing here is `mmap` — `mdedit_file::load` does a whole-file `fs::read`,
so these figures are the cost of the read the editor performs today, not of the O(1) first paint the
Rendering rules describe.

**One thing this section does not measure, and should be read as excluded rather than absent.**
`PictureKey::of` calls `std::fs::metadata` for every picture destination it lays out, on the same
thread. It is a stat rather than a read, and it sits behind the layout cache so it runs on a miss and
not per frame, but it is synchronous I/O on the thread that draws and no number here covers it.

**It was a one-off, not a per-frame cost.** A picture is decoded once and cached by path, length and
mtime (MDEDIT-146), so the stall landed on the frame that first laid the image out and never again.
That is why it was recorded here rather than fixed on the spot: a single hitch at open is not the same
defect as a hitch while typing.

**Fixed on MDEDIT-156, and the split is worth reading before proposing another worker.** The obvious
version of the fix has a visible cost — decode off-thread, and the first frame has no dimensions, so
it draws alt text and the page reflows when the image pops in. What removed that is that a PNG's
*header* is not expensive: `mdedit_image::measure` reads a few dozen bytes on the drawing thread, so
the row is the right height on the frame the image first appears in, and only the pixels are late.
Nothing moves when they land.

That also keeps it clear of MDEDIT-118's trap. The layout is a function of the header alone, so it
does not change when the decode completes and `ChunkKey` is not missing anything it would need to
carry. `decoding.rs` has the reasoning.

`crates/image/tests/decode_cost.rs` and `crates/file/tests/load_cost.rs` are the harnesses, both
`#[ignore]`d beside `find_cost` and `frame_cost`. Re-run them before believing any of the above.
