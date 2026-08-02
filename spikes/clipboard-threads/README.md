# clipboard-threads — MDEDIT-67

Reading the Windows clipboard from more than one thread of a single process corrupts the heap. This
probe is the measurement that says so, and the thing to re-run when `arboard` is bumped.

```
cargo run --release        # the whole matrix, as child processes
cargo run -- get 2 200     # one case, in this process
```

## What it found (2026-08-02, arboard 3.6.1, clipboard-win 5.4.1, Windows 11 26200)

| mode | threads | rounds | aborted |
| --- | --- | --- | --- |
| `new` — construct and drop | 9 | 200 | 0/8 |
| `get` — read the text | 1 | 2000 | 0/8 |
| `get` | 2 | 200 | 8/8 |
| `get` | 9 | 200 | 8/8 |
| `mixed` — one reader, eight writers | 9 | 200 | 4–6/8 |

Every abort is `0xC0000374`, `STATUS_HEAP_CORRUPTION`.

The `mixed` figure is the only one that moves between runs — 6/8 in the debug build, 4/8 in release —
because it needs one particular thread to be interrupted rather than any of nine. The rest are 0 or 8
every time.

Three things fall out of that. Reading is the operation at fault — constructing a clipboard opens
nothing, and writing is clean at nine threads. **Two threads are enough**, so this is not a rare
interleaving that needs a loaded machine. And `mixed` aborting is the decisive one: a *single* reader
is corrupted by other threads that only ever write, so the fault is not two readers racing each other
over a shared buffer.

## Why

`arboard`'s Windows read path ends in `clipboard_win::raw::get_string`, which takes a handle from
`GetClipboardData`, sizes a UTF-8 conversion buffer from it with one `WideCharToMultiByte` call,
reserves that much in the caller's `Vec`, converts into it with a second call, and then `set_len`s to
the size the first call reported.

That handle belongs to the clipboard and is only valid while **this process** has it open — and
`CloseClipboard` is process-wide, not per-`Clipboard`-value. So any other thread finishing its own
clipboard operation, read or write, closes the clipboard out from under a reader standing between its
sizing pass and its conversion pass. The `Vec` is then sized against one answer and written and
`set_len`'d against another.

`arboard` says as much, immediately above the call, and the assumption is what is wrong rather than
the code under it:

```rust
// XXX: ToC/ToU race conditions are not possible because we are the sole owners of the clipboard currently.
```

Sole owner of the *clipboard*, yes — Windows will not let a second process in. Sole owner within the
process, no.

## What the editor does about it

Nothing here can fix `arboard`, so the editor arranges never to be in that position:

- `mdedit_app::clipboard::System` is `!Send`, so the compiler refuses to move it to another thread.
  The editor is a synchronous frame loop on the UI thread, which is where clipboard work belongs.
- An `Editor` is built holding `clipboard::HoldNothing`, and `main` installs `System` over it — the
  same wiring the prompts, the file pickers and the recent-files list already use.

The second is what MDEDIT-67 actually was. While the real clipboard was the default, every test
binary read it, from as many threads as the test harness runs tests on; `context_menu` aborted 15 runs
in 20 because three of its tests ask `context_state()` whether there is anything to paste.
