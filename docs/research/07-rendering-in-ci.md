# 07 — Rendering in CI

MDEDIT-123, measured 2026-08-15. **The answer is yes**, and further than the ticket expected.

`AGENTS.md` has said since the beginning that "CI cannot open a window, so rendering and hit-testing
are verified by hand on Windows". That sentence has cost real defects — MDEDIT-92's row overflowing a
default window, MDEDIT-116's unclickable menu rows, MDEDIT-81's missing caret, MDEDIT-70's missing
window icon. Every one was found by a human opening the editor.

The sentence is still true and it was never the whole story: **rendering does not need a window, only
a device.**

## What was measured

`spikes/warp-ci` — four probes, run on this laptop (Windows 11 26200, Intel Iris Xe present) and on a
GitHub-hosted `windows-latest` runner (Hyper-V, no hardware adapter at all).

| | laptop | hosted runner |
| --- | --- | --- |
| adapters offered on DX12 | `IntegratedGpu` Iris Xe, `Cpu` Basic Render Driver | **only** `Cpu` Basic Render Driver |
| `Editor<DirectWriteText>::build_frame`, no window | 143 quads, 14.0 ms | 143 quads, **3.2 ms** |
| device from the software adapter | 107 ms | 40–118 ms |
| pipeline, shader, atlas upload | 27 ms | 13–17 ms |
| first frame | 786 ms | 546–554 ms |
| **steady-state frame** | **5.5 ms** | **2.4–3.0 ms** |
| pixels differing from the ground | 2317 / 540000 | **2317 / 540000** |
| stable across processes | yes | yes |

**The hosted runner is faster than the laptop at every stage**, which is worth knowing before
budgeting for it.

## Three findings, in order of how much they change

### 1. The layout half needs no GPU and was already possible

`Editor` is generic over `TextLayout`. Every integration test in `crates/app` instantiates it with
the grid stub whose glyphs are all eight pixels wide — which is precisely what cannot answer "does
this fit". Nothing stopped it being instantiated with `DirectWriteText` instead, and `build_frame`'s
own doc comment says it "touches no renderer".

So `Editor<DirectWriteText>` builds a real frame, headless, in **3 ms on a runner**, and the quads
carry 23 distinct widths where the stub would report a handful. **Every defect in the list above was
a geometry defect** — a row too wide, a row in the wrong place, a missing rectangle — and geometry is
what this produces. `crates/app/tests/icons.rs` already proved the font stack is reachable in CI;
what nobody had tried was pointing the whole editor at it.

This is the cheap, high-value half and it needs no GPU, no WARP, and no new dependency.

### 2. WARP renders this project's actual pipeline

Not a toy triangle: the real `SHADER`, the real `Atlas`, real `QuadInstance`s built from DirectWrite
glyph coverage, alpha-blended into an offscreen `Rgba8Unorm` target and read back.

The runner offers **no hardware adapter at all**, so `DeviceType::Cpu` is not a fallback there, it is
the only option — which makes it exactly the thing to test against.

**The cost is in the first frame, not in setup.** 550 ms for frame one and 2.5 ms for every frame
after it: the driver compiles the shader on first draw. A test that renders one frame pays 550 ms; a
test that renders twenty pays 600 ms. That shapes how such tests should be written — one fixture,
many assertions — and it is not visible from a single-frame measurement.

### 3. The pixels are byte-identical across machines, and that was not expected

The ticket said, reasonably: *"Not pixel-perfect screenshot testing: font rendering varies by Windows
build and that would be a test that fails on somebody else's machine for no reason."*

Measured, both machines produced the same frame — FNV-1a `67440a044e5165cb`, and the same 2317 inked
pixels. WARP is a deterministic software rasteriser and DirectWrite produced identical coverage
bitmaps on both.

**This is evidence, not a guarantee, and the distinction matters.** It is two machines on one day
with one version of Segoe UI. A Windows update that reships a font changes it, and the failure would
land on an unrelated pull request with a diff that explains nothing. The prohibition in the ticket
was about *avoiding a brittle test*, and it should stand — but the *reason* given for it turns out
not to hold on this pairing, which is worth having written down before somebody re-derives it.

## What to build, if this is picked up

In order of value per unit of work:

1. **`Editor<DirectWriteText>` assertions in `crates/app/tests`.** No GPU. Assert geometry: that the
   toolbar fits, that a row is where it should be, that a caret quad exists. This is the one that
   would have caught four of the five defects listed above, and it is a new test file, not a new
   crate.
2. **An offscreen render test**, one fixture, many assertions, `#[ignore]`d or `cfg(windows)`-gated.
   Assert *ink*, not bytes: that something drew where text should be, that the caret is not missing,
   that a panel is opaque. Roughly 550 ms plus 2.5 ms per frame.
3. **Nothing golden.** No committed hash, no reference image. See finding 3.

**One production change is needed for (2) and it is small**: `Editor` keeps its `Atlas` private, so
the spike had to build its own quads rather than render the editor's. A `pub fn atlas(&self)` would
close that, and the argument for it is the same one already written on `build_frame`.

The 6m05s of build time in the spike's CI job is an artefact of it being a separate workspace with no
cache; as a test inside the workspace the marginal cost is the two seconds the probes took.

## Re-running it

```text
cargo run --release --manifest-path spikes/warp-ci/Cargo.toml
```

Printing is the whole output. `.github/workflows/warp-spike.yml` ran the same binary twice on a
hosted runner and compared the hashes; it is a spike artefact and should be deleted rather than
maintained.
