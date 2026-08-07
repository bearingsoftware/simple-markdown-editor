# Decisions and known gaps

One entry per decision, mostly named for the ticket that produced it. Together these are the record
of **why the editor is the way it is** — the trade that was made, the thing that was measured, and the
mistake that was made first.

## Why this is not in `AGENTS.md`

It was, and it reached **111 KB**. `AGENTS.md` is loaded into an agent's context at the start of every
session, so every byte of it is paid for on every task, whether or not the task goes anywhere near
the file pane or the YAML lexer. The root guide is now the rules you must not break and the map of
where things are; this directory is what you read when you are about to change one of them.

Nothing was rewritten in the move. Every entry is verbatim.

## What is here

| File | What it covers |
| --- | --- |
| [chrome.md](chrome.md) | The formatting bar and its lists, the tooltip, the find bar, the tab strip, the file pane, the gutter, the status bar, zoom, the settings and the recent-files list |
| [rendering.md](rendering.md) | The two palettes and their roles, concealment, per-kind marker colouring, the caret line, diagnostics, whitespace marks |
| [editing.md](editing.md) | What a word is, inline formatting, undo grouping |
| [files-and-sessions.md](files-and-sessions.md) | Byte-preserving load and save, the three sanctioned rewrites, encodings, line endings, crash recovery, the session file, external changes |
| [languages.md](languages.md) | The language trait, Markdown, JSON and YAML, validation, JSON formatting, blocks and scoped re-parsing |
| [platform.md](platform.md) | Windows: the window icon, menus, clipboard, click counting, IME, AltGr, the log file, COM shell integration |
| [text-layout.md](text-layout.md) | The text trait, why DirectWrite is required on Windows, underlines |
| [performance.md](performance.md) | Startup attribution, keystroke cost, chunking and virtualization, the two scroll axes, what is measured rather than gated |

## How to read an entry

Each one states what is true **now**, and usually what was true before and why it changed. Where a
defect was found by opening the editor rather than by a test, the entry says so — that is not
self-flagellation, it is the honest signal about which parts of this codebase the test suite cannot
see. There are more of those than anybody would like; `MDEDIT-123` is the ticket about closing the
gap.

## Adding one

Put it in the file whose subject it is, at the end, in the same voice: what changed, what it cost,
and what would go wrong if somebody undid it. If an entry needs more than a screen, it probably wants
a Confluence plan and a link rather than more prose here.

**Keep `AGENTS.md` under 20 KB.** That is the budget the split exists to protect, and the reason to
put a new decision here rather than there.
