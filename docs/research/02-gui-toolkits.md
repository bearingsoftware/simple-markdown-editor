# 02 — GUI toolkit evaluation

Verified 2026-07-26 against Slint git HEAD `814caf5`, crates.io API, and the
respective issue trackers.

## The premise that reframes everything

**No Rust GUI toolkit has a rich-text editing widget. None of them.**

All of them give you styled *display* text and plain-text-with-syntax-colouring
*editing*. You are writing the text view yourself in every scenario. So a toolkit
is only buying you windowing, input routing and a paint API — which is a much
smaller purchase than it first appears, and it changes what "choosing a toolkit"
even means.

---

## Slint — rejected

Requested specifically. The answer is no, and unusually clearly so. Every layer
needed is absent or explicitly out of scope.

### Text editing is the disqualifier

**`TextEdit` is plain-text only.** Its entire API is `font-size`, `font-family`,
`font-italic`, `text: string`, `read-only`, `wrap`, alignment, placeholder,
viewport props, plus select/copy/cut/paste. One flat `string`. No per-span
anything. It does not even have `font-weight` —
[#9022 "Complete public API for TextEdit"](https://github.com/slint-ui/slint/issues/9022),
open since 2025-07.

**`StyledText` (new in 1.16) is display-only by explicit design.** Scope
statement from [#9560](https://github.com/slint-ui/slint/issues/9560),
closed/completed, verbatim:

> "providing the APIs to feed attributed text into Slint and rendering, **NOT**
> to support the end-user editing said text."

Maintainer `tronical` in [discussion #10859](https://github.com/slint-ui/slint/discussions/10859)
(2026-02-24), answering someone building a code editor:

> "I don't think we'll have markdown support in the editor (TextEdit/TextInput) —
> for now it'll be a separate element for displaying only."

**And `StyledText` cannot render Markdown anyway.** From the
[docs](https://docs.slint.dev/latest/docs/slint/reference/elements/styledtext/),
unsupported verbatim: *"Headings, Images, Tables, Block Quotes, Subscripts,
Superscripts, Horizontal Rules, Footnotes, Math expressions, Other HTML tags."*

**It cannot render `# Heading`.** So it is not even adequate as a preview pane.

**Syntax highlighting in an editable widget:** not supported. Maintainer `hunger`,
[discussion #3401](https://github.com/slint-ui/slint/discussions/3401):
*"I am sorry, that is not supported yet."* That was 2023-09. Still true.

### The existing TextEdit is unusably slow

[#10087 "TextEdit is far too slow"](https://github.com/slint-ui/slint/issues/10087),
**open**. A commercial user:

> "The TextEdit Control is very very slow, We were forced to create our own
> CustomTextEdit control able to handle 1M lines of text. the current TextEdit
> becomes unuseable at more that 5K lines of text, at 50K it takes over a minute
> on an i7-10gen machine to insert the text... we had to spend an additional
> 4 weeks working on a replacement. we used the ropey crate."

`tronical` conceded: *"It's true that we do way too much work for large text
input fields right now."* After layout caching landed on master, a retest with
**15,000 lines** (2026-02-23):

> "Clicking on the text completely freezes the entire application for several
> seconds, sometimes more than 20 seconds." — same in release build.

The fix is a community PR, [#12039](https://github.com/slint-ui/slint/pull/12039),
AI-assisted, still open. `tronical` (2026-07-20): *"I'm not sure about the
timeline, but it's now on my TODO."*

### Tracking issues — nothing is moving

| Issue | Opened | Status |
|---|---|---|
| [#1325](https://github.com/slint-ui/slint/issues/1325) Text document data structure (rope/piece table) for undo/redo + rich text | 2022-06-07 | **Open, no PRs, no progress in 4 years** |
| [#2723](https://github.com/slint-ui/slint/issues/2723) Rich text editor | 2023-05-11 | Open, no milestone, no PRs |
| [#432](https://github.com/slint-ui/slint/issues/432) Canvas painting API | 2021-08-25 | **Open ~5 years**, `roadmap` label, last user ping 2026-01 with no maintainer reply |
| [#3811](https://github.com/slint-ui/slint/issues/3811) IME in the public Platform API | 2023-10 | Open — *"implemented (in Rust) but still internal"* |
| [#7841](https://github.com/slint-ui/slint/issues/7841) Text editing with bidirectional text | 2025-03-10 | Open |
| [#2895](https://github.com/slint-ui/slint/issues/2895) Accessibility for text inputs | 2023 | Open |

There are **60 open issues** labelled `a:text`.

### Windows specifics — several are fatal for a text editor

**Good:** native menus via real Win32 `HMENU` (`muda`), since 1.10.0. Dark mode
follows the system setting live via `WindowEvent::ThemeChanged`. Multi-window
since 1.7, system tray in 1.17.

**Bad:**

- **[#10499](https://github.com/slint-ui/slint/issues/10499) — text rendering
  broken at fractional DPI scales**, i.e. 125% / 150%, the Windows defaults.
  Specifically fatal here.
- **Dropping a `.md` file from Explorer onto the window is not possible in 1.17.x.**
  `DragArea`/`DropArea` are within-window only. File support
  ([#12550](https://github.com/slint-ui/slint/pull/12550)) is **Qt-backend only**;
  the winit implementation is merged into an *unmerged* branch gated on an
  unreleased winit. Windows uses winit. Realistic ETA 1.18.
- **[#11073](https://github.com/slint-ui/slint/issues/11073)** — fixed-size windows
  grow unboundedly when dragged between different-DPI monitors; source carries a
  literal `// TODO: send a resize event`.
- **[#768](https://github.com/slint-ui/slint/issues/768) — no closeable/dynamic
  tabs**, open since 2021-12. `TabWidget` is static-only. Directly relevant.
- No built-in file dialogs ([#9781](https://github.com/slint-ui/slint/issues/9781)).
- No custom title bar ([#2521](https://github.com/slint-ui/slint/issues/2521),
  open since 2023).

### Renderer reality on Windows is worse than the docs imply

1. `cargo add slint` default features give you **FemtoVG on OpenGL** — Skia is
   not compiled in. Weakest GPU path on Windows.
2. If you do enable Skia, its default surface on Windows is **CPU, not D3D**.
   CHANGELOG 1.11.0: *"Skia renderer: Changed default on Windows to use the
   software renderer, instead of Direct3D"* — because of
   `DXGI_ERROR_DEVICE_REMOVED` crashes on NVIDIA
   ([#7641](https://github.com/slint-ui/slint/issues/7641)).
3. D3D is not reachable via `SLINT_BACKEND` at all, only the Rust API.
4. [#4039](https://github.com/slint-ui/slint/issues/4039), open since 2023-11:
   OpenGL driver crash on Remote Desktop disconnect/reconnect. Hits the *default*
   Windows configuration.

### Virtualization is real but broken for documents

`ListView` genuinely virtualizes (100k items works — a debug-build test showed
~4s startup). But the scroll extent is **extrapolated from the average height of
currently visible items** (`repeater.rs:342-346`), so with heterogeneous item
heights the scrollbar thumb resizes and content jumps. Scrolling more than ~1.5
screens destroys all instances and *guesses* the offset. CHANGELOG 1.17.0 still
lists *"ListView: Fixed drag with differing height items."*

You cannot route around it —
[discussion #7986](https://github.com/slint-ui/slint/discussions/7986): a
maintainer states the virtualization is Rust-internal and *"it is at this point
not possible to implement it the same way for a custom ListView."*

Note also a **1.16.0 regression made a 100k-row table take 4 minutes**, shipped,
and was fixed in 1.16.1. This area regresses.

### No text-heavy app has ever shipped on Slint

All ~42 entries on [madewithslint.com](https://madewithslint.com/) enumerated.
**No code editors, no IDEs, no note apps, no markdown editors, no word
processors.** Nearest real desktop apps: Koharu (manga translation), Raccoin,
Cargo UI, ImageSieve, WSL Dashboard. The rest is games (11), toy utilities, and
embedded/HMI.

Slint says so themselves, in
[Making Slint Desktop Ready](https://slint.dev/blog/making-slint-desktop-ready):

> "Over the last five years, it has become the preferred option for building GUIs
> and HMIs for **embedded systems**"
> "we often encounter **questions or concerns about whether Slint is a viable
> option** for creating full-featured desktop applications"

The text editors that do exist are hobby-scale: Oryn (1★), sred (0★),
slint-codeeditor (1★). That last one documents the fight — **Slint doesn't
support nested `for` loops**, so the author flattened every token from every line
into one model with manual x-offsets, absolutely positioned inside a `Flickable`,
with a hand-rolled blinking cursor made from `Rectangle` + `animate opacity`.

### Licensing — fine, but read three clauses

Tri-licence: `GPL-3.0-only` OR
[`LicenseRef-Slint-Royalty-free-2.0`](https://github.com/slint-ui/slint/blob/master/LICENSES/LicenseRef-Slint-Royalty-free-2.0.md)
OR paid commercial. A free OSS desktop app qualifies under the royalty-free
licence; attribution (an AboutSlint widget or a download-page badge) is the only
cost.

Three things to note anyway:

1. **"MIT app" is partly a fiction.** Your files can carry MIT headers, but the
   distributable binary is not MIT-redistributable — a forker must independently
   elect GPLv3 or royalty-free.
2. **The "exposes the APIs" clause is a landmine for a plugin system.** *"The
   License does not permit the distribution of Application that exposes the APIs,
   in part or in total, of the Software."*
3. **Licence churn is real** — ~8 revisions in 3 years.

The licence is not why we're rejecting it. The toolkit simply cannot do the job.

### The single biggest risk, stated plainly

You would write your own text engine *and* your own text renderer, and Slint
gives you less help doing that than any other candidate.

- **No canvas** ([#432](https://github.com/slint-ui/slint/issues/432), 5 years).
- **No public text-measurement or shaping API** — `text_size`, `font_metrics`,
  `text_input_byte_offset_for_position`, `cursor_rect_for_byte_offset` all live
  on the private `RendererSealed` trait.
- **No IME hook for a custom view, at all.** `slint::platform::WindowEvent` has
  only `KeyPressed`/`KeyPressRepeated`/`KeyReleased` carrying a `SharedString`.
  No composition/preedit variants. **A hand-rolled text view in Slint cannot
  support CJK input or dead keys.** You would also lose accessibility, since AT
  integration is bound to built-in items.

That is not a temporary gap to wait out; it follows from `i-slint-core` being
sealed, and there is no public timeline for opening it.

There is a working escape hatch — render to a buffer/texture, wrap in
`slint::Image`, place as an ordinary `Image` element (verified in
[examples/wgpu_texture](https://github.com/slint-ui/slint/blob/master/examples/wgpu_texture/scene.slint)).
But **no partial updates on `slint::Image`**, so you re-upload every frame, and
the wgpu path is explicitly outside Slint's API stability guarantee.

### The bitter irony

Slint uses **parley** internally (migrated in 1.16 —
[#9559](https://github.com/slint-ui/slint/issues/9559),
[#9558](https://github.com/slint-ui/slint/issues/9558)). Parley has ranged
styles, bidi, and inline boxes — exactly the capability needed. Slint has it and
does not expose it. You would be building on a framework that is actively hiding
the thing you came for.

---

## The alternatives

| | **egui** | **iced** | **GPUI** | **Xilem/Masonry** | **Floem** | **Tauri v2** | **Dioxus+Blitz** | **windows-rs** |
|---|---|---|---|---|---|---|---|---|
| Latest | 0.35.0 (2026-06-25) | 0.14.0 (2025-12-07) | crate 0.2.2 (2025-10-22), tree far ahead | 0.4.0 (2025-10-29) | **0.2.0 (2024-11-14)** | 2.11.5 (2026-07-01) | blitz 0.3.0-beta.1 | `windows` 0.62.2 |
| Licence | MIT OR Apache-2.0 | MIT | Apache-2.0 **(disputed, see below)** | Apache-2.0 only | MIT | MIT OR Apache-2.0 | MIT OR Apache-2.0 | MIT OR Apache-2.0 |
| Styled runs in *editable* text | Yes — `LayoutJob`/`TextFormat` | **No** — `Highlight` exposes only colour + font | Yes (Zed does it) | **No** — `TextArea` wraps single-style `PlainEditor` | Unclear | Yes (it's HTML) | Yes (it's HTML) | Yes — `IDWriteTextLayout` |
| Inline images/widgets in text flow | **No** | **No** | Zed-internal only | **No** | **No** | Yes | Yes | Yes — `IDWriteInlineObject` |
| Custom render hatch | `PaintCallback` (backend-specific, silently no-ops on downcast mismatch) | **`Shader` widget** — cleanest | Full control | Vello scene | wgpu/vello/vger | none meaningful | canvas | total |
| GPU backend | wgpu default | wgpu + tiny-skia fallback | Metal / **DX11** / wgpu(Linux) | **Vello (alpha, needs compute shaders)** | vger | WebView2 | Vello/Skia | D2D/D3D/DWrite |
| Text shaping | skrifa + vello_cpu + harfrust | cosmic-text 0.15 | **DirectWrite** on Windows | **parley** | cosmic-text | Chromium | parley | **DirectWrite** |
| Accessibility | **AccessKit default-on** | **No** | Yes | AccessKit | **No** | via Chromium | AccessKit | UIA, DIY |
| Ships in | Rerun | Kraken, Halloy, Sniffnet; COSMIC via **soft fork** | Zed; Longbridge Pro | nothing | Lapce only | thousands | nothing | — |

### egui

Maintainer's own verdict on this use case
([discussion #3350](https://github.com/emilk/egui/discussions/3350)):
*"egui is currently not well suited for large text documents (1MB+)."*
[PR #5411](https://github.com/emilk/egui/pull/5411) added paragraph-level caching
(2000-line edit: ~8 ms → ~300 µs) but **added no viewport culling** — `TextEdit`
still tessellates the whole document.

Worth knowing: **egui tried to adopt Parley and abandoned it.**
[PR #5784](https://github.com/emilk/egui/pull/5784) is a stale draft; the author
wrote that Parley *"expects to control layout within a rectangle, egui needs
finer control"* and went direct to skrifa/HarfRust instead. That is the most
relevant negative datapoint on Parley's API fit for anyone wanting fine layout
control.

Immediate mode is structurally wrong for "many files open at once" — every open
buffer's galley pipeline is touched per frame unless you gate rendering hard.

### iced

`Highlight` exposes **only `color()` and `font()`** — no per-run font size, so no
variable heading sizes. Dealbreaker for WYSIWYG.

Also: no release in ~7 months, still 0.x, README still says *"experimental
software."* The [FAQ](https://book.iced.rs/faq.html) is candid: *"a one-person
project… No timelines. No promises. No expectations,"* and *"I don't care about
wide adoption."* Its largest production consumer (System76 COSMIC) runs a
[soft fork](https://github.com/pop-os/libcosmic). Zero accessibility.

The `Shader` widget is genuinely the cleanest escape hatch of any toolkit here.

### GPUI — best-engineered, worst-governed

The technically strongest Windows Rust UI stack in existence. `crates/gpui_windows/src/`
is a real native backend: `direct_write.rs` (69 KB), `directx_renderer.rs` (69 KB),
`shaders.hlsl` (47 KB), `direct_manipulation.rs`, `destination_list.rs` (jump
lists). Zed ships official Windows builds; Windows became a *"fully supported
platform"* on 2025-10-15 with a full-time team. Blade was removed
([PR #46758](https://github.com/zed-industries/zed/pull/46758)) — *"blade is a
mess and causes several issues for both Zed users as well as other 3rd party apps
using GPUI."*

Four blockers:

1. **Live licence dispute.**
   [zed#55470](https://github.com/zed-industries/zed/issues/55470), opened
   2026-05-02, label `area:legal`, **no maintainer response**: a default release
   build statically links GPL-3.0-or-later object code via
   `gpui → sum_tree → ztracing → zlog`. Reporter argues any binary depending on
   gpui inherits GPL obligations, contradicting the Apache-2.0 label. Proposed
   fix (swap `ztracing` for `tracing`) not applied.
2. **Publicly deprioritized.** Zed leadership, Discord ~Feb 2026: *"GPUI
   development is getting some major brakes put on it. We gotta focus on some
   business relevant work in 2026."* Nate Butler (ex-Zed #1) started community
   fork `gpui-ce`; ~1 PR merged, stalled. His words: *"it is hard for Zed
   Industries to justify work on gpui that is purely for the community."*
   ([HN 47003569](https://news.ycombinator.com/item?id=47003569))
3. **crates.io release cannot satisfy its own README.** It tells you to use
   `gpui_platform::application()`; `gpui_platform` is not published. Neither is
   `gpui_windows` or `gpui_wgpu`. Only escapes: a git pin, or third-party
   `gpui-unofficial` (v1.11.3, ~2.1k downloads, not maintained by Zed).
4. **No rich-text editing widget.** Zed's editing lives in the separate `editor`,
   `text` and `ui_input` crates — which are **GPL-3.0**, not Apache.

The one real on-ramp if you do go this way:
[gpui-component](https://github.com/longbridge/gpui-component) v0.5.1,
Apache-2.0, 12.3k★ — 60+ widgets including a code editor good to ~200k lines with
LSP and tree-sitter highlighting, plus markdown rendering. Longbridge Pro (a
trading client) ships on it. That's the proof it's survivable.

### Xilem / Masonry / Linebender

**Take the libraries, not the framework.**

The libraries are excellent and ship monthly: parley 0.11.0, fontique 0.11.0,
swash 0.2.10, vello 0.9.0, accesskit 0.24.1. Parley has **1.14M recent
downloads** and is the ecosystem convergence point — Bevy migrated *to* it, plus
Slint, Blitz, Masonry, CuTTY, Gosub.

The UI layer has not shipped in ~9 months (xilem/masonry 0.4.0, 2025-10-29), and
released `masonry` 0.4.0 still depends on `parley ^0.6` and `vello ^0.6` — five
and three majors behind respectively.

There is **no public roadmap**
([xilem#1481](https://github.com/linebender/xilem/issues/1481), open since
2025-11-14). PoignardAzur, 2025-12-01: *"We currently have bandwidth issues, and
Xilem is the project that suffered the most from them."* Maintainer xStrom,
2026-01-19, asked how long until you can build apps:

> "Experimental apps can be built right now. **Serious business apps you'll need
> to wait a few years.**"

Governance: Linebender is *"a group of volunteers and enthusiasts"* — no legal
entity, no disclosed funding. Blog cadence dropped monthly → quarterly.

**Vello is the specific risk.** README: *"in an alpha state"*, and **compute
shader support is mandatory**. For Windows-first that means RDP sessions, VMs,
old integrated GPUs and locked-down driver estates are a live failure mode.
`vello_cpu` / `vello_hybrid` mitigations sit at **0.0.9**.

### Floem

Best architectural fit on paper (leptos-style fine-grained reactivity, Taffy
layout, muda native menus, and `floem::views::text_editor` is literally Lapce's
production editor core). But crates.io is stuck at **0.2.0 since Nov 2024** —
~20 months. [Issue #1050 "Plans for floem 0.3?"](https://github.com/lapce/floem/issues/1050)
opened March 2026, no visible maintainer response. Windows IME reportedly won't
start. No accessibility. Parent project Lapce is moribund.

### Tauri v2 — mature, but the premise doesn't survive its own data

Most mature and lowest-risk option here. 2.11.5, monthly releases, 8.57M recent
downloads.

Pulled the raw JSON behind Tauri's CI benchmark
([tauri-apps.github.io/benchmark_results](https://tauri-apps.github.io/benchmark_results/)),
run 2026-07-26. Methodology: hyperfine, 3 warm-ups + 10 runs, measured to
`DOMContentLoaded`, GitHub Actions runners.

| | hello_world exec | binary | max_memory |
|---|---|---|---|
| Tauri, windows-latest | **0.576 s** | 2.76 MB | **not measured on Windows** |
| Tauri, ubuntu-latest | 0.73–0.76 s | 2.97 MB | 271–439 MB |
| Electron (**last datapoint 2023-09-24**) | **0.475 s** | 166 MB | 476 MB |

Reading this honestly:

1. **Binary size is a genuine 60× win.**
2. **Startup is not a demonstrated win** in Tauri's own harness. The repo warns
   cross-OS results aren't directly comparable, so read it as "no evidence of a
   startup advantage" rather than "Tauri is slower." Either way the marketing
   claim isn't supported by the project's own data.
3. Memory ~1.4× better, not 10× — and there is **no Windows memory data at all**.
4. CPU-bound work does favour Tauri: `cpu_intensive` 4.30 s vs Electron 8.83 s.

Two further problems for this specific product:

- **[tauri#15436](https://github.com/tauri-apps/tauri/issues/15436)**, opened
  2026-05-24, still `needs triage`: **WebView2 IME/TSF freezes on first focus of
  inputs with existing text — CJK input blocked on Windows.** Directly on the
  critical path.
- **You cannot pin the engine.** Microsoft:
  *"Your app cannot require a particular version of the WebView2 Runtime."*
  Fixed Version is **over 250 MB** — worse than Electron.

And the "Rust core" is not meaningful for an editor's hot path. Tauri IPC is
*"a JSON-RPC like protocol under the hood… all arguments and return data must be
serializable to JSON."* Your document model, layout and text shaping all run in
JS/DOM inside WebView2. Typing latency would be a webview's typing latency —
~19.6–31.2 ms measured, versus 0.9–8.2 ms for native editors. That is the whole
thesis of the product, gone.

### Dioxus + Blitz — ruled out with numbers

Blitz publishes an honest status site. From
[blitz.is/status/events](https://blitz.is/status/events):

| Event class | Status |
|---|---|
| `compositionstart` / `update` / `end` | **All ❌ — no IME** |
| `cut` / `copy` / `paste` | **All ❌ — no clipboard** |
| `beforeinput` | ❌ |
| `select` (selection change) | ❌ |
| drag & drop (all 7) | ❌ |

**contenteditable does not appear anywhere** in the status matrices or roadmap.
[WPT css interop: 42.20%](https://blitz.is/status/wpt). `position:static` ❌,
`position:fixed` ❌, `position:sticky` ❌, `overflow:auto` ❌, `vertical-align` ❌,
`text-overflow` ❌. `<select>` ❌. The 1.0 roadmap still lists `position:static`.

README is explicit: **pre-alpha**, *"significant bugs and missing features
persist,"* actively discourages production use. Only **541 recent downloads**.
And only ~1.6% of Dioxus users are on the native renderer — the mainstream desktop
path is `dioxus-desktop` on WebView2, which puts you back in Tauri's world with a
smaller ecosystem.

### windows-rs + Direct2D/DirectWrite

`windows` 0.62.2, Microsoft-maintained, MIT/Apache, 277M lifetime downloads,
metadata-generated so Direct2D, DirectWrite, DirectComposition and TSF are all
bound.

What DirectWrite gives you free — and this is exactly the list every Rust toolkit
above is missing: shaping, bidi, line breaking, system font enumeration,
**font fallback that actually works for CJK**, `IDWriteTextLayout` with per-range
formatting, `IDWriteInlineObject` for inline images/widgets, hit-testing both
directions, and TSF-based IME.

Cost, stated honestly: `crates/gpui_windows/src/` is roughly **310 KB of Rust
plus 48 KB of HLSL** for the Windows platform layer alone — by a funded team,
and it still shipped with IME/keyboard-layout and screen-reader caveats. That is
your effort estimate. And cross-platform later means repeating a chunk of it.

Ergonomics: raw COM in `unsafe`. The official
[Direct2D sample](https://github.com/microsoft/windows-rs/blob/master/crates/samples/windows/direct2d/src/main.rs)
is **553 lines for an animated clock**.

---

## Recommendation

**No toolkit. winit + wgpu + parley-for-layout.**

```
Window/input/platform   winit 0.30 + muda (native menus) + rfd (dialogs)
Renderer                wgpu 30, hand-written quad + glyph pipeline
                        [NOT vello — alpha, mandatory compute shaders]
Text layout             parley 0.11 + harfrust + fontique + skrifa
Glyph raster            swash 0.2 (CPU atlas) or vello_cpu
Buffer                  ropey 1.6.1
Accessibility           accesskit
```

Rationale: since no toolkit provides a rich-text editing widget, a toolkit only
buys windowing, input routing and a paint API — which winit + wgpu provide
directly, without importing egui's per-frame relayout (wrong for many open
buffers), iced's one-person governance, Floem's 20-month release gap, GPUI's
licence dispute, or Xilem/Vello's alpha compute-shader dependency.

### Use parley for layout, not editing

Parley's *layout* side is production-grade. Its *editing* side is not, and its
own tracker says so:

- [parley#525 "Rich Text editor"](https://github.com/linebender/parley/issues/525),
  open since 2026-01-26: *"The current `PlainEditor` only support plain text."*
  Maintainer: *"No time estimate yet."*
- [parley#524 "Editor for large buffers of text"](https://github.com/linebender/parley/issues/524):
  *"`PlainEditor`… is backed by a single `String` which makes it very inefficient
  when editing large quantities of text. **This makes it unsuitable for use cases
  like editing a text file.**"*

These are not blockers — we were always writing the editor core. They just make
it explicit rather than optional.

The primitives we do need are all present:
[`Cursor::from_byte_index()`](https://docs.rs/parley/latest/parley/editing/struct.Cursor.html),
`Cursor::from_point()`, `Cursor::geometry()`, and `TreeBuilder::build()`
returning `(Layout, String)` so accumulated offsets can be tracked. That is
exactly the bidirectional mapping the WYSIWYG problem requires.

Pin exact versions and wrap it: **five breaking releases in seven months**
(0.7 Nov 2025 → 0.11 Jun 2026), no 1.0, no stability statement.

### Windows-first refinement

**Put the text layer behind a narrow internal trait and implement it twice —
DirectWrite now, parley later.**

This de-risks three things at once: parley's release churn,
[fontique#644](https://github.com/linebender/parley/issues/644) (CJK fallback
broken on Windows, open since 2026-06-21), and it gives a correctness oracle to
test parley against. GPUI's `gpui_windows` — DirectWrite + DirectX + HLSL — is
proof this is the right shape for a fast Windows editor.

### Rope choice

**ropey 1.6.1.** Deciding factors: O(1) structural-sharing clones (the whole
ballgame for many open files plus background parsing), UTF-16 indices
(non-optional if LSP or a plugin API ever appears — `crop` is byte-only), and
full Unicode line breaks behind `unicode_lines` (`crop` handles LF/CRLF only, and
Markdown pasted from Word contains U+2028).

Pin 1.6.1; don't chase the 2.0 beta that has been in beta ~21 months. Zed's rope
is off the table on licensing (GPL-3.0-or-later). `jumprope` is disqualified —
clone reallocates, no line API, last commit 2023.

### IME

winit is **IMM32-only** — verified from
[`winit-win32/src/ime.rs`](https://github.com/rust-windowing/winit/blob/master/winit-win32/src/ime.rs),
which references `ImmGetContext`, `ImmGetCompositionStringW`, `GCS_*`, `CFS_POINT`,
`ATTR_TARGET_*`, and **zero TSF interfaces**.

Cost: Win+H voice typing produces nothing —
[wezterm#7791](https://github.com/wezterm/wezterm/issues/7791) documents that
voice typing delivers text via `ITextStoreACP::InsertTextAtSelection` and the
IMM32↔TSF bridge does not bridge it. Also lost: reconversion, handwriting.

Ship IMM32 for v1 (it handles JP/CN/KR composition, and is what Alacritty,
WezTerm and egui all do), but **put the text buffer behind a trait that can later
back an `ITextStoreACP`** — retrofitting TSF's ACP offsets and lock protocol onto
a buffer not designed for it is expensive.

## Spike before writing anything else

Render a paragraph mixing Latin, CJK, Arabic and colour emoji, then type into it
with a Japanese IME. This tests fontique#644 and the IMM32 path in one go. If
fontique fails, `IDWriteFontFallback` is a direct substitute — which is another
argument for the DirectWrite-first path.

> **Result, MDEDIT-5 (2026-07-27): fontique fails. Confirmed on Windows, not
> inferred.** Probe at `spikes/font-fallback/`; full analysis on the Confluence
> page linked from MDEDIT-5.
>
> Two of eight samples produced `.notdef` glyphs. The cause is visible in the
> font fontique selected:
>
> ```
> TOFU Han (Simplified)   汉字编辑器
>        fonts: Arial, Yu Gothic UI
>        .notdef for: 汉, 编, 辑
> ```
>
> **Yu Gothic UI is a Japanese font.** fontique resolves `script = Hani` to one
> fallback family and stops. `汉`, `编` and `辑` exist only in Simplified Chinese
> (`漢`, `編`, `輯` traditional), so they fall through; `字` and `器` render
> because those forms are shared with Japanese.
>
> **The machine had the right font all along.** The coverage audit found `汉`
> covered by **10 of 232 installed families, including SimSun** — the default
> Simplified Chinese font on Windows. fontique never consulted it. This is a
> defect, not a missing-font artefact, and that distinction is what makes the
> result conclusive.
>
> Japanese, Korean, Arabic, Hebrew and emoji all pass, and that is the proof
> rather than a mitigation: fallback works, and breaks only where one Unicode
> script spans locales with disjoint glyph coverage. Hani is that case.
>
> **Consequence: DirectWrite-first is confirmed, at high confidence.**
> `IDWriteFontFallback` handles multi-font fallback natively because it is what
> Windows itself uses to render this text correctly everywhere else. parley is
> not rejected — its layout side stays the portable implementation behind the
> trait, which exists so this is revisitable when #644 closes.
>
> **The fix works, but has not landed.**
> [parley#689](https://github.com/linebender/parley/pull/689), *"Fontique: 3-tier
> font fallback"*, targets this directly. Notably its Windows tier delegates to
> `IDWriteFontFallback::MapCharacters` — the same API the DirectWrite-first plan
> calls directly, so the upstream fix and our workaround converge on the same
> mechanism rather than diverging.
>
> **Verified on Windows 2026-07-27:** with the PR branch patched in, the probe
> passes all eight samples, including the mixed-script line. Han now resolves to
> *two* families — Microsoft JhengHei UI alongside Yu Gothic UI — which is the
> multi-font fallback working as designed.
>
> But as of 2026-07-27 the PR is **open, unmerged, conflicted with main, and
> unapproved**, with one AI-generated commit and a supportive-but-cautious
> maintainer response questioning the char lookup and cache design. Nothing has
> shipped, so **the decision above stands unchanged** — there is no released
> fontique to depend on. What has changed is certainty about the endgame: the
> fix is known to work, so this is now a waiting problem rather than an open
> question. `spikes/font-fallback` carries the `[patch.crates-io]` block and the
> MSRV note needed to reproduce this.
>
> The IME half remains unrun: it needs a human at a Windows keyboard and a
> renderer to type into. Tracked as MDEDIT-6.

## Explicitly unverified

- **No independent benchmarks of Slint exist** for startup, binary size, memory
  or frame rate. The "runtime fits in less than 300KiB RAM" figure on slint.dev is
  vendor marketing in an MCU context — not a desktop number.
- **No published binary-size figure for any Rust GUI toolkit.** Three
  hello-worlds and a `dir` is a ten-minute experiment worth running.
- `gpui.rs` returned HTTP 403; current site claims unverified.
- No independent third-party Tauri-vs-Electron benchmarks found.
- No Windows/DX12-specific quality data for Vello.
- r/rust was unreachable, so developer sentiment is HN/Lobsters only.
