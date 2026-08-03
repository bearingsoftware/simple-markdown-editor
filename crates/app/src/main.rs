//! The editor binary.
//!
//! Picks a text backend and runs the event loop. The backend choice is the only platform-specific
//! line in the application: [`mdedit_app::editor::Editor`] is generic over the trait.

// The binary talks to the user through stdout when it cannot open a window, which is the one place
// printing is the right behaviour rather than a stray debug statement.
#![allow(clippy::print_stdout, clippy::print_stderr)]
// No console window when Explorer launches this. A terminal launch still gets its output, because
// `main` reattaches to the parent's console — see `mdedit_menu_win::attach_parent_console`, and
// MDEDIT-62 for why that mitigation is not optional.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
use std::fmt::Write as _;

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
    fn ask(
        &mut self,
        name: &str,
        what: mdedit_app::prompt::Closing,
    ) -> mdedit_app::prompt::Unsaved {
        use mdedit_app::prompt::{Closing, Unsaved};

        // A scratch buffer has nowhere to go, so the button that saves it opens a picker. Labelling
        // that "Save" would be a small lie, and the description says where the text lives now — the
        // one place it exists is the editor's own store.
        let (save, description) = match what {
            Closing::File => ("Save".to_owned(), format!("{name} has unsaved changes.")),
            Closing::Scratch => (
                "Save As…".to_owned(),
                format!("{name} has never been saved anywhere.\n\nDiscarding it loses it."),
            ),
        };

        let answer = rfd::MessageDialog::new()
            .set_title("Unsaved changes")
            .set_description(description)
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                save.clone(),
                "Discard".to_owned(),
                "Cancel".to_owned(),
            ))
            .show();

        match answer {
            rfd::MessageDialogResult::Custom(choice) if choice == save => Unsaved::Save,
            rfd::MessageDialogResult::Custom(choice) if choice == "Discard" => Unsaved::Discard,
            // Anything else — Cancel, or the dialog being dismissed — keeps the tab. Losing work is
            // the one outcome that must never be the default.
            _ => Unsaved::Cancel,
        }
    }

    fn ask_all(&mut self, names: &[String]) -> mdedit_app::prompt::Unsaved {
        use mdedit_app::prompt::Unsaved;

        // Named, not counted. "3 files have unsaved changes" makes the user reconstruct which, and
        // the whole reason for one dialog rather than three is that the answer should be informed.
        // Capped, because a hundred names is a dialog taller than the screen — and past a handful
        // the list has stopped being the useful part anyway.
        const SHOWN: usize = 10;
        let listed: Vec<&str> = names.iter().take(SHOWN).map(String::as_str).collect();
        let mut description = format!("These files have unsaved changes:\n\n{}", listed.join("\n"));
        if names.len() > SHOWN {
            let rest = names.len() - SHOWN;
            let _ = write!(description, "\n…and {rest} more");
        }
        // Says what Keep actually does, because "keep for now" on its own could be read as "leave
        // the editor open" — the opposite of what it does.
        description
            .push_str("\n\nKeep for now closes the editor and brings the changes back next time.");

        let answer = rfd::MessageDialog::new()
            .set_title("Unsaved changes")
            .set_description(description)
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                "Save All".to_owned(),
                "Keep for now".to_owned(),
                "Cancel".to_owned(),
            ))
            .show();

        match answer {
            rfd::MessageDialogResult::Custom(choice) if choice == "Save All" => Unsaved::Save,
            rfd::MessageDialogResult::Custom(choice) if choice == "Keep for now" => Unsaved::Keep,
            _ => Unsaved::Cancel,
        }
    }
}

/// The file-changed-on-disk dialog.
///
/// Here for the same manifest reason as `NativePrompt`, which documents it.
#[cfg(windows)]
struct NativeConflict;

#[cfg(windows)]
impl mdedit_app::prompt::ConflictPrompt for NativeConflict {
    fn ask(
        &mut self,
        name: &str,
        why: mdedit_app::prompt::Conflicting,
    ) -> mdedit_app::prompt::Conflict {
        use mdedit_app::prompt::{Conflict, Conflicting};

        // The wording differs because the stakes do. On a focus check nothing is being written and
        // the question is which version to look at. On a save, one of the two versions is about to
        // stop existing, and the button that does that says so.
        let (description, keep) = match why {
            Conflicting::Found => (
                format!("{name} has changed on disk, and you have unsaved changes to it."),
                "Keep mine",
            ),
            Conflicting::Saving => (
                format!(
                    "{name} has changed on disk since you opened it. Saving will replace those \
                     changes."
                ),
                "Overwrite",
            ),
        };

        let answer = rfd::MessageDialog::new()
            .set_title("File changed on disk")
            .set_description(description)
            .set_level(rfd::MessageLevel::Warning)
            .set_buttons(rfd::MessageButtons::YesNoCancelCustom(
                keep.to_owned(),
                "Reload".to_owned(),
                "Cancel".to_owned(),
            ))
            .show();

        match answer {
            rfd::MessageDialogResult::Custom(choice) if choice == keep => match why {
                Conflicting::Found => Conflict::Cancel,
                Conflicting::Saving => Conflict::Overwrite,
            },
            rfd::MessageDialogResult::Custom(choice) if choice == "Reload" => Conflict::Reload,
            // Cancel, or the dialog being dismissed. Changes nothing either way, which is the only
            // safe default when the two versions are both somebody's work.
            _ => Conflict::Cancel,
        }
    }
}

/// Tells the user a panic happened, and where to find the detail.
///
/// Supplied to the library rather than living in it: `rfd` is loadable from this executable and not
/// from anything else that links the library, for the manifest reason `NativePrompt` documents.
#[cfg(windows)]
fn report_panic(message: &str, log_path: Option<&std::path::Path>) {
    let where_to_look = log_path.map_or_else(
        || "No log file could be opened.".to_owned(),
        |path| format!("Details were written to:\n{}", path.display()),
    );
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Simple Markdown Editor stopped")
        .set_description(format!("{message}\n\n{where_to_look}"))
        .show();
}

/// Off Windows there is no dialog to show; the log has it either way.
#[cfg(not(windows))]
fn report_panic(_message: &str, _log_path: Option<&std::path::Path>) {}

/// Hands the editor everything that has to ask the user something, and the settings.
///
/// All of it lives out here rather than in the library for the manifest reason `NativePrompt`
/// documents. The defaults inside the library are all the do-nothing answer, so an editor that never
/// gets this call is safe but inert: it keeps every tab, writes nothing anybody did not choose, and
/// saves nowhere it was not told to.
#[cfg(windows)]
fn give_it_dialogs<T: mdedit_text::TextLayout>(editor: &mut Editor<T>) {
    // Read at startup, and written back on the idle timer when something changes — see
    // `mdedit_app::config` for the unknown-key rule that makes writing safe.
    editor.with_settings(mdedit_app::config::Settings::load());
    // The editor keeps every tab by default when it has nobody to ask. This is the asking.
    editor.with_prompt(Box::new(NativePrompt));
    // Without this the editor answers Cancel to every conflict — safe, but it means a save over an
    // externally changed file silently does nothing.
    editor.with_conflict_prompt(Box::new(NativeConflict));
    editor.with_file_picker(Box::new(|suggested| {
        let mut dialog = rfd::FileDialog::new().add_filter("Markdown", &["md", "markdown"]);
        if let Some(name) = suggested {
            dialog = dialog.set_file_name(name);
        }
        dialog.save_file()
    }));
    // Multi-select, because opening four files at once is the case a File → Open exists for; each
    // lands in its own tab. The filters name what the editor highlights, and All Files is last
    // rather than absent — nothing stops a Markdown document being called something else.
    editor.with_open_picker(Box::new(|| {
        rfd::FileDialog::new()
            .add_filter("Markdown", &["md", "markdown"])
            .add_filter("Data", &["json", "yaml", "yml"])
            .add_filter("All Files", &["*"])
            .pick_files()
    }));
    // A dialog rather than a tab, unlike the shortcuts reference: there is nothing here to scroll,
    // search or copy out, and a version number in a document nobody would keep open is a tab that
    // exists to be closed.
    editor.with_double_click_interval(mdedit_menu_win::double_click_interval());
    editor.with_about(Box::new(|| {
        let _ = rfd::MessageDialog::new()
            .set_title("About Simple Markdown Editor")
            .set_description(format!(
                "Simple Markdown Editor {}\n\nA native WYSIWYG Markdown editor.\nThe buffer is the \
                 source of truth: opening a file and saving it changes no byte you did not edit.",
                env!("CARGO_PKG_VERSION")
            ))
            .show();
    }));
    // Somewhere to say why something the user chose cannot be done — Format Document on a document
    // that is not valid JSON is the only caller so far.
    editor.with_messages(Box::new(|message| {
        rfd::MessageDialog::new()
            .set_title("Simple Markdown Editor")
            .set_description(message)
            .set_level(rfd::MessageLevel::Warning)
            .show();
    }));
    // Read from beside the session file. Done here rather than in `Editor::new` so a test gets an
    // editor that touches nothing of the user's.
    editor.with_recent(mdedit_session::recent::Recent::beside_session());
    // Same rule, and MDEDIT-67 is what breaking it cost: while the real clipboard was the default,
    // every test binary read it — from as many threads as the harness runs tests on, which corrupts
    // the heap. See `mdedit_app::clipboard::System`.
    editor.with_clipboard(Box::new(mdedit_app::clipboard::System::default()));
    // And the same again, which is MDEDIT-68: while the real recovery store was the default, every
    // test that edited a buffer and ran the idle timer wrote unsaved work into the user's own store,
    // and it came back in their editor on the next launch. `None` when there is nowhere to put it —
    // crash recovery is a courtesy and never stops a start.
    if let Some(store) = mdedit_session::snapshot::Store::beside_session() {
        editor.with_snapshot_store(store);
    }
}

/// Writes the session back, marked as a clean exit.
///
/// The session file records handles rather than content, so this stays cheap however many tabs there
/// are.
///
/// **Nothing is cleared.** Since MDEDIT-66 the stored copies of unsaved buffers survive an ordinary
/// quit — that is the feature — so the only thing the exit path owes them is the marker that says
/// this run ended on purpose, which is what stops the next launch calling an ordinary restore a
/// crash recovery.
#[cfg(windows)]
fn wind_down<T: mdedit_text::TextLayout>(
    editor: &mut Editor<T>,
    session_path: Option<&std::path::Path>,
) {
    if let Some(path) = session_path {
        if !editor.workspace().tabs().is_empty() {
            editor.save_session_on_exit(path);
        }
    }
}

/// Opens the log and installs the panic hook, before anything that could need either.
///
/// The dialog is supplied from here rather than from the library for the `rfd` reason documented on
/// `NativePrompt` — the manifest that makes `rfd` loadable belongs to this executable.
fn start_reporting() {
    mdedit_app::log::start();
    mdedit_app::log::install_panic_hook(report_panic);
}

fn main() {
    // Reattach to the terminal that launched us, if there was one. The binary is windows-subsystem
    // so Explorer gets no console flash; the cost is no standard handles at all, which would take
    // the latency harness's output with it. See `attach_parent_console`.
    mdedit_menu_win::attach_parent_console();
    start_reporting();

    #[cfg(not(windows))]
    {
        // parley is the portable backend and is not viable on Windows until parley#689 ships in a
        // released fontique (MDEDIT-5); it is not written yet, so there is nothing to run on.
        mdedit_app::log_error!(
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
                mdedit_app::log_error!("DirectWrite unavailable: {err}");
                std::process::exit(1);
            }
        };

        // Carries menu commands: muda raises them on a channel of its own, and this is how they
        // reach the loop that owns the document.
        let event_loop = match EventLoop::<mdedit_menu_win::Command>::with_user_event().build() {
            Ok(event_loop) => event_loop,
            Err(err) => {
                mdedit_app::log_error!("could not create an event loop: {err}");
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
        // Resolved paths from the command line or Explorer, to go into the recent list once there
        // is one. It is read in `give_it_dialogs`, below, which is after this.
        let mut opened: Vec<std::path::PathBuf> = Vec::new();

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
                        mdedit_app::log_error!("skipped a session entry: {}", tab.reason);
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
                // `from_command_line`, not `validate`: the latter guards the session file, which is
                // a file this process wrote and something else may have edited. This is the user
                // typing. See `mdedit_session::path` for why they are separate functions.
                match mdedit_session::path::from_command_line(&path) {
                    Ok(resolved) => {
                        workspace.add(resolved.clone());
                        opened.push(resolved);
                    }
                    Err(reason) => {
                        mdedit_app::log_error!("could not open: {reason}");
                        std::process::exit(1);
                    }
                }
            }
            let mut editor = Editor::with_workspace(workspace, text);
            editor.activate_saved_tab();
            editor
        };
        // **Before the restore, and that ordering is load-bearing in two directions.** The restore
        // reads the snapshot store, which this installs — MDEDIT-68 made the store something the app
        // wires in rather than something `Editor::new` reaches for, so a restore above this line
        // would find nowhere to restore from and silently do nothing.
        //
        // It also repairs a safeguard that had never fired: `restore_one` asks before putting a
        // stored edit back over a file that has changed on disk since, and it asks through the
        // conflict prompt *this* installs. With the call below the restore, that question was always
        // being answered by the inert `ChangeNothing`, which says Cancel, which means restore anyway.
        // No work was ever lost by it — Cancel keeps the buffer — but the question MDEDIT-66 says is
        // asked was not being asked.
        give_it_dialogs(&mut editor);

        // Unsaved work the last run stored, whether it ended on purpose or not. Restored before the
        // window exists, so the first frame already shows it rather than flashing the on-disk
        // version first.
        //
        // The flag only changes the wording: coming back to your own edits after an ordinary quit is
        // the feature working, and the same text after the process was killed is worth saying out
        // loud. Read from the session file before the store, because restoring writes nothing but
        // the session file is about to be rewritten.
        let ended_cleanly = session_path
            .as_deref()
            .is_some_and(mdedit_session::last_run_ended_cleanly);
        editor.restore_unsaved_work(ended_cleanly);

        // Clear the marker now that it has been read, so it describes *this* run rather than the
        // last one. Without this a run that is killed never rewrites the session, the previous clean
        // exit's `true` survives, and the next launch calls a crash recovery an ordinary restore —
        // which is exactly what the by-hand check caught.
        //
        // One extra session write per launch. The file is handles rather than content, so it is
        // cheap, and it has the side benefit that the session on disk always describes the tabs that
        // are actually open.
        if let Some(path) = session_path.as_deref() {
            if !editor.workspace().tabs().is_empty() {
                editor.save_session(path);
            }
        }

        // Opening from Explorer or a terminal is how most people start this editor, and it does not
        // go through `Editor::open_paths` — so without this the recent list would only ever know
        // about files opened from its own dialog. Reversed, so the last one named ends up first.
        for path in opened.iter().rev() {
            editor.remember(path);
        }
        // Menu commands arrive on muda's own channel; this is what carries them into the loop.
        editor.forward_menu(event_loop.create_proxy());

        if let Err(err) = event_loop.run_app(&mut editor) {
            mdedit_app::log_error!("event loop stopped: {err}");
            std::process::exit(1);
        }
        if let Some(err) = editor.gpu_error() {
            mdedit_app::log_error!("could not start the renderer: {err}");
            std::process::exit(1);
        }

        wind_down(&mut editor, session_path.as_deref());

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
