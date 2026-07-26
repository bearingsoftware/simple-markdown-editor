# Research: native WYSIWYG Markdown editor

Research conducted 2026-07-26. All findings verified against primary sources
(vendor docs, GitHub issue trackers, crates.io/GitHub APIs) at that date.
Every substantive claim carries a URL in the detail documents.

## Goal being evaluated

A Markdown editor that is:

- **WYSIWYG** — Typora-style hybrid inline rendering, one editing surface
- **Ridiculously fast** — native, no webview, sub-frame typing latency
- **Windows first**, portable later
- **Many files open at once** — hundreds of tabs, Notepad++-grade session handling
- Plain `.md` files on disk, byte-preserving

## Verdicts

| Question | Answer |
|---|---|
| Is Slint a good option? | **No.** Disqualified on evidence. See [02](02-gui-toolkits.md). |
| Is Rust a good option? | **Yes**, but no toolkit gives you a rich-text editing widget. You write the text view regardless. |
| Should we port MarkText? | **No.** Electron/Vue/TypeScript — nothing transfers. Steal one architectural idea. See [01](01-editor-landscape.md). |
| Is the market gap real? | **Yes**, and unfilled since ~2014. See [01](01-editor-landscape.md). |

## Recommended stack

```
Window/input      winit 0.30 + muda (native menus) + rfd (dialogs)
Renderer          wgpu 30, hand-written quad + glyph pipeline    [NOT vello]
Text layout       parley 0.11 + harfrust + fontique + skrifa     [layout only, not editing]
Buffer            ropey 1.6.1
Markdown parse    pulldown-cmark 0.13, full re-parse per edit
Code fences       syntect 5.3 + two-face, lazy, per-block cached
Accessibility     accesskit
```

Windows-first refinement: put the text layer behind a narrow internal trait and
implement it twice — DirectWrite now, parley later. This de-risks parley's
release churn and its open Windows CJK-fallback bug, and gives a correctness
oracle to test parley against.

## Documents

| Doc | Contents |
|---|---|
| [01 — Editor landscape](01-editor-landscape.md) | Existing editors, MarkText status, user demand signal, the unfilled gap |
| [02 — GUI toolkits](02-gui-toolkits.md) | Slint verdict with evidence; egui/iced/GPUI/Xilem/Floem/Tauri/Dioxus/windows-rs comparison |
| [03 — Architecture](03-architecture.md) | The core decision: decorate-the-source vs document model. Marker locator. Block widgets. |
| [04 — Parsing benchmarks](04-parsing-benchmarks.md) | Measured: pulldown-cmark vs tree-sitter-md incremental. Why we skip incremental parsing. |
| [05 — Windows platform](05-windows-platform.md) | Notepad++ analysis, shell integration, code signing, distribution |
| [06 — Performance targets](06-performance-targets.md) | Measured competitor baselines and the numbers to hit |

## The three riskiest parts

1. **The marker locator** — no Rust Markdown parser emits delimiter nodes, so the
   primitive that makes CodeMirror 6 / Obsidian / Muya work does not exist in this
   ecosystem and must be written. ~200–400 lines, no reference implementation.
   Build it first, standalone. [03](03-architecture.md)
2. **Windows text correctness** — font fallback and IME. `fontique#644` (CJK
   fallback broken on Windows) is open. winit is IMM32-only with no TSF.
   Spike before writing anything else. [02](02-gui-toolkits.md)
3. **Tables and code fences** — the specific place pure decoration falls over.
   Obsidian's forums are a five-year record of exactly this failure.
   [03](03-architecture.md)

## Method and limitations

Research ran as parallel agents over web search, vendor documentation, GitHub
and crates.io APIs, and direct source reads. Notable constraints:

- Reddit was unreachable from the research environment — **zero Reddit data**.
  Sentiment sampling is Hacker News, GitHub, and project Discourse forums only.
- Web search budgets were exhausted mid-research on several threads; later
  verification used direct fetches and APIs, which is why some claims cite exact
  source lines and manifest contents rather than search summaries.
- Benchmarks in [04](04-parsing-benchmarks.md) were **measured by us** because no
  public data existed. Single machine (WSL2), synthetic documents, wall-clock
  timing without statistical rigour. Magnitudes are 50–360×, so noise does not
  threaten the conclusions, but treat exact figures as indicative.
- Items that could not be verified are flagged inline in each document.
