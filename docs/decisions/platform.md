# Platform — Windows, and the three crates allowed to be unsafe

The window icon, the menus, the clipboard, click counting, IME, AltGr, the log file, and the COM shell integration that gets a whole Explorer selection into one process.

Decisions and known gaps, one entry each, mostly named for the ticket that produced them. Moved
out of `AGENTS.md` when it passed 100 KB — see [the index](README.md) for why and for the rest.

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
- **Clicks are counted in the editor, because winit does not count them.** The double-click interval
  is injected rather than hardcoded: Windows has a setting for it and somebody who lengthened it did
  so because the default fails them. `GetDoubleClickTime` is read in `mdedit-menu-win`, where the
  `unsafe` belongs. Being a parameter is also what makes click counting testable without sleeping.
  A run wraps at three, so a fourth click is a caret again.
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
