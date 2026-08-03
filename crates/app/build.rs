//! Embeds the Windows application manifest and the application icon.
//!
//! `AGENTS.md` requires Per-Monitor DPI v2 be declared in the manifest, and winit deliberately will
//! not do it — it is a process-wide decision that a library has no business making for its host.
//! Without it Windows lies about the window size on any scaled display and hit-testing lands in the
//! wrong place.
//!
//! # Two crates, and why neither does the other's job
//!
//! `embed-manifest` is pure Rust and writes the manifest without a resource compiler, which is what
//! keeps the DPI declaration free of any toolchain requirement. It does manifests only.
//!
//! `embed-resource` compiles `mdedit.rc` for the icon. It needs `rc.exe`, which is **not** an extra
//! requirement: `rc.exe` ships in the same Windows SDK that provides the `link.exe` and the CRT that
//! any MSVC-target Rust build already needs. If this crate builds at all, the SDK is there.
//!
//! They do not collide, because on MSVC `embed-manifest` passes linker options rather than
//! contributing a resource. Moving the manifest into `mdedit.rc` would work and is the wrong trade:
//! it would make the DPI declaration depend on a resource compiler being found.
//!
//! # What embedding the icon does not do
//!
//! It does not give the **window** one. A window's icon comes from its class or from `WM_SETICON`,
//! and winit sets neither — so the resource here is what Explorer, the taskbar's pinned entry and
//! `DefaultIcon` draw, and `mdedit_menu_win::set_window_icon` is what puts it on the title bar and
//! in Alt-Tab. MDEDIT-70 assumed one implied the other; the window said otherwise.

// A build script has no error channel other than failing the build, and shipping a binary that
// silently lacks Per-Monitor DPI v2 is worse than not building: the defect only shows up as
// mis-scaled text on someone else's monitor. The icon is held to the same standard — an executable
// that quietly lost its icon is the defect this ticket exists to fix.
#![allow(clippy::expect_used)]

fn main() {
    #[cfg(windows)]
    {
        use embed_manifest::{embed_manifest, manifest::DpiAwareness, new_manifest};

        // Only when building for Windows as an executable; a cross-compile check or a docs build
        // must not fail here.
        if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
            embed_manifest(
                new_manifest("BearingSoftware.SimpleMarkdownEditor")
                    .dpi_awareness(DpiAwareness::PerMonitorV2),
            )
            .expect("embedding the application manifest");

            // The binary only. A test harness linking the library has no use for an icon, and
            // naming the target keeps the resource off everything else in the crate.
            embed_resource::compile_for("mdedit.rc", ["mdedit"], embed_resource::NONE)
                .manifest_required()
                .expect("embedding the application icon");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=mdedit.rc");
    println!("cargo:rerun-if-changed=../../assets/icons/mdedit.ico");
}
