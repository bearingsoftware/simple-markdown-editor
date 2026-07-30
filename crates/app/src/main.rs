//! The editor binary.
//!
//! Picks a text backend and runs the event loop. The backend choice is the only platform-specific
//! line in the application: [`mdedit_app::editor::Editor`] is generic over the trait.

// The binary talks to the user through stdout when it cannot open a window, which is the one place
// printing is the right behaviour rather than a stray debug statement.
#![allow(clippy::print_stdout, clippy::print_stderr)]

#[cfg(windows)]
use mdedit_app::{SAMPLE, editor::Editor};
#[cfg(windows)]
use winit::event_loop::{ControlFlow, EventLoop};

/// The unsaved-changes dialog.
///
/// Here rather than in the library, and not only for testability: `rfd` imports from the `ComCtl32` v6
/// side-by-side assembly, which comes from the manifest `build.rs` embeds into *this* executable. A
/// test binary has no manifest, so a test that merely links this would die at load with
/// `STATUS_ENTRYPOINT_NOT_FOUND` before running. Keeping it in the binary keeps it out of them.
#[cfg(windows)]
struct NativePrompt;

#[cfg(windows)]
impl mdedit_app::prompt::UnsavedPrompt for NativePrompt {
    fn ask(&mut self, name: &str) -> mdedit_app::prompt::Unsaved {
        use mdedit_app::prompt::Unsaved;

        let answer = rfd::MessageDialog::new()
            .set_title("Unsaved changes")
            .set_description(format!("{name} has unsaved changes."))
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                "Save".to_owned(),
                "Discard".to_owned(),
                "Cancel".to_owned(),
            ))
            .show();

        match answer {
            rfd::MessageDialogResult::Custom(choice) if choice == "Save" => Unsaved::Save,
            rfd::MessageDialogResult::Custom(choice) if choice == "Discard" => Unsaved::Discard,
            // Anything else — Cancel, or the dialog being dismissed — keeps the tab. Losing work is
            // the one outcome that must never be the default.
            _ => Unsaved::Cancel,
        }
    }
}

fn main() {
    #[cfg(not(windows))]
    {
        // parley is the portable backend and is not viable on Windows until parley#689 ships in a
        // released fontique (MDEDIT-5); it is not written yet, so there is nothing to run on.
        eprintln!(
            "No text backend for this platform yet. DirectWrite is Windows-only and the portable \
             parley backend is not written — see AGENTS.md, \"Text layer\"."
        );
        std::process::exit(1);
    }

    #[cfg(windows)]
    {
        // Stamped first, so "file open to first paint" includes everything the user waited for and
        // not just the part after the window exists.
        let launched = std::time::Instant::now();

        let text = match mdedit_text_dwrite::DirectWriteText::new() {
            Ok(text) => text,
            Err(err) => {
                eprintln!("DirectWrite unavailable: {err}");
                std::process::exit(1);
            }
        };

        // Carries menu commands: muda raises them on a channel of its own, and this is how they
        // reach the loop that owns the document.
        let event_loop = match EventLoop::<mdedit_menu_win::Command>::with_user_event().build() {
            Ok(event_loop) => event_loop,
            Err(err) => {
                eprintln!("could not create an event loop: {err}");
                std::process::exit(1);
            }
        };
        // Wait rather than poll: a text editor has nothing to do between events, and polling would
        // burn a core and distort the very latency numbers this slice exists to measure.
        event_loop.set_control_flow(ControlFlow::Wait);

        // Arguments are files to open. No flag parsing, because there are no flags.
        let arguments: Vec<std::path::PathBuf> =
            std::env::args_os().skip(1).map(Into::into).collect();
        let session_path = mdedit_session::default_session_path();

        let mut editor = if arguments.is_empty() {
            // Nothing named, so restore what was open. Only the active tab is read; the rest stay
            // handles until something displays them.
            match session_path
                .as_deref()
                .map(mdedit_session::Workspace::restore_session)
            {
                Some(Ok((workspace, skipped))) if !workspace.tabs().is_empty() => {
                    for tab in &skipped {
                        // Reported rather than silently dropped: a tab vanishing without explanation
                        // looks like data loss.
                        eprintln!("skipped a session entry: {}", tab.reason);
                    }
                    let mut editor = Editor::with_workspace(workspace, text);
                    editor.activate_saved_tab();
                    editor
                }
                // No session, an unreadable one, or one with nothing in it: show the sample.
                _ => Editor::new(SAMPLE, text),
            }
        } else {
            let mut workspace = mdedit_session::Workspace::new();
            for path in arguments {
                match mdedit_session::path::validate(&path) {
                    Ok(()) => {
                        workspace.add(path);
                    }
                    Err(reason) => {
                        eprintln!("could not open: {reason}");
                        std::process::exit(1);
                    }
                }
            }
            let mut editor = Editor::with_workspace(workspace, text);
            editor.activate_saved_tab();
            editor
        };
        // The editor keeps every tab by default when it has nobody to ask. This is the asking.
        editor.with_prompt(Box::new(NativePrompt));
        editor.with_file_picker(Box::new(|suggested| {
            let mut dialog = rfd::FileDialog::new().add_filter("Markdown", &["md", "markdown"]);
            if let Some(name) = suggested {
                dialog = dialog.set_file_name(name);
            }
            dialog.save_file()
        }));
        // Menu commands arrive on muda's own channel; this is what carries them into the loop.
        editor.forward_menu(event_loop.create_proxy());

        if let Err(err) = event_loop.run_app(&mut editor) {
            eprintln!("event loop stopped: {err}");
            std::process::exit(1);
        }
        if let Some(err) = editor.gpu_error() {
            eprintln!("could not start the renderer: {err}");
            std::process::exit(1);
        }

        // Written on the way out so the next launch reopens what was open. Only handles, so this
        // stays cheap however many tabs there are.
        if let Some(path) = session_path.as_deref() {
            if !editor.workspace().tabs().is_empty() {
                editor.save_session(path);
            }
        }

        // Printed on exit so a manual session doubles as a measurement run, which is how the
        // battery numbers in the PR body are produced.
        if let Some(painted) = editor.first_paint() {
            let elapsed = painted.duration_since(launched).as_secs_f64() * 1000.0;
            let verdict = if elapsed <= 50.0 { "PASS" } else { "FAIL" };
            println!("\nlaunch -> first paint: {elapsed:.2} ms (target <= 50 ms {verdict})");
        }

        if !editor.latency().is_empty() {
            // Two numbers, because they answer different questions. The first is what this process
            // spends; the second includes waiting for the display, which under Fifo is a refresh
            // interval however little work the frame did.
            println!("\nkeystroke -> quads ready (CPU)");
            print!("{}", editor.build_latency().report());
            println!("\nkeystroke -> present (includes the wait for vblank)");
            print!("{}", editor.latency().report());
        }
    }
}
