# simple-markdown-editor — agent & contributor guide

Native WYSIWYG Markdown editor in Rust. Windows first, portable later.

This project follows the Bearing Software baseline at
[`bearingsoftware/code-standards`](https://github.com/bearingsoftware/code-standards), pinned to
**`v1.2.0`**. Read the baseline first — it owns the generic rules (git/PR workflow, work tracking,
error handling, testing, security, observability). This file holds **only** project specifics.
Don't duplicate baseline rules here.

## Baseline (read these)

- Cross-cutting: `code-standards/standards/_cross-cutting.md`
- Architecture: `code-standards/standards/architecture.md`
- Rust: `code-standards/standards/rust.md` · review: `code-standards/review/rust.md`
- Reviewing a PR: `code-standards/review/_template.md`

Rust landed in the baseline at `v1.2.0` ([#20](https://github.com/bearingsoftware/code-standards/pull/20),
[#21](https://github.com/bearingsoftware/code-standards/pull/21), MDEDIT-1). Tool configs are vendored
from that tag — see [Configs](#configs).

## Work tracking

- **Jira project:** `MDEDIT` — https://chattersley.atlassian.net/browse/MDEDIT
- **Confluence space:** `MDEDIT` — https://chattersley.atlassian.net/wiki/spaces/MDEDIT/overview
  (`Features` and `Bugs` parent pages)

Commit scope is the ticket key: `feat(MDEDIT-12): add marker locator`. Statuses are
`To Do → In Progress → In Review → Done` (this project's board adds `In Review` over the baseline
flow).

## Repo state

**A working editor on Windows.** It opens, edits and saves files without changing a byte nobody
touched; it has a window, a renderer, menus, tabs, a file pane, a scrollbar, a status bar, undo,
inline formatting, crash recovery and a settings file. It finds, replaces and goes to a line. It does
not run at all off Windows — there is no portable text backend, which is the first known gap below.

The crates below the app are still headless and platform-agnostic, and that is the line worth
keeping: everything that can be tested without a window is.

```
crates/offsets/         # mdedit-offsets: ByteOffset, ByteSpan, CharOffset, LineIndex, LayoutPos,
                        #   BlockId
crates/language/        # mdedit-language: what the editor needs from a language — spans with a
                        #   role, block extents, whether syntax is ever concealed, and what is wrong
                        #   with it. Depends on `mdedit-offsets` and nothing else, deliberately
crates/json/            # mdedit-json: an error-tolerant, span-producing JSON lexer. No serde —
                        #   a value has no spans and a strict parser is useless mid-edit. Plus the
                        #   strict check that is a separate pass for exactly that reason
crates/yaml/            # mdedit-yaml: the same for YAML, hand-written for evidenced reasons —
                        #   read its crate docs before proposing a parser crate
crates/markers/         # mdedit-markers: Markdown's implementation of that trait. Partitions
                        #   source into marker and content spans, and reports block extents — one
                        #   parse for both
crates/document/        # mdedit-document: rope buffer keeping that partition current, plus
                        #   selection, the reveal-on-cursor predicate, the heading trail to a
                        #   position, word and line extents, the undo transaction and the
                        #   many-site replace built on it, and the inline formatting toggles that
                        #   use the marker spans rather than guessing
crates/text/            # mdedit-text: the text-layout trait. No platform types cross it.
crates/text-dwrite/     # mdedit-text-dwrite: DirectWrite backend. Windows-only; one of the three
                        #   places `unsafe` is permitted. Compiles to nothing elsewhere.
crates/menu-win/        # mdedit-menu-win: the native menu bar, the one unsafe call that attaches it,
                        #   and the mode its bar and popups are drawn in — a parameter since
                        #   MDEDIT-79, not a hardcoded dark. Plus the small Win32 reads that are not
                        #   worth a crate of their own: the double-click interval, the parent
                        #   console, the window icon, and `AppsUseLightTheme`. Windows-only.
crates/shell-win/       # mdedit-shell-win: the DropTarget verb and its COM local server, so a whole
                        #   Explorer selection reaches one process. The registry footprint is a
                        #   value, which is what makes unregister exact. Windows-only
crates/file/            # mdedit-file: reads and writes files without changing an unedited byte,
                        #   and the mtime+length stamp that says when something else has
crates/session/         # mdedit-session: tabs as handles, the session file that restores them, the
                        #   crash-recovery snapshots, and the recent-files list — the last two
                        #   behind the same path boundary as the session file
crates/app/             # mdedit-app: the window. winit + wgpu + glyph atlas, the decoration
                        #   step, per-block layout and its cache, the settings file, the status
                        #   bar, the tab strip and file pane, click counting, the generated
                        #   shortcuts reference, the single-line text field and the find, replace
                        #   and go-to-line bar built on it, and the keystroke latency harness.
                        #   Binary: `mdedit`.
assets/icons/           # MDEDIT-70: the app icon, 24 PNGs and the `mdedit.ico` built from eight of
                        #   them. `build-ico.ps1` rebuilds it without ImageMagick. Read its README
                        #   before adding a size — there is no vector master, deliberately
spikes/font-fallback/   # MDEDIT-5 probe; outside the workspace, Windows-only, throwaway
spikes/gpu-baseline/    # MDEDIT-19 probe: what an empty wgpu window costs in memory. Re-runnable
spikes/startup-attribution/
                        # MDEDIT-20 probe: where the ~250 ms before first paint goes. Re-runnable,
                        #   and worth re-running whenever wgpu is bumped
spikes/clipboard-threads/
                        # MDEDIT-67 probe: reading the Windows clipboard from two threads of one
                        #   process corrupts the heap. Re-run when arboard is bumped
docs/research/          # 7 briefs; read README.md first
  README.md             # verdicts, recommended stack, top 3 risks
  01-editor-landscape.md
  02-gui-toolkits.md    # why not Slint; toolkit comparison
  03-architecture.md    # THE design decision + marker locator spec
  04-parsing-benchmarks.md
  05-windows-platform.md
  06-performance-targets.md
```

Read `docs/research/README.md` before proposing architecture — the major decisions are already made
and evidenced. Don't relitigate them without new evidence.

Known gaps, so nobody assumes they are done:

- **The executable has an icon, and embedding the resource was only half of it.** MDEDIT-70. Eight
  hand-drawn PNGs become `assets/icons/mdedit.ico`, `crates/app/mdedit.rc` declares it as resource
  `1`, and `build.rs` compiles that with `embed-resource`. That is what Explorer, the taskbar's
  pinned entry and `mdedit-shell-win`'s `DefaultIcon = "<exe>",0` draw — index 0 there means "the
  first icon group", so **registration has been pointing at a resource that did not exist** since
  MDEDIT-61.

  **It does not give the window one.** A window takes its icon from its class or from `WM_SETICON`,
  and winit sets neither, so the title bar and Alt-Tab drew a system fallback while a perfectly good
  icon sat inside the binary. `mdedit_menu_win::set_window_icon` is the fix: `LoadImageW` against the
  already-embedded resource, so there is no second copy of the pixels and no PNG decoder at runtime.
  It is called **before** `attach_menu` and not inside it, because that function gives up when muda
  cannot build a bar and a window with no menu should still have an icon. **Assumed, then disproved
  by looking** — `WM_GETICON` returned null for all three sizes and `GCLP_HICON` was zero. No test
  could have caught it; CI does not open a window.

  **Two resource crates, and neither does the other's job.** `embed-manifest` is pure Rust and keeps
  the Per-Monitor DPI v2 declaration free of any toolchain requirement; `embed-resource` needs
  `rc.exe`, which is not an extra requirement because it ships in the same Windows SDK as the
  `link.exe` any MSVC build already needs. They do not collide: on MSVC `embed-manifest` passes
  linker options rather than contributing a resource. Moving the manifest into the `.rc` would work
  and is the wrong trade.

  **ImageMagick is not a dependency and is not on the runners**, so `assets/icons/build-ico.ps1`
  builds the `.ico` from the PNGs with nothing but Windows, and the result is committed. Sizes below
  256 are BMP DIBs and 256 is a PNG. `System.Drawing.Icon` **cannot read the 256 entry** — it
  predates PNG-in-ICO — so a verification script that asks it for 256 gets 128 back and is reporting
  its own limitation. `.gitattributes` marks `*.png` and `*.ico` binary outright rather than trusting
  `text=auto`, because an icon whose `0x0d` bytes were rewritten on checkout would still build.
- **No portable text implementation.** `mdedit-text-dwrite` is the only backend, so the trait has
  one implementation and therefore no correctness oracle yet. parley fills that in when
  [parley#689](https://github.com/linebender/parley/pull/689) ships released, and until then the
  app does not run at all off Windows.
- **IME composition is implemented but unverified at a real keyboard.** The pre-edit is drawn at the
  caret, the candidate window is told where the caret is, a commit is one edit and a cancel is none
  — and all of that is unit-tested. What is *not* confirmed is what it looks like and where the
  candidate list actually lands, because composition cannot be driven programmatically: with
  Microsoft IME (Japanese) installed and the layout active, injected input leaves the IME closed
  (`IMC_GETOPENSTATUS` stays 0) and no `WindowEvent::Ime` ever arrives. MDEDIT-6 carries the
  seven-step script for a human.
- **AltGr is fixed but unconfirmed at a real keyboard**, and belongs in the same manual session as
  MDEDIT-6's IME script. Windows reports AltGr as Ctrl+Alt, and the chord gate read Ctrl alone — so
  `@` (AltGr+Q), `{[]}` (AltGr+7/8/9/0) and `€` (AltGr+E) went into the chord dispatch, matched no
  arm, and were dropped: a German-layout user could not type a `@` at all. MDEDIT-102 made the gate
  Ctrl **without** Alt, which is the conventional rule and the one every other editor uses. What no
  test here can confirm is the premise — that AltGr arrives that way — because a winit `KeyEvent`
  cannot be constructed. **The check:** install a German layout, activate it, and type `@`, `{`, `}`
  and `€` into the document and into the find box.
- **The pre-edit does not reflow the line it is on.** It is drawn over the following text on its own
  background rather than pushing it along, because it is deliberately not in the buffer and so
  cannot take part in layout. Fine at the end of a line, which is where composition usually happens;
  visibly an overlay in the middle of one.
- **Errors go to a log file, not to a console.** The binary is windows-subsystem, so an Explorer
  launch no longer flashes one — and `main` calls `AttachConsole(ATTACH_PARENT_PROCESS)` so a
  terminal launch still gets its output, including the latency harness line. Soft errors and panics
  land in `mdedit.log` beside the session file, capped at 1 MB with one previous generation. Messages
  and paths only: **never document bytes**, because the log is the thing a user attaches to a bug
  report. The Windows Event Log is deliberately not used — a services convention needing a registered
  message source, read by administrators rather than by the person whose editor misbehaved.
- **All chrome draws after the document.** There is no depth buffer and no scissor, so the painter's
  order is the whole of the answer: the formatting bar, tab strip, file pane and status bar are
  appended after `draw_chunks`. The row cull deliberately keeps up to a line-height of overshoot above the viewport
  — it is what makes a partly-scrolled row look cut off rather than popping in — so a piece of chrome
  drawn first is painted over by it. MDEDIT-39 was exactly that, and `frame.rs` guards it.
- **There is a File, Edit, View and Help menu, a status bar, a scrollbar, a right-click context menu,
  a formatting bar, and the open files shown either as a tab strip or as a left pane.** Every view
  toggle now has a menu item as well as a chord.
- **The formatting bar is a second door, never a second implementation.** MDEDIT-52: every button
  calls the editor method its chord calls, through one `Editor::perform` match, so there is no
  behaviour that exists only when clicked. It is per-run — View → Formatting Bar, no chord, and
  nothing written back to the config. **Its buttons are text**, drawn by the same engine as
  everything else and laid out once into `toolbar_labels`, which is therefore a fifth cache
  `set_zoom` has to clear. No tooltips (there is no hover affordance to hang one on) and no
  accessible names (those wait for the accesskit pass). Lighting is lookups only: `is_inside` asks
  the segments, `carries` and `is_quoted` read the caret's line out of the rope, and headings ask the
  block list so a setext one still lights. Rule and Link never light, because neither is a state.

  **Every document gets a standard set, and that is what stopped the bar disappearing.** MDEDIT-84.
  `toolbar::STANDARD` is New, Open, Save, Undo, Redo, Cut, Copy and Paste — eight buttons a language
  appends its own after, so JSON and YAML have a bar where they used to have none. It used to be
  Markdown's alone, on the rule that a button which does nothing is worse than no button; that rule
  is intact and the *bar* is no longer the thing it applies to. Close is deliberately not in the set:
  a one-click Close beside Save is a misclick that shuts a document, and it is on the menu, on Ctrl+W
  and on each tab's `×`.

  **The standard set is leftmost, so overflow cuts the language's buttons first.** The standard
  actions are what somebody who does not know the chords is looking for, and every formatting button
  has a chord *and* a menu item. Words rather than symbols, because there is no glyph for Open that
  reads without a legend — MDEDIT-92 makes both sets icons, after MDEDIT-85's tooltips.

  **A button that cannot act right now is dimmed and ignores its click**, in `theme.gutter` over the
  bar's `strip`, asserted legible by `contrast.rs`. Save with nothing to save, Undo with no history,
  Cut and Copy with no selection. **Paste is never dimmed, against the ticket's plan**: deciding needs
  the system clipboard, and reading it per frame is what `crate::clipboard` says not to do while
  caching it means a stale dim costs the user a paste — where a live Paste that inserts nothing is
  exactly what Ctrl+V already is. The dimmed check is in `click_toolbar` rather than in `perform`,
  because `perform` is what a chord reaches too and Ctrl+S on an unmodified document is allowed.
  **Geometry does not change with state** — a dimmed button keeps its rect, or the row would reflow
  as the user selects and deselects.

  **It is the topmost chrome, above the tab strip.** MDEDIT-84's second item, and it is four y
  offsets and two hit tests in `editor.rs` — `tabs.rs` and `toolbar.rs` are untouched, because each
  lays out within a row whose origin the caller supplies. The document's top edge does not move:
  `Insets::top` is a sum and a sum does not care about order. The find bar, status bar and file pane
  keep their places.

  **The strip's conversion is `strip_hit`, and it exists because there are two callers.** A left
  click and a middle-click-to-close both turn a window position into a tab, and the first version of
  this moved only the left one — the same mistake `pane_hit` was written to prevent, made again one
  function away from it. **Both bands are thirty pixels of `strip`**, so a test cannot tell the two
  backgrounds apart; `the_bar_is_drawn_above_the_strip` reads what is unique to each instead — an
  inactive tab's quad, and a dimmed label's `gutter`. Without that, moving the drawing and not the
  clicks (or the reverse) passes every other test in the file, and so does leaving the tab *labels*
  at their old baseline while their backgrounds move.
- **There is a find bar, and the text field is the part that did not exist before.** MDEDIT-44. Until
  it, the document was the only thing in this editor that took a keystroke; `crates/app/src/field.rs`
  is the other one, and Replace, Go to Line and every future dialog-ish thing are meant to reuse it
  rather than grow a second half-copy. It is a `String` and two byte offsets — no rope, no undo
  history, and one line, so a paste of several arrives flattened. **It takes committed text only:**
  `WindowEvent::Ime` still goes to the document, so composing CJK into the search box does not work.
  That is a real gap for the user most likely to want it, and it is written down rather than left to
  be found.

  **The focus model is one flag and it has to stay that way.** Exactly one of {document, field} takes
  a key, and `find_focused` is consulted in one place at the top of `handle_key`. Chords are
  deliberately *not* swallowed — Ctrl+S saves while the caret is in the box, because anything else
  would be a search box that quietly disables the editor. Only select-all, copy, cut and paste are
  taken by the field.

  **The search is over the source, so `**` is findable**, and the current match is simply the
  selection — which is why the reveal, the scroll and the highlight all follow with nothing new
  written for them. Matches are lit *under* the selection quad, so "the one you are on" needs no
  second colour and no index kept in step.

  **It rescans on the keystroke path, and the number is why that is allowed.** The first
  implementation cost **5.8 ms p50** per keystroke on a 224 KB document with 3,000 matches, against
  a 3 ms budget — so `find::matches` gained an ASCII path (a byte comparison for the first character,
  and `eq_ignore_ascii_case` for the rest) and it is now **1.13 ms p50 against 0.63 closed**. The
  ticket said to debounce; the measurement said it was not needed, and `crates/app/tests/find_cost.rs`
  is the harness that decides which of those is true after the next change. The non-ASCII path is
  unchanged and still correct: `İ` lowercases to two characters, so a match may not end half way
  through one character's expansion, and there is a test saying so.

  **The match list's key includes which document it is.** A revision counts one document's edits and
  says nothing about which — two tabs neither has typed in are both at revision zero — so without it
  switching tab kept the first one's offsets. Same shape, and same reason, as `floating_for`.
- **The same bar does Replace and Go to Line, and the mode is what makes it a different shape.**
  MDEDIT-45. Ctrl+H adds a second row with its own field and a Replace / All pair; Ctrl+G is one row,
  one field, digits only. `find::Mode` is in the match list's key too, so switching to Go to Line
  clears the highlights rather than leaving a search the user has moved on from lit up. **Three
  fields, not one reused**: Ctrl+G would otherwise throw away what you were looking for.

  **Replace All is one undo step, and that is what the ticket was waiting for.** `Document::replace_all`
  works **from the end backwards** inside MDEDIT-63's `transact`, so no site moves before it is used
  and no offset arithmetic is needed — undo restores the original bytes, property-tested. A call that
  replaces nothing records no history, so a Replace All that matched nothing does not cost a press of
  Ctrl+Z that appears to do nothing.

  **Replace with the caret away from a match moves to one rather than replacing what is nearby.** The
  second press is what edits. A Replace that acted on the nearest thing is how a find-and-replace
  loses work.

  The two word buttons are **measured**, not given the symbols' fixed width — "Replace" at 24 pixels
  would be a button with three letters on it. `Button::row` returns a row *number* rather than a y
  offset so grouping buttons by row is an integer comparison; comparing two `f32`s for equality would
  work today and fail the first time a height is scaled.
- **A settled selection also gets a bar that floats over it, and "settled" is the whole feature.**
  MDEDIT-53. It appears on mouse *release*, or on the idle timer for a selection extended with Shift
  — never during a drag, because a bar that chases the pointer is the classic bad implementation of
  this pattern. It is dismissed by typing, Esc, scrolling, collapsing the selection, clicking
  anywhere else, and switching tab. **Hidden on scroll rather than moved**: repositioning against a
  viewport still in motion is jitter.

  Three things are worth knowing before touching it. It anchors to the selection's **first visible
  row**, not the bounding box, and that rect is recorded in `draw_chunks` because nothing else keeps
  it — `row_selection` returns x-extents that go straight into a quad and are then gone, so
  `selection_anchor` is to the floating bar what `caret_area` is to the IME. It is drawn **genuinely
  last** and hit-tested **first**, and `Editor::click`'s order is the whole of that z-order in both
  directions. And the tab it belongs to is held in `floating_for` rather than hooked into the eight
  paths that activate a tab — a comparison cannot be forgotten by the ninth.

  Its buttons are the inline set plus three heading levels. The list and block actions stay on the
  fixed bar: they act on lines rather than on a selection. The selection **survives** a press, which
  is what makes bold-then-italic two clicks instead of two selections.

  **It does not appear for a selection the search made.** `found_selection` says where the current
  one came from, and `settle_selection` refuses while it is set. Both halves of that were found by
  eye rather than by a test: every press of Find Next put a bar over the prose, and then Esc — the
  gesture that dismisses chrome — raised one as a parting gift, because closing the find bar leaves
  the match selected. Keying it on where the selection came from rather than on whether the find bar
  is open is what fixes the second.
- **The View menu's toggles are check items, and the ticks follow the *chord* as well as the click.**
  MDEDIT-40. That is the whole reason a toggle belongs in a menu rather than only on a chord: a chord
  can toggle something but cannot tell you which way it is now. The cost is that every path which
  changes one has to call `refresh_view_menu` — a menu that only knew about its own clicks would be
  wrong the moment anybody pressed Ctrl+/.
- **Zoom is a multiplier, and clearing the caches is the whole of the feature.** MDEDIT-43: Ctrl+= /
  Ctrl+- / Ctrl+0, a 1.1 ladder. Text sizes come from the theme at layout time and are **not** in
  `ChunkKey`, so changing one without clearing leaves every cached chunk drawing at the old size.
  Five things hold laid-out text; four are cleared on zoom — chunk layouts, measured heights, the
  gutter's digits, the status bar's labels, the pane's row labels — and the tab strip's are measured
  per frame so need nothing. The anchor's `into_chunk` is in *pixels*, so it and the scroll offset
  are scaled by the same ratio or zooming mid-document throws the reader somewhere else. Adding
  anything else that caches laid-out text means adding it to `set_zoom`.
- **The recent-files list is state, and is treated as untrusted input.** MDEDIT-41 put it beside the
  session file rather than in `config.json` — the config file is read-only by design and may not name
  a path at all, and nobody hand-writes a recent list. That puts it in the session file's threat
  model: every entry goes through `path::validate` **on the way out and on the way in**, the second
  direction so the list cannot launder a path past the boundary by storing it now and reading it back
  later. `validate`'s last rule is that the path names a regular file, so a deleted file falls out of
  the menu without anything having to prune it.
- **Help → Keyboard Shortcuts is a Markdown document in a tab, not a dialog.** The editor showing its
  own help as Markdown is the cheapest demonstration that it renders headings, tables and inline code,
  and it costs no UI. It is generated from one list in `crates/app/src/shortcuts.rs` so the page and
  the bindings cannot drift silently. That needed `Origin::Generated`: a tab that is untitled but
  *not modified*, carrying its own text as its record of disk, because a reference document asking to
  be saved on the way out is a nuisance rather than a safeguard. It is never written to the session —
  it can be regenerated, and a stale copy of a reference is worse than none.
- **Closing the window asks nothing at all.** MDEDIT-82. `CloseRequested` once called
  `event_loop.exit()` and lost every unsaved buffer, which is what MDEDIT-64 put a Save All / Keep
  for now / Cancel dialog in front of — and then MDEDIT-54 and MDEDIT-66 made the work *survive*, so
  the dialog was asking the user to decide something the editor had already handled. It is gone.
  Leaving the program leaves the program; the work is beside the session file and comes back next
  launch.

  **`may_close` flushes a snapshot synchronously first**, and that is the part to keep. The copy on
  disk is whatever the idle timer last wrote, so an edit made and the window closed inside that
  interval would come back a few keystrokes short — which is worse than the dialog was, because it
  looks like the feature working. It still returns a `bool`: that is the seam `main` and the tests
  drive, and a caller that ignored the answer would also skip the flush.

  **Closing a *tab* still asks**, and `UnsavedPrompt::ask` is what is left of the dialog — one place,
  where a document is deliberately discarded with the user looking at it. `ask_all` and its native
  implementation are gone with the question they answered.

  **Not done, and the reason it matters:** a machine shutdown or log-off is still not an ordinary
  exit. winit handles neither `WM_QUERYENDSESSION` nor `WM_ENDSESSION` — checked in 0.30.13, the
  strings do not appear in its Windows backend — so a shutdown loses up to an idle interval of typing
  and the next launch calls it a recovery from an unclean run. The fix is a window subclass in
  `mdedit-menu-win`, where the platform `unsafe` already lives: flush, mark the exit clean, return
  `TRUE`, do not block the shutdown. It is the third item on MDEDIT-82's plan and wants a machine to
  test on, not a test.
- **Settings are written back now, and unknown keys survive it.** MDEDIT-60 replaced "read once and
  never written back" — four features were stalled on it. The file is read into a `serde_json::Map`
  as well as the typed `Config`, and a write merges the changed fields into that map, so a key from a
  newer build or one added by hand passes through untouched. **That rule is load-bearing**: a
  typed-struct rewrite silently deletes the future. Writes are atomic, and happen on the idle timer
  when something changes rather than on exit — a run that crashes never reaches its exit path. The
  one thing it cannot preserve is a malformed file's contents, because there was nothing valid to
  preserve. Which chrome is showing, and the zoom, persist through it.
- **A pane row truncates its name; it does not ellipsise it, and there is no hover.** MDEDIT-28's
  plan asked for eliding with the full path on hover. Truncation is what the tab strip already does
  and what this matches; an ellipsis needs its own measurement pass, and a tooltip needs a hover
  affordance that does not exist yet.
- **The pane lists what is open, not what is on disk.** A file *tree* is a different feature with
  filesystem-watching decisions attached, and deliberately not this one.
- **The status bar's word count lags, deliberately.** It is recomputed on the same idle timer that
  runs the whole-document re-parse 150 ms after typing stops, because counting words is a walk of the
  buffer and doing it per frame would undo MDEDIT-17 and MDEDIT-18. While you type, the number shown
  is the one from before you started. A document nobody has typed in yet is counted once, on its
  first frame.

  **Five things share that timer, and it is now scheduled after any edit.** It used to be set only
  when the parse had been scoped — i.e. only above 16 KB — so on an ordinary note it never fired at
  all, which froze the word count and, when MDEDIT-54 arrived, would have meant no crash-recovery
  snapshot for exactly the small unsaved buffers the feature exists to protect. `Editor::on_idle` is
  public so a test can run that work without an event loop.
- **A document can be told it is wrong, and that is a second strict pass rather than a stricter
  lexer.** MDEDIT-47. `Language::validate` returns `Diagnostic`s — a `ByteSpan` and a `&'static str`
  — and defaults to none, so **Markdown opts out by existing**: there is no such thing as invalid
  Markdown. Only JSON implements it. The lexers stay error-tolerant, which is not a detail: `{"a":`
  is what every JSON file looks like halfway through being typed, and a parse that stopped there
  would unhighlight the rest of the document at the moment the user is looking at it. That is why
  `serde_json` and `saphyr` were rejected, and adding failure to `analyze` would give it back.

  **It is the fourth thing on the idle timer, and a keystroke pays 3 ns for it.** Measured at 412 KB
  of JSON: the check alone is 1.3 ms, and what a frame pays is one comparison of `(tab, revision)`
  against the pair the last check was computed for. The keystroke A/B was inconclusive at ±50% run
  to run on a loaded machine, which is why the per-frame number is the one that answers it — see
  `crates/app/tests/validate_cost.rs`, and `json_typing_cost.rs`, which uses only long-standing API
  so it can be run against an older commit.

  **An edit clears the count rather than ageing it.** The word count shows a stale number because a
  stale number is nearly right and costs nothing; a stale *diagnostic* points at bytes that have
  moved, and Tools → Validate Document puts the caret on one. Same `(tab, revision)` key as the find
  bar's match list, and for the same reason — two documents nobody has typed in are both at revision
  zero. The menu item **recomputes before it jumps**, because the count on the bar may be an idle
  interval old and "take me to it" may not be.

  **The checker has an explicit stack, and that is a security decision.** JSON nests without limit,
  so recursive descent overflows on `[[[[[…` — a file, from the internet, opened in an editor. Safe
  Rust does not save you from a stack overflow, and `AGENTS.md` names hand-rolled file parsing as the
  CVE class this project must not reintroduce. It also stops at 200 diagnostics: nobody is fixing the
  201st. Absence of errors is **silence, not "0 errors"**, and an empty document is silent too — a
  new file is empty, and diagnosing that is diagnosing the user's existence.
- **A JSON document can be reformatted, and that is the only whole-document rewrite there is.**
  MDEDIT-48 — the decision itself is in the byte-preservation rules above, because that is the rule
  it is an exception to. `Language::format` returns `Option<Result<String, Diagnostic>>`, and the two
  layers answer different questions: `None` is **"this language does not reformat"** (Markdown has no
  canonical form; YAML cannot round-trip its comments and anchors), which greys the menu item, and
  `Err` is "it does, but not this document", which shows a message naming the line. Nothing compares
  `Language::name()` against `"JSON"`.

  **It emits the original bytes of every token**, which is the whole of how `1.50` stays `1.50`,
  `1e3` stays `1e3` and `-0` stays `-0`. Re-serializing through a number type loses all three, and
  through a map loses key order. Nothing here parses a value — it walks MDEDIT-47's token stream and
  decides where the line breaks go, which is also why it needed no second tokenizer. Property-tested
  both ways: the result is still valid JSON, and every non-whitespace byte is unchanged.

  **MDEDIT-47's validator is the gate**, so a half-typed document is refused rather than mangled — a
  format that "fixed" malformed input would be inventing structure the user has not finished typing.
  An already-formatted document is left completely alone, so there is no undo press that appears to
  do nothing.

  **The menu item's enabled state is compared per frame, not hooked.** Which document is active
  changes on a great many paths, and a list of call sites is a list somebody forgets to add to — the
  same lesson as `counted_for` and `floating_for`, in push form, with `tools_state` holding what the
  menu was last told so the native call happens only when the answer changes.
- **There is one seam for telling the user why something they chose cannot be done**, and it is
  `Editor::with_messages`. Inert by default and wired in `give_it_dialogs`, like every other
  collaborator that reaches the machine. Only for a refusal that answers something the user *just
  did*; anything the editor merely noticed goes to the log.
- **Unsaved work is stored, and comes back — after a crash *or* an ordinary quit.**
  `mdedit-session::snapshot` keeps a copy of each dirty buffer in a `recovery` directory beside the
  session file, addressed by an id rather than a path because the buffer with most to lose is the one
  that has no path. Written atomically on the idle timer, skipped when `Document::revision` has not
  moved. Caret and undo history are not saved: what comes back is the text, and pretending more
  survived than did would be worse than an honest restore.

  MDEDIT-54 built it for crashes and cleared it on every clean exit. **MDEDIT-66 made it the store
  for unsaved work generally**: it survives a quit, restore runs on every launch, and a copy is
  removed only when its buffer is saved or its tab closes. Adding a way to close a tab means adding
  a `forget` with it, or that document comes back for ever.

  **The store is wired in by `main`, not reached for by `Editor::new`** — MDEDIT-68, and the second
  instance of the mistake MDEDIT-67 was merged to remove hours earlier. The old default sent every
  test that edited a buffer and ran the idle timer into the *user's own* recovery directory, and
  MDEDIT-66 then restored those fixtures into their editor on the next launch. It was found by
  opening the editor and reading a tab; no test could see it, because every test in `recovery.rs`
  installs a store and so none of them exercised the default. `Editor::stores_unsaved_work` exists so
  one now does.

  **`give_it_dialogs` runs before `restore_unsaved_work`, and that order is load-bearing both ways.**
  The restore reads the store this installs, so a restore above it would silently find nowhere. It
  also repairs a safeguard that had never once fired: `restore_one` asks before putting a stored edit
  back over a file that changed on disk, and it asks through the conflict prompt `give_it_dialogs`
  installs — which, when the call sat *below* the restore, was always still the inert `ChangeNothing`.
  No work was ever lost by it (that inert answer is Cancel, which keeps the buffer), but the question
  MDEDIT-66 documents was not being asked.
- **A crash is told from a quit by a marker in the session file**, not by whether anything was left
  behind — that stopped working the moment the store outlived a quit. `clean_exit` is written by the
  exit path and **cleared at startup**, so it describes the run in progress. The clearing is not
  optional: a killed run never rewrites the file, so without it the previous quit's `true` survives
  and the next launch calls a crash recovery an ordinary restore. Only the wording depends on it, but
  the wording is the whole point — an ordinary restore says nothing, a recovery says so.
- **The way out asks nothing; Discard lives on the per-tab close.** MDEDIT-82 removed the window
  dialog entirely — see the entry above — and the reasoning that shaped its buttons is why there is
  nothing left to ask: Keep was "do what the store already does", Discard would have been Keep with
  worse words since the copies survive it, and Save was a question the editor could answer itself.
  Closing *one* tab is where a document is discarded, because the user is looking at it. A scratch
  tab is asked with "Save As…" rather than "Save" — a button labelled Save that opens a file picker
  is a small lie, and before MDEDIT-66 picking it did nothing at all.
- **A stored edit is checked against the file before it is put back.** If something else wrote the
  file since, restoring silently would put an edit on top of somebody's work — the same hazard
  MDEDIT-55 answers while the editor runs. Declining removes the stored copy, or the question returns
  on every launch. `Stamp` holds **milliseconds rather than a `SystemTime`** so it survives JSON
  exactly; a stamp that shifted in transit would report every restored file as modified.
- **A missing file is not a hostile one.** `validate`'s last rule is that a path names an existing
  regular file, and applying it to the store threw away the text of any file that had been *deleted*
  — the one case where the stored copy is the only copy left. A missing file now downgrades the entry
  to one with no path and the text survives as a scratch buffer; the three security rules (UNC and
  device paths, relative paths, traversal) still refuse outright, and there is a test saying so.
  **When relaxing one rule in a boundary, prove the others are intact in the same commit.**
- **A file changed on disk is noticed on focus and before every save.** MDEDIT-55: `mdedit-file`
  carries a `Stamp` — modification time *and* length, never a hash, because comparing content means
  reading every open file on every focus. The stamp is taken **before** the read, so a write racing
  it leaves a stamp older than the bytes and the next check says "changed" (harmless) rather than
  "unchanged" about a file rewritten underneath (the whole failure). The focus check keeps buffers
  fresh; the save check is the one that prevents data loss, and lives inside `write_active_to` so
  every route to a save passes it. **No filesystem watcher** — `notify` brings threads and four
  platform backends for a case that happens between focus changes; the gap is a file rewritten while
  the window stays focused, which costs freshness rather than work.
- **There is an encoding picker now, and choosing one is the third sanctioned rewrite.** MDEDIT-42.
  File → Reopen with Encoding and Save with Encoding, over a short practical list —
  `mdedit_file::choices`, whose **order is the contract** because a menu click reports a position.
  `mdedit-menu-win` is told the labels and knows nothing else about encodings, the same rule that
  keeps `Newline` out of it.

  **Reopen works on a tab that never opened**, which is the whole point: a windows-1252 file is not
  valid UTF-8, so it fails to open, and that is the tab the menu exists to rescue. `Workspace::activate`
  therefore marks a tab active **before** the read rather than after — a file that cannot be decoded
  still leaves the tab the one being looked at, and without that there was no active tab for the menu
  to act on. Reopening asks through the Close prompt when there are edits to discard, and a wrong
  guess costs nothing because the read happens before anything is replaced.

  **Saving with a different encoding disarms the byte-identical fast path.** `save` writes the
  original bytes verbatim when the text is unedited; after a deliberate encoding change that is
  exactly wrong, so `LoadedFile::set_encoding` sets `re_encode`. Without it an untouched buffer came
  back in its old encoding with nothing to say the choice had been ignored.

  **`encoding_rs` cannot encode to UTF-16 and does not fail when asked.** The Encoding Standard makes
  the UTF-16s decode-only and maps their output encoding to UTF-8, so `UTF_16LE.encode` returns UTF-8
  bytes with no error — which behind a UTF-16 BOM is a file no reader can parse, reported as a
  successful save. `FileFormat::encode` does UTF-16 by hand, which is safe in a way the legacy
  encodings are not: every `char` fits, and a `&str` holds no unpaired surrogate.

  **Content the target cannot hold refuses the save and writes nothing**, and now *says so* — a
  failed save goes through `with_messages` rather than only to the log, because a save that fails
  silently is the worst outcome there is.
- **Line endings can be converted, and saving still never converts anything.** MDEDIT-59, the fix for
  what MDEDIT-46 made visible. File → Line Endings → LF or CRLF rewrites **every** terminator —
  including the lone `\r` that nothing else here produces, which is the one a user is most likely to
  be trying to get rid of. One `replace` inside a transaction, so one Ctrl+Z restores the original
  bytes; a conversion that would change nothing makes no edit at all. The same exception the
  byte-preservation rules grant Format Document, for the same reason: the user asked.

  **Three things have to agree — the buffer, the `FileFormat` and the status bar** — and the third is
  how you find out the first two do not. So the conversion sets the format too, and **Enter inserts
  the file's own ending** rather than always `\n`. It used to always insert `\n`, which is how a CRLF
  file becomes Mixed without anybody pasting anything into it.

  **`Newline` grew a `Cr` variant**, because a classic-Mac file was being reported as LF — a document
  containing not one of them. MDEDIT-46 draws those endings in the warning colour, which put the
  bar's wrong answer directly beside the evidence. It is never a conversion *target*: nothing offers
  to make a file that.

  The submenu's ticks say what the document **is**, so a Mixed document ticks neither. Refreshed by
  comparison per frame, like the Tools menu's enablement.

  **The endings are re-detected on the idle timer**, which is the fifth thing on it. They used to be
  detected once at load and believed for ever, and two things made that a lie: undoing a conversion
  put the bytes back without putting the format back, and *pasting* CRLF into an LF file — which this
  editor deliberately preserves — had never updated the bar at all. Deriving it from the buffer makes
  both self-correcting; it costs a walk of the buffer, which is why it is beside the word count and
  not anywhere a keystroke reaches. **The undo half was found in the window, not by a test.**
- **Whitespace and line endings can be drawn, and drawing is *all* it does.** MDEDIT-46. Spaces are a
  middot, tabs an arrow, the rest of C0 its Unicode Control Picture, and each line ending its own mark
  — `␊`, `␍␊`, or a lone `␍` **in the warning colour**, because a bare CR is nearly always damage
  where the other two are ordinary. This is the answer to "Mixed" in the status bar, which until now
  said a document had mixed endings without saying which lines.

  **Every mark is stamped into the cell the character already has, at the advance the layout already
  gave it.** Nothing re-shapes, so no glyph moves, no row re-wraps, and no click lands anywhere new.
  That is why **the toggle clears no layout cache**: MDEDIT-46's plan expected one by analogy with
  zoom, and the analogy does not hold — zoom changes what the text engine is *asked for*, and this
  changes nothing it is asked for. The marks themselves are a sixth cache of laid-out text, so they
  *are* cleared by `set_zoom`.

  Per-run and not remembered, like the drawing mode and for the same reason: it is a way of looking
  at a document for a moment rather than a preference.

  **Two of its three defects were only visible in the window, and both came from the same place** —
  the headless stub shapes text more simply than DirectWrite does. `view::ending_before` exists
  because the offset past a row's last glyph may be the terminator or past it, and the whitespace
  marks walk trailing spaces out of the *text* rather than off the glyphs. **A test on the stub
  cannot see either.**

  **The DirectWrite behaviour those two were written for is not what DirectWrite does**, which
  MDEDIT-110 measured and MDEDIT-113 carries. This entry used to say a `\n` gets a glyph of its own
  and trailing whitespace is not shaped; on Windows 11 26200 against the installed Segoe UI, both are
  the other way round — `"ab\ncd"` produces glyphs for bytes 0, 1, 3 and 4, and `"ab   "` produces
  one for every trailing space. `crates/text-dwrite/tests/layout.rs` pins that as a characterisation
  test, so a change is noticed rather than assumed; whether `ending_before` and the whitespace walk
  should be simplified to match is MDEDIT-113, and wants an eye on Windows rather than a test.
- **The breadcrumb shows headings, and nothing else.** A caret above the document's first heading
  gets an empty breadcrumb rather than a root. Nothing in the bar is clickable.
- **The status bar is two containers that truncate, not one row that overflows.** The right-hand
  fields ask first, up to 60%% of the bar; the breadcrumb takes what is left, and both clip at their
  own edge. Because the fields read with the least important last, what a narrow window loses is the
  word count, then the line endings, then the encoding — line and column survive longest, which is
  the priority order without anything encoding it. Truncated, never ellipsised: an ellipsis needs its
  own measurement pass.
- **Line and column count characters.** Not bytes, and not display columns — a tab is one column
  here. `ByteOffset` and `CharOffset` exist as separate types so this cannot be fudged; see
  `Document::line_and_column`.
- **There are two palettes, and one of them is unreachable.** MDEDIT-71. `Theme::dark` is what the
  editor has always drawn with — **unchanged to the byte**, which was that story's whole acceptance
  criterion — and `Theme::light` is the MDEDIT-69 design pack's Nocturne, complete and
  contrast-checked and read by nothing until MDEDIT-78 adds the OS read and the config key. `Default`
  is dark, so no caller changed and no test was touched.

  **`Appearance`, not `Mode`.** `Mode` already means how the *document* is drawn — preview against
  plain markup — and the two are independent: plain markup is still dark.

  **Seven roles exist that nothing reads**: `span_error`, `warn`, `link`, `muted`, `gutter`,
  `caret_line`, `line`, `panel`. Each names the story that will consume it. They are here rather than
  added one at a time because every story after this would otherwise re-open the struct, and because
  the light values have to be chosen against one another to be checked against one another.

  **`span_error` is rose and `status_error` stays amber**, deliberately two roles where the pack had
  one. Red in a status bar reads as "the editor broke"; a squiggle under the offending bytes is
  exactly the case a warning colour is for. Both arguments are right about different things.

  **The contrast test names its pairs, and that is the point of it.** `crates/app/tests/contrast.rs`
  computes WCAG ratios from the palette rather than checking in a table of numbers, and it asserts
  only the foreground-on-background combinations the renderer actually produces — traced from the
  draw sites. Asserting everything against everything measures fictions: `token_comment` on
  `tab_active` is 4.13:1 and irrelevant, because a JSON comment is never drawn on a tab. The pack
  itself measures against the page alone and so misses every pair that is on chrome.

  It carried **one known shortfall** — the close `×` on the *active* tab, `marker` on `tab_active` at
  4.31:1 — which MDEDIT-72 cleared at 5.25:1, so the list is gone rather than shorter.
- **The dark palette is Nocturne, and that is the visible one.** MDEDIT-72. Four hue families over
  one neutral ramp: lavender for structure, sage for literals, sand for numerics, steel for
  constants, rose for errors alone. The editor stops looking like VS Code's default.

  **Six roles are not in the pack and are derived from its ramp**, each for a stated reason.
  `find_match` is sand and translucent, so that blended over the page it is within 1.2× of the
  selection's *brightness* — the two are told apart by hue, which survives peripheral vision and
  colour blindness, and not by weight, which is the failure mode. `strip`, `tab_active` and
  `tab_inactive` are three surfaces from the pack's two: the strip is `chrome`, the active tab is
  `panel` because it is the raised one, and the inactive tab is a ramp step between the page and the
  panel. `tab_modified` is the accent. `token_comment` is a dimmed sage, because the pack sends
  comments to `muted` and the editor already makes that distinction.

  **`gutter` is the one value deliberately not the pack's.** The pack measures every role against the
  page alone; against `panel` its `#82869a` is 4.22:1, under its own floor, on exactly the surface a
  file pane is. `#8b8fa4` clears it at 4.75:1 there and 5.50:1 on the page, and
  `the_gutter_clears_the_floor_on_a_panel_as_well_as_on_the_page` is what keeps that true.

  **The file pane draws on `panel` now**, not on the tab strip's colour — which it only ever borrowed
  because the strip's was the one dark chrome value there was.

  **The tints are real alpha and the pipeline is `ALPHA_BLENDING`,** so `contrast.rs` blends before it
  measures: what a reader compares is the tint over the page, not the entry in the palette. Text on
  the find highlight, and on the current match where the selection sits over it, are both asserted.

  **The status bar's count stays amber and a diagnostic span stays rose**, asserted rather than
  trusted, because collapsing them into the pack's single `error` role is the easy mistake here.

  Verified by eye on Windows against Markdown, JSON and YAML — the four families read distinctly, the
  current match is lavender against the other matches' sand, and the count is still amber. The
  scrollbar thumb is 2.01:1 against the page where the old palette was 2.07:1; it is not text.
- **Quoted prose and `true`/`false`/`null` are italic, which is half the contract's second cues.**
  MDEDIT-74. The pack's rule is "never colour alone": a role's colour always has a second signal, so
  a reader who cannot separate two greys or two blues still sees the distinction.

  **A quote is asked of the block list, not tracked in `DecorState`.** That state resets at every
  newline — inline delimiters pair within a line — and a quote is the opposite shape, spanning lines
  by definition, so a toggle would be cleared by the first one. `Document::in_block_quote` answers
  "is anything enclosing this byte a quote", which `block_at` cannot: it returns the *innermost*
  block, and a quoted paragraph is a paragraph. Same binary search and same bounded walk, because the
  renderer asks it once per line and a scan of the block list is MDEDIT-37 again.

  Markers stay upright — a slanted `>` reads as prose rather than syntax — and so does code inside a
  quote, because italic monospace is where alignment goes to die.

  **`Token::Literal` is the first token to reach `TextStyle` rather than only a colour.** A quoted
  `"true"` is `Token::String` and stays upright: the classification already knows the difference, and
  italicising on the text would be ignoring it. There is a test that says so.
- **The light palette is reachable, and the editor follows Windows by default.** MDEDIT-78. A `theme`
  config key — `"system" | "dark" | "light"`, never a path — three View → Appearance items that tick,
  and `Theme::for_appearance` doing the rest. Half the design pack was unused until this.

  **The setting is read from the registry, and that is not a style preference.** `AGENTS.md` and
  `docs/research/05-windows-platform.md` both say `ShouldAppsUseDarkMode` is unreliable from 1903, so
  `mdedit-menu-win::apps_use_light_theme` reads
  `HKCU\…\Themes\Personalize\AppsUseLightTheme` with one `RegGetValueW`. It lives in the menu crate
  because that is where the platform `unsafe` for appearance already is — **no fourth unsafe crate**.

  **winit's `ThemeChanged` is not a reliable notification, and this is the measurement.** winit raises
  it from `WM_SETTINGCHANGE`, but only when *its own* answer changed — and its answer comes from
  `ShouldAppsUseDarkMode`, the API this project does not trust. Flipping `AppsUseLightTheme` under a
  running editor on Windows 11 produced **no event at all**: the palette did not move for five
  seconds, while a direct read of the registry returned the new value immediately. So the event is
  wired up and welcome, and two event-driven checks stand behind it: **on focus**, beside
  `check_disk`, because coming back to the window is when somebody who just changed the setting is
  looking at it; and **on the idle beat**, which is one registry read of a few microseconds against a
  whole-document re-parse. Neither is a timer — `on_idle` runs when the loop is about to wait after
  handling input, so an idle editor asks nothing. Verified by eye: flip the setting, press a key, the
  document is light.

  **Nothing is cleared on a switch, and that is asserted rather than reasoned.** MDEDIT-78's ticket
  asked for exactly that: `theme_switch.rs` swaps palettes and checks that no quad still carries a
  colour from the old one, and that the frame after a switch lays out no more text than an ordinary
  frame. It holds because colour is in neither `TextStyle` nor `ChunkKey` — every cache holds
  laid-out glyphs, and a glyph is the same shape in both palettes.

  **The zoom is carried across by ratio**, taken from the theme being replaced rather than from the
  config or the settings: those two disagree for as long as it takes `with_config` to run, and a
  palette switch that quietly reset the text size would be a strange bug to hunt.

  **The menus followed on MDEDIT-79**, which closed the epic: the mode `mdedit-menu-win` forces is a
  parameter now, taken from the editor's resolved palette rather than from Windows'.
- **The menus are drawn in the editor's palette, not Windows'.** MDEDIT-79, which closes MDEDIT-69.
  `dark.rs` forced `PreferredAppMode::ForceDark` unconditionally and the bar attached as
  `MenuTheme::Dark`; both are parameters now, and `MenuBar::set_appearance` re-themes an *attached*
  bar so a switch needs no restart — `FlushMenuThemes` is the call that makes that true, and a menu
  bar that only picked the change up on the next launch would have been a bug rather than a limit.

  **Still not `MenuTheme::Auto`, and that is the whole point.** Auto follows Windows; the editor's
  palette may be pinned against Windows by `theme`. On a light-themed machine with `"theme": "dark"`,
  Auto would put a light bar over a dark document — MDEDIT-32 reintroduced in exactly the case the
  config key exists to serve.

  **Verified in light, not in dark.** A pinned-light editor draws a light bar and light dropdowns.
  The dark dropdown could not be confirmed on the machine this was built on (Windows 11 26200) — the
  popup photographed light with `attach` provably receiving `Dark`, which points at the uxtheme
  ordinals rather than at this wiring, since forcing dark popups is the undocumented best-effort
  trick `dark.rs` has always been. **If dark dropdowns are broken there, they were broken before this
  story**: nothing here changed how the call is made, only what it is passed. Worth an eye and a
  ticket rather than a claim in either direction.
- **The gutter has its own colour, and the caret's line is lit.** MDEDIT-77. Line numbers draw in
  `gutter` rather than borrowing `marker` — which was a good call while there was one dim colour and
  stopped being one when there were several — and the caret's own number steps up to `text`, which is
  the pack's rule for the role. The row itself takes the `caret_line` tint, a 7% lavender.

  **Two cues for one question.** The tint is deliberately near invisible, which is exactly why the
  brighter number exists: a reader who cannot see a 7% wash still sees which number is bright. The
  gutter is off by default, so on a default install the tint is the only cue — that is a real gap and
  it is written down rather than fixed by turning a setting on for people.

  **Three decisions the ticket asked to be made explicitly**, all in `draw_caret_line`'s docs: every
  row of a **wrapped** line is lit, because a line is a source line and that is what every editor
  does; it stays lit while the **find field** has focus, because the document's caret has not moved
  and Esc puts you back there; and it stays lit **under a selection**, so two tints stack.

  **`contrast.rs` asserts the stack**, which the pack does not model: it measures every role against
  `bg` alone and claims "selection tints preserve 4.5:1" — a claim about one tint, not two. Text on
  the caret line, and text on a selection over the caret line, are both asserted, as is the caret's
  number reading brighter than an ordinary one.

  **In the gutter, rose outranks bright outranks dim**: a line that is both flagged and current stays
  rose, because something wrong outranks where you are.

  The tint is drawn **first** on its row, so the selection, the search highlights and the glyphs all
  land on top. It spans the document's width and its margins but **not** the gutter, which has its own
  answer. It is placed in window coordinates rather than panned with the text: a highlight that slid
  sideways with a horizontal pan would leave the rest of its row untinted.
- **A diagnostic is drawn on the document now, not only counted.** MDEDIT-76. A rose squiggle under
  the offending span, and the line's gutter number in rose and bold with a `!` in front of it — the
  design pack's `error` role is "paired with a gutter marker", and adopting the colour without
  drawing one would have been claiming a cue that did not exist.

  **The squiggle is a row of quads**, because there is one pipeline and it draws rectangles: short
  dashes alternating between two heights, which reads as a wave at body size and needs no shader.
  Its geometry comes from `row_selection`, the same helper the selection and the find highlights use,
  so a span crossing rows is clipped per row by code that already knows how.

  **It follows `diagnostics()`, so an edit takes it away and the idle timer brings it back**, with no
  hook on any edit path — the same `(tab, revision)` staleness rule the count has, and for the
  stronger reason: a squiggle under bytes that have moved points at innocent text. **Nothing on the
  drawing path re-runs the check.** Measured at 412 KB of JSON: typing p50 **31.7 ms before, 29.6 and
  32.1 ms after** across two runs — the change is inside the run-to-run spread. `validate_cost.rs`.

  **The check's 200-diagnostic cap is what makes the per-chunk filter free**: the whole list is
  shorter than one screen of find matches, so it is scanned rather than binary-searched.

  **The squiggle draws *after* the row's glyphs.** Everything else under a row is a ground the text
  sits on; this is a mark *about* the text, and a descender must not be able to hide it.

  **A gutter number sits on its line's baseline, and did not before.** Found by putting a `!` there
  and looking: the digits were placed from the row's *top* and then offset by the glyph's own
  distance from the baseline, which is negative — so every number was drawn about an ascent above the
  line it belonged to. `gutter.rs` asserts where numbers sit *horizontally* (MDEDIT-34) and the
  headless stub gives every glyph a one-pixel offset, so the error was one pixel in a test and most
  of a row in the window. Now asserted vertically too.

  **With line numbers off there is no gutter, so the squiggle is the only cue.** Reserving gutter
  space when an error appears would reflow the document at the moment something went wrong, which is
  worse; the honest answer is that the second cue depends on a gutter being shown.
- **Markdown's syntax is coloured per kind, and `Role` carries the marker to do it.** MDEDIT-75. All
  thirteen `Marker` variants used to collapse to one colour, so a heading's `#`, a bullet, a fence and
  a link's `](` were the same green. Two kinds leave the `marker` colour, and both are landmarks
  rather than scaffolding: a **list bullet** takes `key`, and a **link destination** takes `link`,
  underlined — MDEDIT-73's first consumer.

  **A destination is not a `Marker` variant and does not become one.** `pulldown-cmark` reports a link
  as one construct and subtracts its *text*, so `[a](url)` arrives as the markers `[` and `](url)`,
  with the destination inside the closing marker's own segment. `view::push_line` splits that segment
  by a character class (`brackets_a_link`) rather than re-deriving where the URL starts: the
  alternative is a second link parser in the renderer, handling angle brackets, titles and reference
  labels. A reference label and a title take the destination's colour under that rule — the right
  answer for a label, and a knowingly loose one for a title. **Nothing about the locator or its tiling
  property test changes**: this is a renderer-side split of bytes the locator already classified.

  **A link's text is `Content`, so it is told from prose by where the walk is.** `DecorState` grew
  `link_text`, set from the marker's own bytes — `[` and `![` open one, everything else closes it —
  rather than by counting segments, which depends on the form the link is written in. Unlike
  MDEDIT-74's quote it *is* per-line state, because a link is inline and pairs within a line like the
  emphasis beside it.

  **Code content takes `string` in both forms.** A code span's content is `Content` too, and the
  `code_span` flag already tracked for the monospacing carries the colour with it; a fence's content
  comes from the block list.

  **`style_at` became `decorate`, returning the style and the colour together**, because both now need
  the same `block_at` answer and asking twice would double the per-run cost MDEDIT-37 went to find.

  **Concealment did not change**, which is the acceptance criterion that mattered: a marker still
  vanishes off the caret's line, and a test asserts the drawn text of a document full of syntax. Plain
  markup still shows every byte — and keeps the destination's underline, because "never colour alone"
  is not a preview-only contract.
- **A run can be underlined, and where the rule sits is the font's answer rather than ours.**
  MDEDIT-73. `TextStyle::underline` reaches DirectWrite as `SetUnderline`, and what comes back is a
  `mdedit_text::Underline` **per row** — x, width, top and thickness, the last two from the font's own
  metrics. That is what makes a wrapped underlined run underlined on both rows, and what makes the
  rule grow with a zoom, without the renderer knowing a rule from a rectangle: it draws the quad and
  decides only the colour, which is the colour of the run above it.

  **Nothing asks for one yet.** The theme's `link` role is what will, and colouring markers per kind
  is MDEDIT-75; this is the same shape as MDEDIT-71's unread roles, deliberately.

  **`DWRITE_UNDERLINE` carries geometry and no text position**, so a rule is matched to its text by
  *position* — the first glyph under it names the byte the colour is looked up from. A rule with no
  glyph under it is whitespace only, and is dropped rather than coloured by a guess.

  **Underline is not in `ChunkKey` and does not need to be**: like every other `TextStyle` field it is
  derived from the chunk's bytes and the drawing mode, and both of those are.

  **No test on the stub can see any of this** — a rule's position comes from a font and the stub has
  none, so it reports no underlines at all. `crates/text-dwrite/tests/layout.rs` is where the
  behaviour is held up, against the real system stack, which is the only oracle this layer has.
- **The drawing mode is global and deliberately not remembered.** Toggling to plain markup switches
  every tab, and the editor opens previewed every time. Unlike the chrome choice, the zoom and the
  line numbers — all three of which persist since MDEDIT-60 — this one is left per-run on purpose:
  it is a way of looking at a document for a moment rather than a preference, and an editor that
  opened in plain markup because you glanced at the source last week would be wrong.
  `plain_markup_does_not_persist` pins it, so changing it is a decision rather than an accident.
- **Menu accelerators are labels, not bindings.** The items show "Ctrl+S" and so on, but they do not
  fire: muda's accelerators need the message loop to call `TranslateAcceleratorW`, and winit's does
  not. That is deliberate — the key handler owns those chords, and two things acting on one Ctrl+Z
  would undo twice.

  **The cost is that a label and a binding can disagree with nothing to catch it.** MDEDIT-65 was
  exactly that: Ctrl+Shift+S was labelled "Save As" and plain-saved over the file, because winit
  reports Shift+S as `"S"` and the arm did not read Shift. Nothing *could* have caught it — a chord
  arrives as a winit `KeyEvent`, which carries a private platform field and cannot be constructed by
  a test. So the character dispatch is now `Editor::apply_chord(character, shift)` and every chord is
  reachable from `crates/app/tests/chords.rs`. When adding a chord, add it there and to
  `crates/app/src/shortcuts.rs`, whose test asserts every implemented chord is written down.
- **The clipboard is text only, and a paste keeps its own line endings.** An image on the clipboard
  is ignored, because a Markdown buffer cannot hold one and deciding where to write the file would be
  a feature rather than a paste. Pasted text goes in verbatim, so pasting CRLF into an LF file leaves
  mixed endings — the byte-preserving choice, and the opposite of what VS Code does. Revisit if it
  annoys anyone.
- **Reading the real clipboard is a UI-thread-only operation, and the compiler enforces it.**
  MDEDIT-67: reading the Windows clipboard from two threads of one process corrupts the heap.
  `arboard`'s read path sizes a conversion buffer from a clipboard handle and then converts into it,
  and that handle is valid only while *this process* has the clipboard open — but `CloseClipboard` is
  process-wide, so any other thread finishing its own clipboard work invalidates it mid-read. Measured
  in `spikes/clipboard-threads`: one thread reading 2,000 times is clean, two threads reading abort
  8 runs in 8, and one reader against eight writers aborts 4–6 in 8 — so it is not two readers racing
  each other, and two threads are enough. `clipboard::System` is therefore `!Send`.

  **And an `Editor` is built with a clipboard that holds nothing; `main` installs the real one.** That
  is what the bug actually was. Every other collaborator touching the user's machine — the prompts,
  the file pickers, the recent-files list — already defaulted to inert with the real one wired in
  `give_it_dialogs`, and the clipboard was the sole exception. So every test binary read the real
  clipboard, on as many threads as the harness runs tests on: `context_menu` aborted with
  `STATUS_HEAP_CORRUPTION` 15 runs in 20. Adding a collaborator that touches the machine means adding
  it to `give_it_dialogs`, not to `Editor::new`.
- **A word is a Unicode word, and that is decided once for the whole editor.** MDEDIT-57 took
  `unicode-segmentation` (UAX #29) rather than an alphanumeric-run classifier, which splits `café` at
  a decomposed accent, takes a whole CJK run as one word, and cuts `don't` in half. Ctrl+arrows and
  MDEDIT-50's wrap-the-word want the same definition. One deviation, deliberate: UAX #29 breaks a run
  of punctuation into one segment per character, so runs of *non-word* segments are joined — word
  segments are not, because joining those is the CJK failure again. See `Document::word_at`.
- **Clicks are counted in the editor, because winit does not count them.** The double-click interval
  is injected rather than hardcoded: Windows has a setting for it and somebody who lengthened it did
  so because the default fails them. `GetDoubleClickTime` is read in `mdedit-menu-win`, where the
  `unsafe` belongs. Being a parameter is also what makes click counting testable without sleeping.
  A run wraps at three, so a fourth click is a caret again.
- **Inline formatting removes delimiters it can point at, not ones it guessed.** MDEDIT-50:
  `Document::toggle_inline` uses the marker spans, so `**a _b_ c**` with the caret in `b` takes off
  the italic pair and nothing else. That is the differentiator — every editor that bolts Markdown
  buttons onto a text widget does un-bolding with a regular expression and gets nested emphasis
  wrong. Toggling twice is byte-identical, property-tested.

  **Ctrl+B is bold, and the file pane moved to Ctrl+Shift+E.** Non-negotiable: a Markdown editor
  where Ctrl+B moves a sidebar is wrong in a way people notice within a minute. Ctrl+I is italic,
  Ctrl+Shift+X strikethrough, Ctrl+E inline code — the last is this project's choice rather than the
  ticket's, and the weakest of the four.

  **Inside a code span the inline toggles do nothing.** `**` there is not markup, so a second press
  could not find it to take away, and a toggle that cannot be un-toggled is not one. The empty pair a
  press with no selection leaves behind *is* matched, textually and narrowly, for the same reason —
  otherwise pressing the chord twice by mistake leaves `****` with no way off but backspace.
- **There is no menu to reach copy and paste from**, only the chords. A context menu is MDEDIT-58.
- **Startup costs ~250 ms before anything is drawn, and none of it is ours to fix.** MDEDIT-20
  attributed it with `spikes/startup-attribution`: creating the window is 60–79 ms, `request_adapter`
  70–100, `request_device` 62–72, `surface.configure` 39–54. Shader compilation is **0.3 ms** and
  pipeline creation under 11, so the obvious suspects are not it, and `PowerPreference` makes no
  difference. Creating a window alone exceeds the ≤50 ms target before any GPU work starts, so that
  target has been **split**: cold start is recorded rather than budgeted, and `file open → first
  paint ≤50 ms` now applies to a process that is already running — which is the case that matters,
  because opening N files from Explorer is required to reuse one process. See
  `docs/research/06-performance-targets.md`. Still measured on every run: `mdedit` prints
  `launch -> first paint` on exit.
- **A keystroke re-parses a region, not the document.** MDEDIT-18: an edit re-parses the block it is
  in plus a block of margin either side, and splices that into the existing analysis. A region
  cannot see what the rest of the document does to it — an unclosed fence above, a link reference
  definition below — so the whole document is re-parsed 150 ms after the typing stops. Documents
  under 16 KB skip all of this and parse whole, exactly as they used to.
- **A chunk is a whole top-level block.** A document that is one enormous list or one enormous
  paragraph is one chunk, and no cheaper than before — only its *drawing* is culled, line by line.
  Splitting inside a block needs the layout engine to lay out part of one, which none of them does.
- **Off-screen heights are estimated**, at a line count times a nominal line height, so a long
  wrapped paragraph nobody has scrolled to yet is under-estimated. The estimate is corrected the
  moment a chunk is measured, and the viewport is anchored so the correction does not move what is
  on screen — but the document's total height, and therefore how far it can scroll, is approximate
  until it has all been looked at. This is what a scrollbar would make visible, and there is none.
- **Scrolling has two axes now, and the second one only exists when word wrap is off.** MDEDIT-49.
  View → Word Wrap, per-run like the drawing mode; with it off, chunks lay out at `UNWRAPPED` (a
  large *finite* width, because it goes into `ChunkKey` and is hashed there) and **Shift+wheel** pans.
  No horizontal scrollbar, the same position as the vertical one. Scrolling is still instant — no
  smooth or momentum scrolling.

  **`ContentRect::to_window` takes an `Offset`, not an `f32`,** and that is the whole safety argument
  of the ticket: the conversions used to take a bare vertical scroll, so every caller that had never
  considered a second axis kept compiling and would have drawn in the wrong place. Making it a pair
  made the compiler ask each of the ten call sites once. The gutter and the file pane are unaffected
  because they draw in window coordinates and never ask.

  **The horizontal clamp is measured, not estimated**, so it only knows about chunks that have been
  laid out — the same position the vertical axis is in with its estimated heights, and it grows as
  more of the document is looked at. `widest` is cleared with the layout caches by both `set_wrap`
  and `set_zoom`.

  **The caret's horizontal reveal happens after `warm`, not beside the vertical one**, and that is
  load-bearing: before `warm` the chunk the caret is in may not be measured — which is *always* true
  of the chunk just edited — so `widest` was zero there and the reveal clamped to nothing. Typing at
  the end of a long line left the caret off the right edge.

  **And a pan clears `follow_caret`, or it is undone by the frame it asks for.** This is the one the
  window found and the tests could not: Shift+wheel is the only way to pan, **Shift is a key press**,
  so holding it re-armed the caret reveal — the pan landed, the next frame revealed the caret at the
  left edge, and the offset went straight back to zero. Panning did nothing at all, on any document,
  while `pan_by` itself was provably correct in isolation. The vertical axis has the same hazard and
  hides it, because a plain wheel is not preceded by a keystroke. **Anything that moves the viewport
  deliberately has to say so** — `drag_thumb` already did, which is what it should have been read
  against.

  Measured at 333 KB of prose: **wrap on p50 0.98–1.36 ms, wrap off 1.23–1.48 ms** across three runs,
  both well inside the 3 ms budget and overlapping run to run. `crates/app/tests/wrap_cost.rs`.
- **A whole Explorer selection reaches one process, and a `DropTarget` verb is the only way that is
  true.** MDEDIT-61. The obvious registration — `shell\open\command` holding `"mdedit.exe" "%1"` —
  defaults to `MultiSelectModel=Document`, which runs the command **once per file** and past fifteen
  stops invoking the verb at all, silently. Notepad++ ships that. A `DropTarget` verb points at a
  CLSID with a `LocalServer32`, Explorer activates it through COM, and because the running instance
  published its class object the activation **binds to the process already running** and arrives as
  one `IDropTarget::Drop` carrying the lot. Measured: twenty files, one process, twenty tabs, in
  selection order.

  **`REGCLS_MULTIPLEUSE` is the reuse**, not a mutex. There is deliberately no single-instance lock:
  launching `mdedit` twice by hand still opens two windows, exactly as before. Anything that made a
  second hand-launch impossible would be a different feature and a worse one.

  **Registration is manual, per-user and reversible** — `--register` / `--unregister`, the first
  flags this binary has. MDEDIT-33's "no flags" stance was about *file* arguments; these are checked
  before anything treats an argument as a path, and the cost is that a document called `--register`
  cannot be opened by name. Everything is written under `HKEY_CURRENT_USER`, which needs no elevation
  by construction — and `never elevate` is a rule here rather than a preference, because elevating
  breaks drag-and-drop from a non-elevated Explorer. No installer grows out of this: the gap register
  parks the installer and updater deliberately, and keeping this manual is what stops half of one
  appearing by accident.

  **The footprint is a value, and that is what makes unregister exact.** `registry::plan` returns
  every key and value; registering folds over it forwards and unregistering folds over it backwards,
  so a key one direction knows about and the other forgets cannot exist. Keys are `Owned` (the ProgID
  tree, the CLSID — deleted with their subtrees) or `Shared` (a value inside `.md`, which exists
  whether or not this editor does; removing the key would take somebody else's association with it).
  The tests assert the property that matters — **nothing in the footprint leaves `HKEY_CURRENT_USER`**.

  **It does not become the default handler, and cannot.** Windows 8 onwards protects the user's
  choice with a hash in `UserChoice` that no program can produce, so writing the `.md` default either
  does nothing or fights the user. What registration does is publish the ProgID and add it to
  `OpenWithProgids`; making it the default is a click in Open With → Always. `.json`, `.yaml` and
  `.yml` are deliberately **not** claimed — highlighting a format is not a reason to seize it.

  **`-Embedding` is filtered before paths are parsed.** COM adds it when *it* started the process
  rather than a person, and treating it as a path opens a tab named after it on every Explorer launch.

  **This is the least CI-able thing in the repo.** The registry footprint, the flag parsing and the
  twenty-file open are unit-tested; COM activation is not testable at all without a shell, and was
  driven by hand through `CoCreateInstance` + a hand-built `CF_HDROP` — which is what Explorer does.
- **Undo groups break on caret movement, not on a timer.** Deliberate — see `crates/document/src/history.rs`.
  It means a long pause mid-word still undoes as one step, which is the one place the behaviour
  differs from editors that use a clock.
- **Four settings, all reachable from the View menu.** `%APPDATA%\SimpleMarkdownEditor\config.json`
  holds `line_numbers`, `chrome`, `zoom` and — since MDEDIT-78 — `theme`. Read at startup and written
  back on the idle timer when one changes — see the settings entry above for the unknown-key rule that makes writing safe.
  **No setting may name a path** — see `crates/app/src/config.rs` for why that is a rule rather than
  a caution. JSON because `serde_json` was already here; the lack of comments is the known cost, and
  TOML is the thing to revisit if this grows past a handful of keys — preserving unknown keys removed
  the strongest reason a hand-editor would object to JSON.
- **A line number counts source lines, not rows.** A wrapped paragraph is numbered once, at its first
  row, which is what every editor does. The gutter is sized for the largest number the document can
  show, with a two-digit floor, so it does not visibly resize as you pass line 9.
- **Nothing sniffs a file to decide its encoding.** `load` honours a BOM and otherwise assumes UTF-8,
  and a windows-1252 file therefore fails to open — deliberately, because charset guessing is
  heuristic by nature and guessing wrong rewrites somebody's document. MDEDIT-42 answers it by
  *asking* instead: see the encoding picker below.
- **`.json` files open highlighted and never concealed**, by extension only — a JSON file named
  `data.txt` opens as Markdown, deliberately. Content sniffing is the trust boundary the 2023
  Notepad++ CVEs were all in, so nothing reads a byte of a file to decide what it is. The fix when
  somebody wants one is a language picker, which is a UI decision rather than a sniffing one.
- **`.yaml` and `.yml` open highlighted too, from a hand-written lexer — and the parser decision is
  evidenced, so read `crates/yaml/src/lib.rs` before proposing a crate for it.** Measured 2026-07-31:
  `syntect` **fails MSRV 1.87** (`plist` and `time` need 1.88); `tree-sitter` passes but is a C
  parser at a file-parsing trust boundary, which is the CVE class this project's language choice
  exists to remove; and a real YAML parser (`saphyr-parser`) **failed 3 of 4** mid-edit documents —
  a half-typed flow sequence, inconsistent indentation, and a tab — which is what a file being
  written looks like.
- **The YAML lexer highlights; it does not parse.** It understands comments, document markers, keys,
  sequences, quoted scalars, anchors, aliases, tags, flow collections and block scalars including
  their indentation. It approximates a key as text before `:` at end-of-line or `: ` before a
  comment, which is wrong for the explicit `? key` form and for a colon-space inside a flow scalar.
  Both mis-*colour* rather than mis-*cover*, and tiling is property-tested either way.
- **JSON and YAML re-parse a region, like Markdown — but a keystroke in them still costs more than
  one in Markdown, and the parse is not why.** MDEDIT-36 made both languages restart at a block
  boundary: JSON is one container deep there and YAML is provably outside every block scalar, and
  `Document::scope_for` already snapped every region to one. That halved the document-level cost of
  an edit on a 357 KB JSON, from ~3.7 ms of lexing to 1.96 ms end to end — **and moved the app's
  keystroke latency not at all**, p50 10.03 ms whole against 10.24 scoped in a same-session A/B.
  The remaining ~8 ms is elsewhere; MDEDIT-37 carries finding it, with segment density as the
  suspect (JSON 140,009 segments to Markdown's 19,410).
- **There is a language boundary, and one language behind it.** MDEDIT-29 put Markdown behind
  `mdedit_language::Language` so JSON and YAML can follow, and changed nothing observable — the whole
  test suite passed unmodified, which was the acceptance criterion. A language answers three things:
  its spans, its block extents, and whether its syntax is ever concealed. That last one is the
  rendering model in a single bool: Markdown hides its syntax because the syntax is scaffolding for
  prose, and JSON must not because its braces *are* the document. Nothing but Markdown implements it
  yet, so the trait has no correctness oracle — the same position `mdedit-text` is in.
- **A language that cannot say where to restart gets a whole-document parse.** `analyze_region`
  returns `Option`, and `None` means "no safe boundary here". Correct and merely slower, which is the
  right trade — JSON nests without limit, so an edit deep inside an object may have no boundary short
  of the top.
- **Blocks are extents, not a tree.** `Document::blocks` is a flat list with a depth on each entry.
  Nothing walks parent or child links because nothing has needed to. `block_at` **binary-searches**
  rather than scanning — MDEDIT-37 found it was the largest single cost in drawing a frame, because
  the renderer asks it once per styled run: 4,001 blocks times thirty-six runs times twenty visible
  chunks is 2.9 million comparisons a frame for a 357 KB JSON. Fixing it took `build_runs_in` on one
  JSON chunk from 0.064 ms to 0.005.
- **No perf assertions.** The bench in `crates/markers/benches` measures; nothing gates a regression.
- **CI does not run the app**, only the headless crates. There is no way to open a window on a CI
  runner, so rendering is verified by hand on Windows.

## Stack

Decided in [`docs/research/02-gui-toolkits.md`](docs/research/02-gui-toolkits.md). No GUI toolkit —
none of them has a rich-text editing widget, so the text view is ours regardless.

```
Window/input      winit + muda (native menus) + rfd (dialogs)
Renderer          wgpu, hand-written quad + glyph pipeline    [NOT vello]
Text layout       parley + harfrust + fontique + skrifa       [layout only, NOT parley's editor]
Buffer            ropey 1.x                                   [pin 1.x; 2.0 has been beta ~21 months]
Word boundaries   unicode-segmentation                        [UAX #29; one definition of "word"]
Markdown parse    pulldown-cmark                              [full re-parse; no incremental]
Code fences       syntect + two-face, lazy, per-block cached
Accessibility     accesskit
```

Rejected, with evidence, so nobody re-proposes them: **Slint** (plain-text-only `TextEdit`, rich
text explicitly display-only, unusable past ~5k lines, broken text rendering at fractional DPI),
**Tauri/webview** (webview typing latency ~20–31 ms vs a ≤3 ms target; open CJK IME freeze on
Windows), **GPUI** (unresolved GPL-contamination issue against its Apache-2.0 claim; publicly
deprioritized by Zed), **Xilem/Vello** (alpha; Vello mandates compute shaders), **iced** (no
per-run font size — no variable heading sizes), **tree-sitter-md** (self-disclaims correctness;
open stack-corrupting overflow in `serialize()`).

## Build & test

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo nextest run --all-features   # what CI runs; needs `cargo install cargo-nextest --locked`
cargo test --all-features --doc    # nextest does not run doctests, so this is the other half
cargo doc --no-deps --all-features # broken intra-doc links are denied, and CI runs this
cargo deny check                   # licences + advisories
cargo bench -p mdedit-markers      # segmentation cost against the keystroke budget
cargo bench -p mdedit-document     # what an undo group and a transaction cost against typing
```

`cargo test --all-features` still works and is fine for a quick loop. **CI runs nextest** (MDEDIT-111),
which puts every test in its own process — the structural answer to the MDEDIT-67 class, where two
threads of one process reading the Windows clipboard aborted a whole test binary. There are **no
retries** configured and that is deliberate; see `.config/nextest.toml`.

CI runs fmt, clippy, tests, doctests and docs on both Linux and Windows. Check the MSRV before pushing —
`cargo +1.87.0 check --all-features` — because the local toolchain is newer and will happily accept
things the floor rejects. Let-chains are the trap: they stabilised in 1.88.

Check the Linux build too — `cargo check --target x86_64-unknown-linux-gnu --all-features`. Windows
is the only platform anything runs on, so it is easy to add a dependency or a `cfg` that only
compiles here. `rfd` needing GTK headers and a `cfg`-gated enum going dead both got through to CI
that way. `cargo deny` and the MSRV check
pinned to `rust-version` (1.87, raised from 1.85 by wgpu 30) run on Linux only — neither is platform-sensitive. The bench is not a CI
job; it is run by hand when touching segmentation.

After CI passes on `main`, `.github/workflows/project-status.yml` measures unit coverage with
`cargo llvm-cov` and pushes it to the status board. `scripts/build-status.sh <sha>` builds both
`status.json` and the lcov bundle from one measurement, and can be run locally to see the numbers.
The figure is the Linux one — the Windows-gated code has no regions to measure there.

**Fuzzing is a separate workflow and a separate workspace** (MDEDIT-111). `fuzz/` holds four targets
— the marker locator against its tiling gate, JSON's validator, JSON's formatter against the
`meaningful_bytes` oracle it shares with the property test, and the YAML lexer. It is **excluded from
the workspace on purpose**: `libfuzzer-sys` builds a C++ runtime and is Unix-only, so a Windows
`cargo check` would try to build it and fail. The four crates it covers are platform-agnostic, which
is what makes Linux-only the right place for it rather than a compromise.

```bash
cargo +nightly fuzz run markers_segment -- -max_total_time=60   # nightly is cargo-fuzz's requirement
```

Five minutes a target on a PR, half an hour on the Sunday schedule, corpus cached between runs and
the crashing input uploaded as an artifact. AddressSanitizer comes with it, which is the only place
this project gets one: rustc has no sanitizer support on Windows.

**cargo-mutants runs on the diff, and reports rather than blocks** (MDEDIT-111). It changes a `<` to
a `<=`, deletes a line, replaces a return value, and says what the tests still let through — the
mechanical version of the second reviewer a solo project does not have, and the class of finding that
produced MDEDIT-110. `--in-diff` keeps it to minutes; a whole-workspace run is hours and would never
be read. Linux, `continue-on-error`, `.cargo/mutants.toml` for what is excluded and why.

```bash
cargo mutants --in-diff <(git diff origin/main...HEAD)   # needs `cargo install cargo-mutants`
```

A **surviving mutant is a question, not a defect**: sometimes the right answer is a better assertion,
and sometimes it is that the mutated line genuinely does not matter. Promote the job to blocking only
if it earns it.

`cargo test --release -- --ignored` is reserved for the perf assertions described under
**Performance** below. None exist yet, and they will not run in CI: GitHub runners are virtualised
and on AC power, which is the condition the research says is least informative. They belong in a
local harness whose numbers go in the PR body.

Line endings are pinned to LF by `.gitattributes`, because `rustfmt.toml` sets
`newline_style = "Unix"` and Windows checkouts default to `core.autocrlf=true`.

## Rust, beyond the baseline

`standards/rust.md` owns the general rules. Only these are project decisions:

- **No `async` in the editor core.** The UI thread is a synchronous frame loop; I/O goes to a worker
  thread. Adding a runtime to the core needs a design decision on the ticket.
- **`unsafe` anywhere outside a platform crate is a review block**, not a discussion — stricter than
  the baseline's `deny` + scoped-allow. **Three** crates qualify: `mdedit-text-dwrite` for
  DirectWrite, `mdedit-menu-win` for the call that attaches a menu bar to a window, and
  `mdedit-shell-win` for COM class registration and `IDropTarget`. The third was MDEDIT-61, and its
  reason is on the ticket rather than in a `SAFETY` comment, which is the rule: shell integration is
  unsafe Win32 by nature and is neither menus nor text. A **fourth** needs the same — a reason on a
  ticket, not a `SAFETY` comment in `mdedit-app`.

  **`mdedit-menu-win` is where a stray Win32 call goes**, despite its name, and that is deliberate:
  *one Win32 call is not a reason to mint a crate.* `GetDoubleClickTime`, `AttachConsole` and
  MDEDIT-70's `LoadImageW`/`WM_SETICON` all live there for that reason. The line is size, not
  subject — a *body* of platform work with its own decisions (COM, DirectWrite) earns a crate; a
  single documented call does not.
- **Newtype every offset**: `ByteOffset`, `CharOffset`, `LineIndex`, `BlockId`, `LayoutPos`. This
  codebase carries at least four coordinate spaces at once — source bytes, rope chars, laid-out glyph
  positions, and block indices — and silently mixing two `usize`s is the likeliest bug class in the
  whole project. They live in `mdedit-offsets`, which depends on nothing: the text layer needs
  `ByteOffset` to back an `ITextStoreACP` and must not reach a Markdown parser to get it.
- **A byte range is a `ByteSpan`, not a `Range`.** `Range<ByteOffset>` cannot index a `str` — the
  orphan rule bars `impl Index<Range<ByteOffset>> for str` — so spans carry their own type and slice
  via `ByteSpan::slice`. `Range<LineIndex>` is fine, because nothing is indexed by a line range.
  Converting raw offsets in or out (`ByteSpan::from`, `ByteSpan::range`) is deliberate and confined
  to boundaries: `pulldown-cmark` event ranges, `ropey`'s character indices, and test fixtures.

## Project-specific rules

**The byte-preservation invariant — the one rule that outranks everything:**

- **The buffer is the source of truth. Never re-serialize the document.** Opening a file and saving
  it must produce an identical byte sequence. MarkText's six-year-old
  [#2189](https://github.com/marktext/marktext/issues/2189) is what happens if this slips.
- Inline content is **never** serialized — it is decorated in place. Emphasis token (`*` vs `_`) and
  hard-break style are lost by every Rust Markdown parser; we sidestep that by not rewriting those
  bytes.
- Only the block under the cursor may be re-serialized, and only on exit, spliced back at its span.
- A diff that introduces a whole-document serializer needs an explicit design decision on the ticket.
- **That decision has been made once, on MDEDIT-48, and its scope is exactly Tools → Format Document
  for JSON.** The invariant forbids the editor rewriting bytes *the user did not ask it to rewrite*;
  it does not forbid an explicit, undoable command whose entire purpose is to rewrite them. The line
  is between "the editor reformatted my file" — forbidden, permanently — and "I asked it to".
  Everything on the far side of that line still holds: it is one `replace` inside a transaction so
  **one Ctrl+Z restores every byte**, it is reachable only from the menu, and **it is never reached
  from a save or from anything on a timer**. Citing MDEDIT-48 to justify a second serializer is not
  what that decision says.

**Marker locator (`crate::markdown::markers`):**

- No Rust Markdown parser emits delimiter nodes, so we compute them. This is the highest-risk
  component — see [`docs/research/03-architecture.md`](docs/research/03-architecture.md).
- **Property-tested against the CommonMark spec suite with one invariant: concatenating all marker
  and content spans, in order, reproduces the source byte-for-byte.** If that test is red, nothing
  else in the editor is trustworthy. It is the gate, not a nicety.
- Known-wrong upstream spans to work around: nested emphasis, escapes, collapsed links
  (`pulldown-cmark#1077`), image titles with no recoverable offset (`#441`), fence info strings
  (`#271`).

**Rendering & virtualization:**

- **Never materialize the whole document.** Render visible blocks only. `file open → first paint`
  is O(1): mmap plus viewport parse. Every large-file failure in the competition comes from
  violating this.
- **A tab is a lightweight handle until it becomes visible.** Session restore persists handles, not
  content. Copy Obsidian's `DeferredView` model — it is the only implementation in this category.
- Make the **renderer** incremental, not the parser. Diff the block list, patch changed blocks.
- **The layout cache is keyed by content, never by position.** `BlockId` is an index into one
  analysis: insert a character in the first paragraph and every block after it is renumbered, so a
  cache keyed on it would serve the wrong layout for the whole document below an edit. The key is a
  hash of the block's bytes plus its kind and the wrapping width — the same bytes lay out the same
  way wherever they are. A block holding the caret is laid out fresh and never cached, because what
  it draws depends on where in it the caret sits, and that is not in the key.
- Tables and code fences are **model-backed block widgets**, not decorations. Pure decoration over a
  linear character stream cannot do 2D layout; Obsidian's forums are a five-year record of that
  failure.

**Text layer:**

- Text layout sits behind a narrow internal trait with **two implementations: DirectWrite (Windows)
  and parley (portable).** Not a precaution — **DirectWrite is required on Windows today.** MDEDIT-5
  confirmed on real hardware that fontique resolves `script = Hani` to a single fallback family and
  stops, so Simplified-Chinese-only characters render as tofu even though SimSun is installed
  ([parley#644](https://github.com/linebender/parley/issues/644), open, cause is architectural).
  A Markdown editor that cannot render 汉 is not shippable. The trait also de-risks parley's release
  churn (five breaking releases in seven months, no 1.0) and gives a correctness oracle.
- **Do not use parley for text layout on Windows** until `spikes/font-fallback` passes there against a
  *released* fontique — passing against a patched branch does not count, since there is nothing to
  depend on. Its layout side remains the portable implementation for macOS and Linux.
  [parley#689](https://github.com/linebender/parley/pull/689) is the fix, and **it has been verified
  to work**: with its branch patched in, the probe passes all eight samples on Windows including the
  mixed-script line. Its Windows tier delegates to `IDWriteFontFallback::MapCharacters` — the same API
  we call directly — so the two paths converge rather than diverge.
  **The requirement stands only until that PR ships.** As of 2026-07-27 it is unmerged, conflicted
  with main and unapproved, so nothing is available to depend on. When it lands in a released
  fontique, re-run the probe against the release and, if green, parley becomes viable on Windows and
  this restriction lifts.
- Pin parley to an exact version. Wrap it; don't leak its types into the editor core.
- IME is IMM32 via winit for v1, but **the buffer sits behind a trait that can later back an
  `ITextStoreACP`** — retrofitting TSF's offsets and lock protocol onto a buffer not designed for it
  is expensive.

**Windows integration:**

- **Opening N files from Explorer must reuse one process.** Register a **DropTarget verb**
  (CLSID + `LocalServer32`), not a command-line verb — command-line verbs default to
  `MultiSelectModel=Document`, which spawns one process per file and silently drops the verb past
  15 items. Notepad++'s shell extension gets this wrong and has since 2023. **Done on MDEDIT-61**;
  see `mdedit-shell-win` and the entry in the repo state above.
- **Declare Per-Monitor DPI v2 in the manifest**, via `embed-manifest` — winit deliberately won't.
  Re-run glyph rasterisation and hit-testing on `WM_DPICHANGED`, and use the suggested rectangle.
- Read the dark-mode setting from the **registry**, not `ShouldAppsUseDarkMode()` (unreliable from
  1903+). Refresh on `WM_SETTINGCHANGE` / `ImmersiveColorSet`.
- **Per-user install. Never elevate.** Elevation breaks drag-drop from a non-elevated Explorer, and
  the documented mitigation is one Microsoft says isn't safe.

**Security (this project's specific threat model):**

- **Any auto-updater verifies signatures, pinned.** Notepad++ shipped an unverified updater, got its
  host compromised for ~6 months, and landed on CISA KEV with a nation-state actor delivering Cobalt
  Strike through it. This is the single highest-consequence design decision in the project.
- **Treat our own config and session files as untrusted input.** Never route a config string into a
  process launch; normalise every path before a prefix check. Notepad++ shipped three consecutive
  bypasses in 2026 by patching symptoms instead of the boundary.
- File parsing is a trust boundary. The 2023 Notepad++ CVEs were all heap overflows in hand-rolled
  encoding detection — the class Rust removes, provided we don't reintroduce it in `unsafe`.

**Performance ([`docs/research/06-performance-targets.md`](docs/research/06-performance-targets.md)):**

- Targets are commitments, not aspirations: cold start ≤100 ms, keystroke→pixel p50 ≤3 ms,
  **p99 ≤8.3 ms**, file open → first paint ≤50 ms at any size. Baseline RSS is now split into a
  floor we do not control and a budget we do — MDEDIT-19 measured an empty wgpu window at 124 MB,
  so the old ≤50 MB total was unreachable. See `docs/research/06-performance-targets.md`.
- **Measure p99 and measure on battery.** Every published editor benchmark is a mean on AC power,
  which is the least informative condition — Emacs goes 5.3 ms → 50.5 ms on power-save.
- **Two keystroke numbers, and the second one has a floor.** The harness reports *keystroke → quads
  ready*, which is what this process spends, and *keystroke → present*, which adds the wait for
  vblank. Under `PresentMode::Fifo` the second cannot go below a refresh interval however fast the
  frame was, so once a frame is cheap it stops saying anything about our own work. Compare builds
  on the first; use the second to confirm frames are still being presented.
- A change that regresses a target needs a number in the PR body, not an assurance.

## High-signal findings to watch for in review

- **Whole-document serialization** sneaking in — breaks byte preservation.
- **Marker-locator property test weakened or skipped** to make a case pass.
- **A block materialized outside the viewport** — reintroduces the large-file freeze.
- **parley or DirectWrite types leaking past the text trait.**
- `unwrap()` / `expect()` on a path reachable from user input or file contents.
- `unsafe` outside the platform layer, or without a `// SAFETY:` comment.
- Offsets passed as bare `usize` between coordinate spaces.
- An updater or plugin path that loads code without signature verification.
- `async` creeping into the editor core.

## Configs

Vendored from the baseline tag `v1.2.0`:

| Tool | Project file | Baseline source |
|------|--------------|-----------------|
| rustfmt | `rustfmt.toml` | `configs/rustfmt.toml` |
| clippy | `clippy.toml` + `[workspace.lints]` in `Cargo.toml` | `configs/clippy.toml` |
| cargo-deny | `deny.toml` | `configs/deny.toml` |

`clippy.toml` tunes lints but cannot enable them — the lint set lives in `[workspace.lints]`, copied
from the comment block at the foot of the baseline config. Set `msrv` to match `rust-version`.

Layer enforcement needs no config: workspace crate `[dependencies]` are the allow-list and Cargo
rejects cyclic crate deps at build time (`standards/architecture.md`).

Bump the pinned tag deliberately; re-vendor the configs when it changes.
