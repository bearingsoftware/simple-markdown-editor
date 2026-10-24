# Files and sessions — saving, restoring and never losing a byte

Reading and writing without changing an unedited byte, the three sanctioned rewrites, crash recovery, the session file and the settings.

Decisions and known gaps, one entry each, mostly named for the ticket that produced them. Moved
out of `AGENTS.md` when it passed 100 KB — see [the index](README.md) for why and for the rest.

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

  **A machine shutdown or log-off is now an exit, through the loop rather than the window
  procedure** (MDEDIT-120). winit handles neither `WM_QUERYENDSESSION` nor `WM_ENDSESSION`, so a
  subclass in `mdedit-menu-win` (`watch_session_end`) does. It cannot flush: `run_app` owns
  `&mut Editor` and a window procedure has no way to reach it. So `WM_QUERYENDSESSION` posts
  `Wake::SessionEnding` and answers `TRUE` at once, and the loop calls `Editor::end_session` — snapshot
  the dirty buffers, write the session marked clean, set a shared flag. `WM_ENDSESSION` with `TRUE`
  means the process may be killed on return, so it pumps messages for up to three seconds until the
  flag is set, which is what lets winit dispatch the wake; winit 0.30.13 was read to confirm that a user
  event reaches `user_event` from a nested pump, because the runner is only "in use" while a handler is
  running. A cancelled shutdown posts `Wake::SessionContinues`, which writes the session unclean again,
  so a later crash is not reported as a clean exit. The shutdown is never blocked. The message
  handling cannot be tested, because nothing can deliver a session-end message; `end_session` is, in
  `session_end.rs`.
- **Settings are written back now, and unknown keys survive it.** MDEDIT-60 replaced "read once and
  never written back" — four features were stalled on it. The file is read into a `serde_json::Map`
  as well as the typed `Config`, and a write merges the changed fields into that map, so a key from a
  newer build or one added by hand passes through untouched. **That rule is load-bearing**: a
  typed-struct rewrite silently deletes the future. Writes are atomic, and happen on the idle timer
  when something changes rather than on exit — a run that crashes never reaches its exit path. The
  one thing it cannot preserve is a malformed file's contents, because there was nothing valid to
  preserve. Which chrome is showing, and the zoom, persist through it.
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
- **Nothing sniffs a file to decide its encoding.** `load` honours a BOM and otherwise assumes UTF-8,
  and a windows-1252 file therefore fails to open — deliberately, because charset guessing is
  heuristic by nature and guessing wrong rewrites somebody's document. MDEDIT-42 answers it by
  *asking* instead: see the encoding picker below.
