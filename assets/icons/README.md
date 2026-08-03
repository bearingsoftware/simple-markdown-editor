# Application icons

Three stepped bars — the "Slab" mark. Delivered 2026-08-03 as a design pack alongside the theme
(MDEDIT-69), and committed here rather than left in a downloads folder because the build reads one
of them.

## Provenance and licence

Commissioned for this project and owned by it, along with everything else in this repository. The
pack arrived with no licence file and no author metadata; it carries the repository's licence
because it was made for the repository.

## What is here

| Pattern | Wired? | Use |
| --- | --- | --- |
| `mdedit-<size>.png` | **yes**, via `mdedit.ico` | The transparent glyph. |
| `mdedit.ico` | **yes** | Built from the eight glyph PNGs. `crates/app/mdedit.rc` names it. |
| `mdedit-tile-<size>.png` | no | Dark rounded tile. For a Start tile, an installer or a store listing. |
| `mdedit-mono-<size>.png` | no | Single-colour `#e9e9ed`. For a tray icon, a notification, or a light theme. |

Sizes are 16, 20, 24, 32, 48, 64, 128 and 256.

**Sixteen of the twenty-four files have no consumer today** — there is no tray, no notifications, no
installer (the gap register parks it deliberately) and no light theme until MDEDIT-78. They are kept
rather than discarded because they cost 26 KB between them and because redrawing them is not cheap:
see below.

## There is no vector source, and that is deliberate

Every size is drawn on its own pixel grid rather than scaled from one master, which is why the bars
land on whole pixels at 16 and 20 and why the accent ramp lightens at the small sizes — `#9184d9`,
`#8477cd`, `#6f63ab` at 16 and 24 against `#9184d9`, `#796cbf`, `#5d5294` at 32 and above, so the
third bar still separates from the ground when it is four pixels tall.

An SVG has not gone missing. Adding a size means drawing it.

## Rebuilding `mdedit.ico`

The design pack's own README says to run ImageMagick. ImageMagick is not a dependency of this
repository and is not on the CI runners, so the `.ico` is built by a script that needs nothing but
Windows, and the result is committed:

```powershell
pwsh assets/icons/build-ico.ps1
```

Sizes below 256 are written as BMP DIBs and 256 as a PNG. Windows has accepted PNG entries at any
size since Vista, but the shell's older code paths are only reliably exercised for BMP below 256,
and an icon that fails to draw in one context is expensive to find later.

Note that `System.Drawing.Icon` **cannot read the 256 entry** — it is PNG-compressed, and that
wrapper predates the format. A verification script that asks it for 256 gets 128 back and is
reporting its own limitation, not a defect in the file.

## How it reaches the executable

`crates/app/mdedit.rc` declares it as resource `1`, and `crates/app/build.rs` compiles that with
`embed-resource`. Windows uses the lowest-numbered icon group as an executable's icon, which is also
what makes `mdedit-shell-win`'s `DefaultIcon = "<exe>",0` resolve — index 0 there means "the first
icon group", not "resource id 0".

## Colours

Bars `#9184d9`, `#796cbf`, `#5d5294` — the accent ramp at 500, 600 and 700. Tile ground `#1b1d2b`,
corner radius 18% of the size. Mono glyph `#e9e9ed`.
