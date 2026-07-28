//! Embeds the Windows application manifest.
//!
//! `AGENTS.md` requires Per-Monitor DPI v2 be declared in the manifest, and winit deliberately will
//! not do it — it is a process-wide decision that a library has no business making for its host.
//! Without it Windows lies about the window size on any scaled display and hit-testing lands in the
//! wrong place.

// A build script has no error channel other than failing the build, and shipping a binary that
// silently lacks Per-Monitor DPI v2 is worse than not building: the defect only shows up as
// mis-scaled text on someone else's monitor.
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
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
