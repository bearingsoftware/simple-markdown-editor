# 01 — Editor landscape

What exists, what is dead, what users actually complain about, and what nobody
has built. Data verified 2026-07-26.

## The gap, stated once

Users have asked for the same five properties since **2014**: *fast, native
(non-Electron), open source, WYSIWYG, plain `.md` files on disk.*

Every existing option fails at least one:

| Editor | Fails on |
|---|---|
| Typora | closed source, Electron |
| Obsidian | closed source, Electron |
| Zettlr | Electron; currently also fails "fast" |
| MarkText | Electron; was dead until May 2026 |
| Logseq 2.0 | **just dropped plain markdown files** |
| Joplin | SQLite not files, slow, WYSIWYG second-class |
| Ghostwriter | native Qt, but split-pane, single-doc, Windows build 4 years stale |
| Notion | everything except WYSIWYG |

Demand proof: [Ferrite](https://github.com/OlaProeis/Ferrite) earned **241 HN
points** ([46571980](https://news.ycombinator.com/item?id=46571980)) for an
unfinished, admittedly 100%-AI-generated editor whose entire pitch was
*"~15MB vs ~300MB+ (no Electron) / Instant startup vs seconds."*

## MarkText

**It is not dead any more.** This premise needs correcting before any porting
discussion.

| Metric | Value |
|---|---|
| Stars | 59,164 |
| Latest stable | v0.19.1, 2026-06-06 |
| Latest release | v0.20.0-rc.1, 2026-07-05 (pre-release) |
| Licence | MIT |
| Domain | **marktext.me** (the old one lapsed to a malware squatter) |

Commit history tells the story:

| Period | Commits |
|---|---|
| 2023 | 6 |
| 2024 | 3 |
| 2025 | 1 |
| 2026-01 → 2026-05 | 0 |
| **2026-05 → 2026-08** | **464** |

Original author Jocs returned 2026-05-11 after leaving full-time employment
([#4191](https://github.com/marktext/marktext/issues/4191)). **Bus factor 1** —
418 of those 464 commits are his; next contributor has 10.

v0.20.0-rc.1 is a **ground-up editor engine rewrite**
([#4668](https://github.com/marktext/marktext/issues/4668)): *"the entire editor
engine has been rewritten from the ground up."*

### Do not port it

Stack is Electron 42 + Vue 3.5 + Pinia + TypeScript 6 + snabbdom vDOM +
`ot-json1` operational transform. Nothing transfers to Rust.

Two things are worth taking, though.

### Worth stealing #1 — the CSS marker collapse

From [Muya](https://github.com/marktext/muya), MarkText's editor core
(`assets/styles/inlineSyntax.css`):

```css
.mu-gray { color: var(--editor-color-30); text-decoration: none; }

.mu-hide, .mu-hide .mu-highlight, .mu-hide .mu-selection {
    display: inline-block;
    width: 0; height: 0;
    overflow: hidden;
    vertical-align: middle;
}
```

The `**` markers are **always present in the DOM text**, so text offsets stay
aligned with the markdown source. They are merely collapsed to a zero-size box
when the cursor isn't in them. This is the single cleverest trick in the codebase
and the reason cursor arithmetic stays sane.

The reveal predicate is one interval-overlap test
(`inlineRenderer/renderer/index.ts`):

```ts
checkConflicted(block, token, cursor = {}) {
    const anchor = cursor.anchor || cursor.start;
    const focus  = cursor.focus  || cursor.end;
    if (!anchor || !focus || (cursor.block && cursor.block !== block)) return false;
    const { start, end } = token.range;
    return conflict([start, end], [anchor.offset, anchor.offset])
        || conflict([start, end], [focus.offset, focus.offset]);
}

getClassName(outerClass, block, token, cursor) {
    return outerClass || (this.checkConflicted(block, token, cursor)
        ? CLASS_NAMES.MU_GRAY   // cursor inside this token -> show marker, dimmed
        : CLASS_NAMES.MU_HIDE); // cursor elsewhere -> collapse marker
}
```

That one function is the entire "WYSIWYG" illusion. CodeMirror 6 / Obsidian Live
Preview and vim's `conceal` do the identical thing by different means.

### Worth stealing #2 — syntax choices as block metadata

Muya's block model (`docs/jsonState.ts`) preserves the syntax decisions a naive
serializer normalizes away:

```js
{ name: 'atx-heading',    meta: { level: 1 },                              text: '# foo bar' }
{ name: 'setext-heading', meta: { level: 1, underline: '===' },            text: 'foo\nbar' }
{ name: 'code-block',     meta: { type: 'indented', lang: 'javascript' },  text: '...' }
{ name: 'bullet-list',    meta: { marker: '-', loose: false },             children: [...] }
{ name: 'order-list',     meta: { start: 1, loose: true, delimiter: '.' }, children: [...] }
```

Note inline content is stored as **unparsed source** (`text: 'foo\nbar'`), with
decoration applied at render time. Block structure gets a model; inline gets raw
text. That two-level split is the right shape — see [03](03-architecture.md).

### MarkText's two cautionary failures

**It rewrites your file.** [#2189](https://github.com/marktext/marktext/issues/2189),
48 reactions, open since 2020. Maintainers' own explanation:

> "Mark Text formats all content according CommonMark and GFM... The reason for
> this is that we don't store the document as string with spaces etc but as block
> structure with minimal information. As a side effect, we need to build the text
> file when saving the document."

> "It's currently not planned to change the markdown formatting as it's built-in
> into the editor."

Open a file, save it, get a noisy diff. For a Markdown tool that is the cardinal
sin. This is the fundamental tax of AST-backed WYSIWYG.

**The engine rewrite did not fix large files.**
[#4887](https://github.com/marktext/marktext/issues/4887), filed 2026-07-07
against v0.20.0-rc.1, Windows 11: a ~300,000-word file freezes the app
*"totally unresponsive"* for 4–5 minutes, requiring force-quit. Reporter notes
the same file opens in Obsidian within seconds. Cause: the whole document is
materialized into DOM, no virtualization.

## Comparables

| Editor | Licence | Stack | Model | Native | Tabs | Status |
|---|---|---|---|---|---|---|
| **Typora** | Proprietary, $14.99 | Electron + CodeMirror for fences | **Hybrid inline — the archetype** | No | **None** — 1 doc = 1 window | Active |
| **Obsidian** | Proprietary, free incl. commercial since 2025-02 | Electron + CodeMirror 6 | Hybrid (Live Preview) | No | Tabs, groups, splits, stacking | Active |
| **Zettlr** | GPL-3.0 | Electron + Vue 3 + CM6 | Hybrid, per-element configurable | No | Tab bar + workspace | Active (13.3k★) |
| **MarkText** | MIT | Electron 42 + Vue 3.5 + Muya | Hybrid inline | No | Scrolling strip | Revived |
| **Logseq** | AGPL-3.0 | ClojureScript + Electron; SQLite in 2.0 | Block outliner | No | No doc tabs | **Breaking pivot** |
| **Joplin** | AGPL-3.0 | Electron + React + CodeMirror | Split-preview default | No | **No tabs** | Active |
| **Ghostwriter** | GPL-3.0+ | **Qt / C++** (KDE), cmark-gfm | Split-pane preview | **Yes** | **No tabs** | Active, Windows stale |
| **Apostrophe** | GPL-3.0+ | Python + GTK4 | Source + preview | **Yes** | Single doc | **Linux only** |
| **SiYuan** | AGPL-3.0 | Electron + Go kernel | Block WYSIWYG, `.sy` JSON files | No | Tabs + tree | Very active |
| **AppFlowy** | AGPL-3.0 | **Flutter + Rust** | Block WYSIWYG | Flutter | Sidebar | Very active (74.3k★) |

Traps in that table:

- **Ghostwriter's GitHub `releases/latest` returns 2.1.6 from 2022** — it moved to
  the KDE Gear cycle and is at 26.04.3. But Windows genuinely is second-class:
  portable build only, and the winget manifest still ships v2.1.6.
- **The Notion-like tier stores something other than markdown files.** SiYuan
  writes JSON AST to `.sy`; Trilium's native format is HTML; AppFlowy, AFFiNE and
  Anytype are block databases.
- **Anytype is not open source** despite the marketing — "Any Source Available
  License 1.0" restricts commercial use.

### Dead or dying

**Dead:** Abricotine (archived, README says discontinued), Notable (last release
2020, 731 open issues, licence removed, site still up), vdmark, Lapce
(moribund — last real commit 2026-04-03, not archived), all three MarkText forks
(each now behind upstream).

**Life support:** PanWriter, Apostrophe (10 months since release, Linux-only),
Ferrite (7 weeks idle, AI-generated codebase), Helix *releases* (25.07.1 from
2025-07-18 — over a year, though commits are active).

## The Typora model, precisely

From the [official Quick Start](https://support.typora.io/Quick-Start/), verbatim
— this is the whole contract in two sentences:

> "Typora uses the feature: **Live Preview**, meaning that you can see inline
> styles as soon as you finish typing them and see block styles as you type or
> after you press the Enter key to focus on the next paragraph."

> "Markdown tags for **inline styles**, such as `**` will be hidden or displayed
> smartly. Markdown tags for **block level styles**, such as `###` or `- [x]`
> will be hidden once the block is rendered."

Decomposed into what actually matters:

1. **Two different timing rules, not one.** *Inline* syntax resolves **as soon as
   the construct completes** — type the closing `**`, markers vanish immediately.
   *Block* syntax resolves **on block exit**, normally on Enter. Conflating these
   is the most common implementation mistake.
2. **Reveal is cursor-proximity, per-token.** Move the caret back into a rendered
   inline construct and its markers reappear.
3. **One surface.** No split pane, no mode toggle in normal use. A source-code
   toggle exists but the docs say *"only very basic support is provided... not
   recommended."* It is an escape hatch, not a mode.
4. **Focus mode is separate and orthogonal** — it blurs non-current lines. Don't
   conflate it with reveal behaviour.
5. **Code fences are the documented exception** — CodeMirror, not the live-preview
   surface.
6. **Known failure modes are all round-trip and edge-case.** Per
   [typora-issues#1317](https://github.com/typora/typora-issues/issues/1317)
   (open since 2018): footnote definitions, HTML anchors and emoji shortcodes have
   no good rendered representation, so they show raw markup permanently and break
   the illusion.

Also worth knowing: **Typora has a hard 2 MB render ceiling** —
`MAX_FILE_SIZE = 2e6`, producing "The file is too large to render in Typora."

## Handling hundreds of open files

Short answer: **Obsidian and Zed. Nothing in the dedicated-markdown category.**

**Obsidian is the only one with real lazy loading.** As of v1.7.2, per the
[Defer views guide](https://github.com/obsidianmd/obsidian-developer-docs/blob/main/en/Plugins/Guides/Defer%20views.md):

> "As of Obsidian v1.7.2, When Obsidian loads, **all views are created as
> instances of `DeferredView`**. Once a view is visible on screen... the `leaf`
> will rerender and the view will be switched out to the correct `View` instance."

300 restored tabs cost 300 cheap placeholder objects, not 300 rendered documents.
It was breaking enough that plugin authors were told to add `instanceof` checks.

Everyone else:

- **Typora**: no tabs at all. Structurally incapable.
- **MarkText**: [#2590](https://github.com/marktext/marktext/issues/2590) — tabs
  went *literally invisible* past ~13 for five years. The 2026 fix only scrolls
  the active tab into view. No overflow menu, no tab search.
- **Joplin, Ghostwriter, Apostrophe, Logseq**: no document tabs.
- **Zettlr, SiYuan, Trilium**: have tab bars; no evidence any lazy-loads.

**The pattern to steal: deferred view materialization** (a tab is a lightweight
handle until visible) plus **session restore that persists handles, not content.**
Obsidian is the only editor in this survey that implements it, and it took them
until 2024.

## User complaints, by editor

Sources are Hacker News, GitHub, and project Discourse forums. **Reddit was
unreachable** — treat this as an incomplete sample skewed toward developers.

### Typora

- **DRM, not price, was the backlash.** josephcsible,
  [29361226](https://news.ycombinator.com/item?id=29361226): *"For all the people
  saying you'd be happy to pay for this given its value to you: you're missing the
  point. The DRM is the problem, and paid software is perfectly capable of
  existing without DRM."*
- **Opaque lineage, still live in 2026.** danfoxley,
  [46578012](https://news.ycombinator.com/item?id=46578012): *"I like the editor,
  but Typora's lineage is opaque, which worries me."*
- **Live preview gets slow with math.** zoomablemind,
  [29362667](https://news.ycombinator.com/item?id=29362667): *"with pages and
  pages of formulas, the rendering gets too sluggish which also impares the
  editing. So I had to disable the rendering just to edit some formulas."*
- **No tabs**, requested for years — issues
  [#6569](https://github.com/typora/typora-issues/issues/6569),
  [#2757](https://github.com/typora/typora-issues/issues/2757),
  [#638](https://github.com/typora/typora-issues/issues/638),
  [#296](https://github.com/typora/typora-issues/issues/296), all open.

### Obsidian

- **"I didn't realize it wasn't open source"** is a recurring surprise.
  himata4113, [48180362](https://news.ycombinator.com/item?id=48180362):
  *"This made me realize that obsidian is not opensource, but in a way obsidian
  made me feel like it was opensource."*
- **Cold start is the single most-cited pain.** A forum user posted their own
  breakdown on a 43,049-file vault, **restricted mode on, default theme, all
  community plugins disabled**
  ([thread](https://forum.obsidian.md/t/android-large-vault-slow-to-load-on-with-all-plugins-disabled/90828)):
  `Total startup time: 367,373ms / Vault (43,049 files): 17,197ms / Core plugins: 349,628ms`
  — a **6-minute cold start**.
- **Typing lag persists into 2026 with everything disabled** —
  [115274](https://forum.obsidian.md/t/typing-continues-to-lag/115274) (2026-06-13),
  [111191](https://forum.obsidian.md/t/tracking-down-typing-lag-on-mobile/111191)
  (2026-02-13): *"even after disabling all plugins, themes, snippets, whatever…
  I still have lag."*
- **Hits Electron's 4 GB V8 heap cage** on large vaults
  ([thread](https://forum.obsidian.md/t/reducing-the-memory-footprint-of-obsidian/102332)).
- **Counter-signal, important:** dboon,
  [46574879](https://news.ycombinator.com/item?id=46574879): *"Any tool worth
  using in this space... is fast. Obsidian is fast. Zed is fast. It's table
  stakes."* Obsidian's perf reputation is bimodal — small vaults are fine and it
  beats its FOSS competition. Pain concentrates in cold start, mobile, and 10k+
  file vaults.

### Zettlr — a clear, recent regression

- [#6188](https://github.com/Zettlr/Zettlr/issues/6188) (2026-02-17): *"now it
  takes more than 30 seconds (or more) to write one single letter."*
- [#5985](https://github.com/Zettlr/Zettlr/issues/5985), with a proper bisect:
  *"From 3.3.1 to 3.4.1, everything works fine. With 3.4.2, it begins to slow...
  With 3.4.6, I can't work normally... Libreoffice is faster than Zettlr."*
- Despite this it is currently **the most-recommended FOSS alternative on HN**.

### Logseq — the freshest cautionary tale

2.0.1 beta shipped 2026-07-13. The DB version **drops markdown files as the
source of truth**. Markdown graphs do not auto-migrate; the formats are not
interoperable.

Top comment on the HN thread — Valodim,
[48896645](https://news.ycombinator.com/item?id=48896645):

> "It pains me to say, but it's probably too little, too late. Logseq remained a
> buggy mess, is now on an unmaintained (thus insecure) version of electron. And
> now after several years of complete stagnation, the supposed improvement is a
> database format to fix their technical issues, **so I can no longer keep all my
> data as markdown files?** At a time when half the edits are done by Claude and
> tracked with jujustu, this is just not useful for me."

Sync has been in beta since 2022. gareim,
[48899393](https://news.ycombinator.com/item?id=48899393): *"I donated $15/month
for from mid-2022 to early-2023 to try Logseq Sync when it was in beta... It's
still in beta. What a joke."*

### Joplin

- Startup: a **2026-07-05** thread titled *"Extremely slow startup time on
  Windows, feels like loading a heavy game"*. A *satisfied* user in the same
  thread normalizing it: *"Joplin takes about 10 seconds to come up. It's been
  that way since day one."*
- **~1.5 GB idle** measured in 2025 —
  [#12354](https://github.com/laurent22/joplin/issues/12354), **closed as not
  planned**. Maintainer position is that this is Electron's floor.
- Notes live in SQLite with UUIDs, not files.
- The WYSIWYG request thread has been **open since 2019, 86 posts**.

## The 2026 twist: coding agents re-energised "files on disk"

This requirement did not exist in 2021 and is now decisive. It is currently
killing Logseq's migration.

- freedomben, [48897091](https://news.ycombinator.com/item?id=48897091):
  *"Not being able to use Claude or codex anymore to write or update pages is a
  real deal breaker for me."*
- Groxx, [48897020](https://news.ycombinator.com/item?id=48897020):
  *"Ain't no way I'm giving up my dumb file syncing that works for decades."*
- chrisweekly, [48901325](https://news.ycombinator.com/item?id=48901325):
  *"I'll stick with my portable local-filesystem markdown over any db format."*

## Table stakes before anyone would switch

Named as blockers in the Ferrite thread and equivalents:

- `[[wikilinks]]` + backlinks
- LaTeX / math
- Real WYSIWYG (not split-pane)
- Image paste with auto asset folder
- Cross-document querying (Dataview equivalent)
- Tabs
- Not-mac-only

## Rust-native competition

**Velotype** — [manyougz/velotype](https://github.com/manyougz/velotype), 490★,
Apache-2.0, created 2026-05-09, v0.7.0 on 2026-07-14. The only serious one.
Built on GPUI 0.2 plus [gpui-component](https://github.com/longbridge/gpui-component).
Genuinely WYSIWYG. **Block-tree runtime model**, parsed on import, serialized back
to canonical markdown on save with raw-source fallback — so it carries the same
round-trip tax as MarkText. ~37k lines of Rust; the author hand-implemented IME
because GPUI 0.2 lacks wrappers. Ships Windows x86_64 + aarch64.

Its key architectural decision — **abandon the one-text-buffer model for a block
tree where each block is its own widget** — is almost certainly right, and is the
same conclusion Muya reached in the DOM.

**Ferrite** — 1.7k★, MIT, egui. Despite the `wysiwyg` topic it is split view plus
a click-to-edit rendered mode, not a Typora-style single surface. Two red flags:
7 weeks idle after gaining 1.7k stars in 6 months, and its HN thread surfaced that
it is **100% AI-generated code**, which the author confirmed.

**Zed** — v1.0.0 shipped 2026-04-29; Windows is no longer lagging. Renders bold
as bold and italic as italic via tree-sitter, but no heading sizes, no hidden
markup, and a separate preview pane.
[#21717 "WYSIWYG Markdown Preview"](https://github.com/zed-industries/zed/issues/21717)
has been open since 2024-12-08 — **19 months, unassigned, 23 comments, not one
response from a Zed team member.** Do not expect Zed to fill this gap.

## Sources

Where a claim is contested or unverified it is marked inline above. Principal
sources: GitHub and crates.io APIs (live reads 2026-07-26), Hacker News Algolia
API, project Discourse forums, and direct source reads of Muya, MarkText, and
Notepad++.

Explicitly unverified: MarkText startup time, memory per window, and file-count
limits — no benchmark exists for this app category; treat any quoted number as
unsourced. Typora and iA Writer are closed source and no architecture details
were found.
