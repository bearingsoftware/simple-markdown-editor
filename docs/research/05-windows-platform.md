# 05 — Windows platform

Notepad++ as the incumbent, the shell integration a replacement must have, and
the 2026 signing and distribution landscape. Verified 2026-07-26.

## Part 1 — Notepad++

### Current state

| | |
|---|---|
| Latest | **8.9.7**, 2026-07-14 |
| Maintainer | **Don Ho**, since Nov 2003. Bus factor ~1 — 3,053 commits; next contributor has 330 |
| Activity | **506 commits in the last 52 weeks**; 28,820★, 2,921 open issues |
| Tech | C++17, pure Win32 + STL, Scintilla static lib + Lexilla lexers + Boost.Regex v1.90 |
| Licence | GPL-3.0-or-later |

**It is not declining.** Stack Overflow Developer Survey: **23.9% (2024) →
27.4% (2025)**, moving from 4th to **3rd place**, ahead of IntelliJ IDEA and Vim.
v8.9.6.4 alone has 6.9M GitHub downloads. Do not pitch this project as replacing
it.

Two architectural weaknesses that are free differentiation:

- **Not per-monitor-DPI aware.** Its
  [manifest](https://github.com/notepad-plus-plus/notepad-plus-plus/blob/master/PowerEditor/src/notepad%2B%2B.exe.manifest)
  declares `<dpiAwareness>system, unaware</dpiAwareness>`. Blurry on mixed-DPI
  multi-monitor setups.
- No `longPathAware` → still `MAX_PATH`-limited.

Dark mode cost them **5,040 lines** in `NppDarkMode.cpp` alone. Budget
accordingly — Windows will not theme scrollbars, menus, common dialogs or message
boxes for you.

### Security — the concern is substantially justified, for one specific reason

#### Tier 1: real, exploited in the wild, CISA KEV

**CVE-2025-15556** — the WinGUp updater performed **no cryptographic verification
of update metadata or installers** (CWE-494). CVSS v4 7.7.

- **[Added to CISA KEV 2026-02-12](https://www.cisa.gov/news-events/alerts/2026/02/12/cisa-adds-four-known-exploited-vulnerabilities-catalog)**,
  federal patch deadline 2026-03-05.
- Exploited by a China-nexus group to deliver **Cobalt Strike Beacon** and a
  previously undocumented backdoor.
- **Root cause was a hosting-provider compromise, not a code bug.** Per the
  [official incident post](https://notepad-plus-plus.org/news/hijacked-incident-info-update/):
  attackers had access from **June 2025**; the host's server was compromised until
  **2025-09-02**; stolen credentials retained until **2025-12-02**. Detection was
  early December 2025. **~6 months of selective, targeted update poisoning.**
- Victims across cloud hosting, energy, finance, government and manufacturing in
  Vietnam, El Salvador, Australia, the Philippines, the US, South America and
  Europe.

Fixes: 8.8.9 added certificate + signature verification. 8.9.2 added a "double
lock" — signed installers verified against GitHub *plus* XMLDSig-signed update
XML; removed `libcurl.dll` (DLL side-loading); restricted plugin-management
execution to WinGUp-cert-signed binaries.

> **The single most important design lesson: an auto-updater without signature
> pinning is the #1 way a desktop editor becomes malware.**

*Attribution note: Wikipedia says APT31; Rapid7/CISA reporting says Lotus
Blossom. Sources disagree.*

#### Tier 2: real but pedestrian, patched promptly

| CVE | Sev | What | Fixed |
|---|---|---|---|
| CVE-2023-40031 | 7.8 | Heap write overflow in `Utf8_16_Read::convert` — wrong UTF-16→UTF-8 size assumption on odd chunk boundaries. Open a crafted file → RCE | 8.5.7 |
| CVE-2023-40036/40164/40166 | Low–Med | Three more read overflows in encoding detection | 8.5.7 |
| CVE-2025-49144 | 7.3 | NSIS installer called `regsvr32`/`rundll32` without absolute paths → binary planting → SYSTEM | 8.8.2 |
| CVE-2026-25926 | 7.3 | App launched `explorer /select,"%s"` without absolute path | 8.9.2 |
| CVE-2026-57233 | 8.1 | **Zip Slip** in WinGUp plugin extraction — `../mimeTools/mimeTools.dll` overwrites sibling plugin DLLs | 8.9.7 |

**All four 2023 bugs were the same class: parsing untrusted file bytes in
hand-rolled C++.** That is precisely the class Rust eliminates — a legitimate
design justification for this project.

#### Tier 3: contested trust boundary (the 2026 surge)

18 GitHub security advisories total: 4 in 2023, 2 in 2025, **12 in 2026**. The
spike is a researcher swarm following the supply-chain publicity, and most of it
is one theme: **Notepad++ reads its own config XML from `%APPDATA%` and passes
values straight to the OS.**

- CVE-2026-48800 (7.8) — `<Command>` in `shortcuts.xml` goes to `ShellExecute`
  unvalidated. PoC is literally
  `<Command name="System Update Check" ...>calc.exe</Command>`, which then appears
  as a legit-looking Run-menu entry. **Persistence mechanism.**
- CVE-2026-48778 (7.8) — `commandLineInterpreter` in `config.xml` substitutes any
  executable for cmd.
- Then CVE-2026-52884 ("CVE-2026-48800 Bypass"), CVE-2026-52885 (TOCTOU: HMAC
  checks disk, executes from memory), and a further macro HMAC bypass in July.
  **Notepad++ bolted on an HMAC and researchers walked around it twice.**
- CVE-2026-52886 — `session.xml` `backupFilePath` validated with a raw
  `starts_with` string compare, no path normalisation, so
  `C:\backup\..\..\Windows\...\hosts` passes. Arbitrary file read into a tab.

These all require the attacker to already write to `%APPDATA%` — a weak
precondition by classic Win32 standards. **But the design lesson is directly
applicable: treat your own config files as untrusted input.** Never round-trip a
config string into `ShellExecute`/`CreateProcess`; normalise every path before a
prefix check. Notepad++ shipped three consecutive bypasses because it patched
symptoms instead of the boundary.

#### Tier 4: overstated

- **CVE-2025-56383** "DLL hijacking in v8.8.3", initially scored 8.4. NVD amended
  the record within 5 days to note it is **disputed by multiple parties** —
  it only applies if you install into a user-writable directory. **LevelBlue
  SpiderLabs, who published it, agreed with the dispute.** Treat as ~0.0.
- **Domain confusion is typosquatting, not a code problem.** Official domain is
  `notepad-plus-plus.org`. Malicious lookalikes: `notepads-plus-plus.org` (Vidar
  Stealer, padded to 700 MB to evade analysis), `nottepad-plus-plus.org`,
  `notepad-plus-plus-mac.org`.

#### Real unaddressed gap: plugins

Plugins Admin verifies the **SHA-256 of the plugin zip** against a signed list.
But **individual plugin DLLs are not signature-verified and there is no security
audit** — the submission bar is *"please test it locally before doing your PR."*
~140 plugins, loaded from `Program Files\Notepad++\plugins\`.

### Killer features, ranked by citation frequency

1. **Session persistence + never-saved unsaved tabs** — the single most-cited
   thing. See below; it is *the* feature.
2. **Find in Files / regex** — Boost.Regex, recursive, wildcard filters with `!`
   exclusion, **Find All in All Opened Documents** into one results pane, and the
   Mark tab.
3. **Large file handling** — reputation exceeds reality, see below.
4. **Portable, no install** — `doLocalConf.xml` (a zero-byte sentinel next to the
   exe) switches everything to local. Repeatedly cited as *the* workaround for
   locked-down corporate machines.
5. **Instant startup.** Used as a scratchpad, not an IDE.
6. Syntax highlighting + user-defined languages.
7. **Macros** — record/playback, "run until caret reaches end of file". Only
   editable by hand-editing `shortcuts.xml`.
8. **Column / block edit** — `Alt+drag`, plus a Column Editor with incrementing
   numbers (initial value, step, repeat-N, leading zeros, dec/hex/octal/binary).
9. Line ops — sort, mark, delete-marked.
10. Plugins — Compare, XML Tools, JSON Viewer, NppExec, **HexEditor** (hex is a
    *plugin*, not built in).
11. **Encoding conversion** — under-cited but a genuine differentiator. The
    **"Encode in" vs "Convert to"** distinction is what everyone gets wrong: upper
    menu entries *reinterpret* existing bytes; `Convert to…` *rewrites* bytes
    preserving glyphs. Detection ladder: BOM → XML/HTML declaration → byte-pattern
    heuristics → ASCII/UTF-8 test → system locale.
12. **Multi-cursor** — Ctrl+Click, default since v8.6. Stream selections only,
    **not composable with column mode**. Weaker than Sublime's.

#### Large-file reality check

Hard ceiling is **2 GB** (Scintilla buffer requests exceed `INT_MAX`). In practice
memory-bound well below: community reports show 318/423/438 MB succeeding but
350 MB and 600 MB failing on other machines. Rule of thumb: **~4× file size in
RAM**, more if lexed.

Notepad++ mitigates with a **Large File Restriction** that disables styling, word
wrap, autocomplete, brace match and clickable URLs above a threshold. Critically,
`Buffer.cpp` **disables session snapshots, periodic backup AND backup-on-save
entirely for large files.** So "opens huge logs" and "never loses your work" are
mutually exclusive in Notepad++.

**EmEditor is the actual large-file editor** — claims 16 TB / 1,099 billion lines.

### Session and backup — the precise spec to match

Authoritative source is a comment block in
[`Buffer.cpp`](https://github.com/notepad-plus-plus/notepad-plus-plus/blob/master/PowerEditor/src/ScintillaComponent/Buffer.cpp)
around line 1219:

> **"Notepad++ quits without asking for saving unsaved file. It restores all the
> unsaved files and document as the states they left."**

Defaults, from
[`Parameters.h`](https://github.com/notepad-plus-plus/notepad-plus-plus/blob/master/PowerEditor/src/Parameters.h):

```cpp
bool   _isSnapshotMode        = true;    // ON by default
size_t _snapshotBackupTiming  = 7000;    // 7 seconds

bool isSnapshotMode() const {
    return _isSnapshotMode && _rememberLastSession && !_isCmdlineNosessionActivated;
}
```

Note the conjunction — **snapshot mode silently requires "Remember current
session" too**, and `-nosession` kills it.

**Algorithm, existing file `c:\tmp\foo.h`:**

1. On **first modification**, create `backup\foo.h@<CREATION_TIMESTAMP>` and bind
   it to the Buffer.
2. **Every 7 s**, if a dirty flag was set by a modification notification, write
   the backup and clear the flag. Dirty-flag gated — no write if untouched.
3. Also syncs **before switching tabs** and **on close/exit**.
4. On **manual save or close**, the backup file is **deleted**.
5. `session.xml` tracks `backupFilePath` plus the original file's last-modified
   `FILETIME`.
6. **Next launch:** load the backup if present (else the real file); mark the tab
   **dirty (red)**; compare the tracked timestamp against disk; if they differ,
   **prompt that the file changed externally and offer reload.**

Untitled `new 4`: backup is `backup\new 4@<TIMESTAMP>`; on restore the tab shows
`new 4` with a **red disk icon**.

Storage: `%AppData%\Notepad++\backup\` normally; the install dir in portable
mode; or `-settingsDir` if passed. `session.xml` sits alongside — and
**deliberately ignores the Cloud settings directory** because it stores absolute
paths.

`session.xml` `<File>` attributes: `filename`, `backupFilePath`,
`firstVisibleLine`, `xOffset`, `scrollWidth`, `startPos`, `endPos`, `selMode`,
`offset`, `wrapCount`, `lang`, `encoding`, `userReadOnly`,
`originalFileLastModifTimestamp(High)`, plus ~10 minimap attrs. Overwritten on
every clean exit.

Tab count: no documented or hard-coded limit; hundreds are routine.

#### Failure modes to design around

- **Kill -9 or power loss loses up to 7 seconds.** That is the whole guarantee.
- **`session.xml` can be emptied while backups survive** —
  [#12194](https://github.com/notepad-plus-plus/notepad-plus-plus/issues/12194),
  "rarely and at random", one occurrence correlated with a pending auto-update.
  Recovery needs hand-editing. **No maintainer response.**
  *Fix: make the backup directory self-describing so the session index is
  reconstructible, and write `session.xml` atomically (temp + rename).*
- **Multi-instance breaks it:** periodic backup works only for the first instance,
  and only the last instance closed saves settings.
- **Manual `File > Save Session…` silently drops unsaved/unnamed documents.**
- **Disabled entirely for large files.**

### Live alternatives

| Editor | Status | Stars | Tech | Missing vs Npp |
|---|---|---|---|---|
| **NotepadNext** | Alive, v0.14 (2026-04-27) | 14,402 | Qt6 + Scintilla, GPL-3 | Still 0.x. No plugin ecosystem, no macro parity. **Author is himself a top-15 Notepad++ contributor.** Closest successor |
| **CudaText** | Very alive, near-daily | 3,136 | Lazarus/FPC, Python plugins | Idiosyncratic UI, smaller ecosystem. Genuinely fast |
| **Notepadqq** | **Revived** — v2.1.1, 2026-07-03 | 2,322 | Qt, GPL-3 | Linux-first; weak Windows story. Was widely assumed dead |
| **Zed** | Windows shipped; v1.12.0 (2026-07-23) | 87,555 | Rust + GPU, GPL-3 | No macros, no column-editor-with-increment, weak encoding tooling. **But it is the reference implementation for everything in Part 2** |
| **Sublime Text** | Build 4200, 2025-05-21 — 14 months | — | C++, proprietary | Alive but slow-moving |
| **EmEditor** | v26.2.3, 2026-07-21 | — | Commercial | The large-file editor. Ships via MS Store MSIX |
| **Windows Notepad** | **Eating the low end fast** | — | UWP/RichEdit | Now has tabs, session restore, dark mode, spellcheck, AI rewrite, **and Markdown formatting**. ~1 GB cap. Still **no block selection** |

**Strategic read: the vacancy is not "a better Notepad++"** — NotepadNext and
CudaText already occupy that. The vacancy is a **memory-safe, PMv2-correct,
properly-signed, correctly-batching Windows editor with Notepad++'s
never-lose-a-buffer session model**. Notepad++'s weaknesses are all where Rust and
2026 Windows APIs are strongest.

---

## Part 2 — Windows engineering requirements

### Opening 50 files from Explorer into ONE process

**Highest-leverage integration item.**

Why Explorer spawns N processes: a command-line verb defaults to
`MultiSelectModel = Document`, meaning *"verbs that create a top-level window for
each item"* — one process per file, **and the verb silently disappears past 15
items**.

| Implementation | `Document` | `Player` |
|---|---|---|
| Legacy command line | **15 items** | 100 items |
| **COM (DropTarget / ExecuteCommand)** | 15 | **No limit** |

**The supported fix is the DropTarget verb** — register a CLSID with
`LocalServer32` instead of a command line:

```
[HKCU\Software\Classes\CLSID\{guid}\LocalServer32]  @="...\editor.exe"
[HKCU\Software\Classes\<ProgID>\shell\open\DropTarget]  "Clsid"="{guid}"
```

The shell then makes **one `IDropTarget::Drop` with all files**, and *"COM
launches your application if it is not already running but connects to a running
instance if one is present"* — batching **and** instance reuse, free. Handle the
`-embedding` argument.

**What Notepad++ does, and why not to copy it.** From
[`winmain.cpp`](https://github.com/notepad-plus-plus/notepad-plus-plus/blob/master/PowerEditor/src/winmain.cpp):

```cpp
::CreateMutex(NULL, false, L"nppInstance");          // :567
if ((!isMultiInst) && (!TheFirstOne)) {              // :733
    ...
    ::SendMessage(hNotepad_plus, WM_COPYDATA, ..., &fileNamesData);   // :785
}
```

plus `ChangeWindowMessageFilterEx(hwnd, WM_COPYDATA, MSGFLT_ALLOW)` so a
non-elevated Explorer can talk to an elevated Notepad++. That part is sound. But
their *shell extension* is broken:
[nppShell#36](https://github.com/notepad-plus-plus/nppShell/issues/36), opened
2023-06-14 and **still open** — *"NppShell launches N times Notepad++ if N files
are selected in Explorer."* And `WM_COPYDATA` handling produced its own CVE
(CVE-2026-48770, DoS crash).

**Recommended design:**
DropTarget verb as primary (no item limit, reuse for free)
→ `shell\open\command` with `MultiSelectModel=Player` as fallback
→ named mutex + hidden window + `WM_COPYDATA` as universal backstop.

`WM_COPYDATA` must be **sent, not posted**; no pointers in the payload; copy
immediately. **Do not build on DDE.**

### File associations

Write ProgIDs to **`HKCU\Software\Classes`** (per-user, no elevation) — `HKCR` is
a merged read-only view. Use **`OpenWithProgIds`**; `OpenWithList` is explicitly
*"Do not use."* Always call **`SHChangeNotify(SHCNE_ASSOCCHANGED)`** or the change
is invisible until reboot. On uninstall, **don't** remove the `.ext` default you
set.

**You cannot set yourself as default.** `IApplicationAssociationRegistration` is
documented: *"As of Windows 8, the only functionality of this interface that is
supported is `QueryCurrentDefault`."* `UserChoice` is hash-protected, and since
the **February 2024 cumulative update** a kernel driver **UCPD.sys** blocks writes
to it.

What you *can* do: register `Capabilities` + `RegisteredApplications` (with
`ApplicationDescription` — omit it and you don't appear at all), then deep-link
the user to `ms-settings:defaultapps`.

### Windows 11 modern context menu — requires package identity

Two mandatory halves: an **`IExplorerCommand` in-proc COM DLL** (*"supports
in-process activation only"* — cannot be an EXE server) and a **signed sparse
MSIX** granting package identity. Legacy `HKCR\*\shell` handlers still work but
land under "Show more options", and *"manual registry hacks that attempt to
forcefully place classic DLLs onto the modern menu will be automatically
sandboxed or ignored."*

[nppShell's `AppxManifest.xml`](https://github.com/notepad-plus-plus/nppShell/blob/master/Packaging/AppxManifest.xml)
is a complete worked reference:

- `uap10:AllowExternalContent=true`, `TargetDeviceFamily MinVersion 10.0.22000.0`
- `rescap:runFullTrust` + `rescap:unvirtualizedResources`
- `desktop4:Extension Category="windows.fileExplorerContextMenus"` →
  `desktop5:ItemType Type="*"` → `Verb Clsid="..."`
- `com:SurrogateServer` hosting the DLL, STA
- `AppListEntry="none"`, `uap10:TrustLevel="mediumIL"`,
  `RuntimeBehavior="win32App"`
- **`Publisher` must exactly equal your signing certificate's Subject**

Build: three DLLs (x86/x64/arm64) + `makeappx pack`, all signed, registered via
`regsvr32 /s`. **Zed does the identical thing from Rust**
([`explorer_command_injector`](https://github.com/zed-industries/zed/blob/main/crates/explorer_command_injector/AppxManifest.xml))
— proven feasible with the `windows` crate's `#[implement]` COM macros.

### Drag and drop

Use **`IDropTarget`** (`RegisterDragDrop`), not `WM_DROPFILES` — same object model
as the DropTarget verb. If you run elevated, UIPI blocks drops from non-elevated
Explorer; the mitigation (`ChangeWindowMessageFilterEx`) is one Microsoft says
*"isn't recommended as a safe solution."* **The real answer: per-user install,
never elevate.**

### Per-Monitor DPI v2

**Declare in the manifest, not at runtime.** Use
[`embed-manifest`](https://docs.rs/embed-manifest/latest/embed_manifest/manifest/enum.DpiAwareness.html)
with `DpiAwareness::PerMonitorV2` — winit deliberately does *not* do this for you
(*"the manifest must be in the .exe, not the .dll"*).

You get free non-client-area scaling; **all glyph rasterisation and hit-testing
must re-run on `WM_DPICHANGED`**. **Use the suggested rectangle** or you get
cursor drift and recursive DPI loops. Swap in `GetSystemMetricsForDpi`,
`AdjustWindowRectExForDpi`, `SystemParametersInfoForDpi`, `GetDpiForWindow`.

Beware: a cross-process `CreateWindow` or `SetParent` across awareness modes
**resets your whole process's DPI awareness**.

### Dark mode

`DWMWA_USE_IMMERSIVE_DARK_MODE = 20`. Microsoft documents it as Win11 22000+, but
real-world practice is build **19041** (Win10 2004) — Notepad++ gates on exactly
that and falls back to `SetWindowCompositionAttribute` below, skipping attribute
19 entirely. Since Windows 10 hit EOL **2025-10-14**, targeting ≥19041 and
ignoring 19 is defensible.

Undocumented uxtheme ordinals still work on all Win11: **135 =
`SetPreferredAppMode`**, 133 `AllowDarkModeForWindow`, 132 `ShouldAppsUseDarkMode`,
49 `OpenNcThemeData`, 136 `FlushMenuThemes`.

**Detect the theme from the registry, not the API** — Notepad++ carries the
comment *"`ShouldAppsUseDarkMode()` is not reliable from 1903+"*. Read
`HKCU\...\Themes\Personalize\AppsUseLightTheme` (0 = dark), refresh on
`WM_SETTINGCHANGE` with `lParam == "ImmersiveColorSet"`.

Mica/Acrylic: `DWMWA_SYSTEMBACKDROP_TYPE` (38) from build 22621. Title-bar colours:
`DWMWA_BORDER_COLOR` (34) / `CAPTION_COLOR` (35) / `TEXT_COLOR` (36), Win11 22000+.

**Budget realistically: the title bar is trivial; the ~5,000 lines of subclassing
for scrollbars, menus, common dialogs and message boxes is the actual work.**

### IME

Neither IMM32 nor TSF is deprecated, but they cover different ground. **winit is
IMM32-only** — verified from
[`winit-win32/src/ime.rs`](https://github.com/rust-windowing/winit/blob/master/winit-win32/src/ime.rs),
which references `ImmGetContext`, `ImmGetCompositionStringW`, `GCS_*`,
`CFS_POINT`, `ATTR_TARGET_*`, and **zero TSF interfaces**.

What that costs: Win+H voice typing produces nothing —
[wezterm#7791](https://github.com/wezterm/wezterm/issues/7791) documents that
voice typing delivers text via `ITextStoreACP::InsertTextAtSelection` and the
IMM32↔TSF bridge does not bridge it. Also lost: reconversion, handwriting.

Ship IMM32 for v1, but **put the text buffer behind a trait that can later back an
`ITextStoreACP`.** Separately: dead keys and AltGr are a `ToUnicodeEx` problem,
not an IME problem.

### Startup performance realities

Microsoft published the model —
[The Windows Prefetcher](https://learn.microsoft.com/en-us/archive/blogs/vancem/the-windows-prefetcher):

```
ColdStartupTimeMSec = WarmStartupTimeMsec + 4 * NumberOfReads + 20 * NumberMBytes
```

Constants are obsolete on NVMe; the seeks-vs-bytes tradeoff still holds.
Critically, the prefetcher *"issues ALL the file I/O specified in this file, at
once, and blocks the program until it completes"* — **no code of yours runs until
prefetch finishes, so a splash screen cannot appear sooner.**

Levers in rank order: cut warm-start CPU work → cut startup bytes → cut seeks.
For Rust: small import table, avoid DLLs with expensive `DllMain`, consider
`+crt-static` (trades against bytes — measure), delay-load anything off the
first-frame path.

#### Defender imposes a hard floor no optimisation beats

For executables from the Internet zone, if the cloud can't decide, *"Microsoft
Defender Antivirus **locks the file and uploads a copy to the cloud**"* — and the
**default block window is 10 seconds** (admin-extendable to 60). Block-at-first-
sight is on by default for consumers. **A brand-new unsigned release realistically
stalls ~10 s on first launch.**

ASR rules that will bite (not default on consumer, but common in managed setups;
**you cannot set exclusions — only the admin can**):

- `01443614-cd74-433a-b99e-2ecdc07bfc25` — *Block executable files unless they
  meet a prevalence, age, or trusted-list criterion*. The rule that blocks new
  low-prevalence apps.
- `b2b3f03d-…` — *Block untrusted and unsigned processes from USB*. **Kills the
  portable-from-USB story for unsigned binaries** — one of Notepad++'s most-cited
  features.
- `c1db55ab-…` — ransomware protection, *"also blocks files that don't yet have a
  positive reputation"*. An editor writing files across the filesystem is squarely
  in scope.

**MotW is leaky for ZIP distribution.** 7-Zip only propagates `Zone.Identifier` if
the user enabled *"Propagate Zone.Id stream"*. **Practical consequence: whether
your portable-zip users hit SmartScreen depends on which extractor they used.**
Document it, don't fight it.

---

## Part 3 — Code signing and distribution

### The CA/B baseline

**Code Signing Baseline Requirements v3.11.0**, effective 16 June 2026. §6.2.7.4.1,
verbatim:

> "Effective June 1, 2023, Subscriber Private Keys for Code Signing Certificates
> SHALL be protected per the following requirements… **in a Hardware Crypto Module
> with a unit design form factor certified as conforming to at least FIPS 140-2
> Level 2 or Common Criteria EAL 4+**"

Three permitted routes: subscriber-held hardware module; a cloud key-generation
solution where keys never leave the HSM boundary; or a Signing Service. **The
mandate is uniform — there is no EV/non-EV split.**

New in 2026: §6.3.2 — certificates issued on or after **2026-03-01** have max
validity **460 days** (down from 39 months). Multi-year purchases are now prepaid
subscriptions with ~annual reissuance.

*⚠️ SSL.com's marketing says 458 days; the CSBR text says 460. Trust the CSBR.*

### EV no longer bypasses SmartScreen

Microsoft, verbatim, on
[SmartScreen reputation for Windows app developers](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)
(2026-05-04):

> "**EV certificates no longer bypass SmartScreen.** Years ago, signing files with
> an Extended Validation (EV) code signing certificate would result in positive
> SmartScreen reputation by default, but this behavior no longer exists. EV
> certificates may matter for enterprise procurement, but they no longer impact
> SmartScreen behavior. **Paying a premium for EV solely to avoid SmartScreen
> warnings is no longer justified.**"

The companion page dates the change: *"That behavior was removed in 2024."*
Many CA marketing pages still imply otherwise.

**Do not budget for EV.** Its only remaining justification is kernel drivers and
enterprise procurement checkboxes.

### How SmartScreen actually works

Two signals: **publisher reputation** (is it signed, by a known cert?) and
**file hash reputation** (has this exact binary been downloaded cleanly?).

> "There is no exact threshold, but it can take **several weeks and hundreds of
> clean installs from a wide audience**."

> "There is no need (or mechanism) to manually submit a file for SmartScreen
> reputation review for consumer endpoints."

Unsigned is the worst case: *"reputation must build for each new version of your
files, starting with zero reputation. Reputation cannot transfer from previous
versions unless both were signed using the same publisher identity."* Self-signed
is treated identically.

**The only true zero-warning path is the Microsoft Store** — *"Apps published
through the Microsoft Store are re-signed by Microsoft and carry full reputation.
Users will never see a SmartScreen warning for a Store-installed app."*

**Smart App Control is the emerging harder gate** and matters for a native Rust
editor: it *"will block execution of unsigned files unless the file has a positive
reputation"*, and applies to **all executable files, not just downloaded ones.**

#### Operational rules that will bite a Rust project

- **Do not modify signed files.** If your build pipeline patches the binary
  (version stamping, installer bundling, packing) it must happen **before**
  signing.
- **Use one signing identity and never change it.** Every CA switch resets
  publisher reputation.
- **Do not sign potentially unwanted applications.** An editor that ships a shell,
  downloads extensions, or self-updates should avoid anything matching PUA
  criteria, or the *certificate* accrues negative reputation.

### Azure Artifact Signing (formerly Trusted Signing)

Rebranded. Canonical docs are now
[learn.microsoft.com/azure/artifact-signing/](https://learn.microsoft.com/en-us/azure/artifact-signing/).
CLI is `az artifact-signing`.

**Pricing** — the public pricing page renders placeholder dashes, so pulled live
from the Azure Retail Prices API 2026-07-26:

| SKU | Price | Quota |
|---|---|---|
| Basic Account | **$9.99/mo** | 5,000 signatures/mo |
| Premium Account | **$99.99/mo** | 100,000/mo |
| Signature Overage | **$0.005**/signature | — |

Requires pay-as-you-go or EA (**free/trial subscriptions rejected**) and is
**not pro-rated** — start it the month you ship.

**🚩 The blocker for a UK individual:**

> "Public Trust certificates are available to organizations in the United States,
> Canada, the European Union, the United Kingdom… **Individual developers must be
> located in the United States or Canada.**"

All three Microsoft pages agree on individuals = US/CA only. UK **organizations**
are eligible.

**The "3 years verifiable business history" rule appears to be gone.** It was a
public-preview throttle announced 2025-04-02, and it is absent from every current
doc (quickstart, FAQ, overview, certificate management, code-signing-options).
*⚠️ No document explicitly rescinding it was found — treat "removed" as
strongly-evidenced inference and confirm at enrolment.*

Individual enrolment is now a documented 26-step path: billing account type must
be "Individual", identity proofing via Microsoft Entra Verified ID with AU10TIX
(government photo ID + selfie/QR + proof of address ≤3 months old). Only your
name and city/state/country land on the certificate.

Mechanics worth knowing:

- **Certificates are renewed daily and valid for only 72 hours.**
- **Timestamping is therefore mandatory**, not optional —
  `http://timestamp.acs.microsoft.com`. *"if you fail to timestamp your signed
  file, the signature will expire when the certificate does just a few days
  later."*
- Already-signed, timestamped binaries survive account or profile deletion.
- **Revocation blast radius is unusually small** — a revocation *"affects only the
  code that was signed by using that certificate on the day that it was issued."*
- Pinning uses a custom EKU under `1.3.6.1.4.1.311.97.<unique>`, not a cert hash.
- **Never issues EV.** *"There's no plan to issue EV certificates in the future."*

### Verified vendor prices (2026-07-26)

**SSL.com** — IV is explicitly aimed at solo devs: *"independent developers
shipping tools, utilities, command-line apps"*, *"no business registration"*.

| Product | 1 yr | 3 yr |
|---|---|---|
| IV (individual) | $129.00/yr | $109.65/yr |
| OV (organization) | $129.00/yr | $109.65/yr |
| EV | $349.00/yr | $249.00/yr |

- YubiKey token: **+$379.00**
- eSigner cloud signing (avoids hardware entirely): **$20/mo** for 20
  signings/mo, $180/yr annual, unused credits roll over

*⚠️ Ambiguity: the OV/IV pages say certificates ship "installed on FIPS hardware
token" as standard *and* list YubiKey as a $379 add-on. Confirm with sales.*

**Certum**

| SKU | Price |
|---|---|
| **Open Source Code Signing in the Cloud** (SimplySign, no card/reader) | **€49.00** |
| Open Source Code Signing (card + reader set) | €69.00 — **listed out of stock 2026-07-26** |
| Standard Code Signing in the Cloud | €209.00 |
| EV Code Signing in the Cloud | €379.00 |

Open Source tier: 1-year validity, subject is *"Open Source Developer"* + your
name, no company needed. *⚠️ Certum does not publish its documentary eligibility
criteria — validate before budgeting.*

**Others:** DigiCert OV $44/mo (~$528/yr), EV $62/mo. Sectigo publishes only
*"starts at $536.25/yr on the five-year option."* GoGetSSL OV $290–409/yr.
SignMyCode, CheapSSLSecurity and K Software are all Cloudflare-blocked —
**treat any price for them as unverified.**

### Recommendation for a UK individual

1. **SignPath Foundation — $0**, OV-level, if the project is open source.
   Microsoft's own docs recommend it: *"offers free code signing for qualifying
   open-source projects."* *⚠️ signpath.io was unreachable during research —
   verify eligibility yourself. It's free; worth ten minutes.*
2. **Certum Open Source in the Cloud — €49/yr.** No hardware, no company, cert
   in your own name. Cheapest verified legitimate path.
3. **SSL.com IV + eSigner — $369 yr 1** if closed-source.
4. **Register a UK Ltd → Azure Artifact Signing at $120/yr** — cleanest CI/CD
   story, FIPS 140-2 L3, tiny revocation blast radius.

Ship unsigned only as an interim, and expect the ~10 s Defender stall plus weeks
of SmartScreen prompts.

### Distribution

| Format | Verdict |
|---|---|
| **Inno Setup 6.7.3** | **Best default.** Free commercially, single .exe, `PrivilegesRequired=lowest` for per-user, handles file assoc + PATH + ARP, can stage a sparse MSIX |
| **Portable ZIP** | Ship alongside. It is Helix's *only* Windows artifact |
| NSIS 3.12 | Fine; uglier scripting for equal capability |
| WiX/MSI | Only for enterprise deployability. **Licensing landmine** — WiX v7's Open Source Maintenance Fee obliges orgs over $10k revenue to sponsor. v4/v5 predate it; Alacritty pins `wix 4.0.5` for exactly this reason |
| MSIX (full) | **Actively hostile to an editor** — filesystem/registry virtualisation, PATH shims |
| **winget** | **Do this from day one.** No fee, **no signing requirement** |
| Microsoft Store | Registration is now **free**, but only via `https://storedeveloper.microsoft.com`. MSIX signing unconditional |

**winget specifics:** fork
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs), schema 1.12.0,
one package + one version per PR. Requires exact `InstallerSha256`, unique
versioned HTTPS URLs, silent install, clean install *and* uninstall for admin and
non-admin. Script-based installers unsupported. **Measured merge latency (last 100
closed PRs): median 1.3 h, p90 2.4 h — but 971 open PRs, oldest 504 days.**

Best path: [`vedantmgoyal9/winget-releaser`](https://github.com/vedantmgoyal9/winget-releaser)
— what Zed uses. Alternatives: `wingetcreate`, or
[Komac](https://github.com/russellbanks/Komac) (Rust, no telemetry, runs on Linux
CI).

**What real Rust apps ship:**

| App | Windows artifacts | Installer | Signed |
|---|---|---|---|
| **Zed** v1.12.0 | `Zed-x86_64.exe`, `Zed-aarch64.exe` | **Inno Setup + signed sparse MSIX** | **Yes — Azure Trusted Signing** |
| Helix 25.07.1 | `.zip` only | none | No |
| Alacritty v0.17.0 | `.msi` + `-portable.exe` | WiX v4 (pinned 4.0.5) | **No** |
| RustDesk 1.4.9 | `.exe` + `.msi` | WiX v4 | not in winget |
| Warp | `.exe` | Inno Setup | both user and machine scope |

*⚠️ WezTerm's latest stable is still 2024-02-03 — do not use as a live reference.*

**`cargo-dist` is alive but thin:** v0.32.0, but 8 of the last 10 commits are
dependabot and there is one human maintainer. **Its MSI path requires WiX v3**,
its only signing integration is SSL.com eSigner (**no Azure Artifact Signing**),
and **it has no winget support.** `cargo-wix` max stable 0.3.9 (2025-03-13), and
its default template is WiX v3.

**Recommended:** Inno Setup + portable zip + winget via winget-releaser. Copy
[Zed's `zed.iss`](https://github.com/zed-industries/zed/blob/main/crates/zed/resources/windows/zed.iss).
Add the signed sparse package later, only when you want the Win11 context menu.
Skip full MSIX and the Store initially. Avoid WiX.

### WebView2, for completeness

*"Preinstalled onto all Windows 11 devices as part of the OS"*, pushed to eligible
Win10 — but Microsoft still recommends distributing the runtime for clean Win10
installs, Server and LTSC. Fixed-version is **≳250 MB** and *"can't be installed
by using an installer"*. **For an editor whose pitch is "native and fast", putting
Chromium on the startup path contradicts the premise.**

## Flagged conflicts and unverified items

- Notepad++ hijack attribution — Wikipedia says APT31; Rapid7/CISA say Lotus
  Blossom.
- Azure Artifact Signing org country list — Microsoft contradicts itself across
  three pages; all agree individuals = US/CA only.
- "3 years business history" — absent from all current docs; one secondary source
  still asserts it. Verify at enrolment.
- Artifact Signing FIPS level — overview says 140-3 L3, FAQ says 140-2 L3.
- OV price — Microsoft quotes both $150–300 and $300–500.
- Certum's €69 card set is currently out of stock; its open-source eligibility
  criteria are unpublished.
- SignPath Foundation eligibility — pages unreachable; rests solely on Microsoft's
  citation.
- SignMyCode / CheapSSLSecurity / K Software prices — all Cloudflare-blocked.
- Win11 24H2/25H2 context-menu changes — secondary sources only; no Microsoft
  primary source found.
- Smart App Control defaults — MS docs 404'd; secondary sources only.
- **No measured startup benchmarks obtained for any of these editors, and none for
  WebView2 vs native.** Don't quote figures.
