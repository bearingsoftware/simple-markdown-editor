//! The editor binary.
//!
//! Picks a text backend and runs the event loop. The backend choice is the only platform-specific
//! line in the application: [`mdedit_app::editor::Editor`] is generic over the trait.

// The binary talks to the user through stdout when it cannot open a window, which is the one place
// printing is the right behaviour rather than a stray debug statement.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use mdedit_app::{SAMPLE, editor::Editor};
use winit::event_loop::{ControlFlow, EventLoop};

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
        let text = match mdedit_text_dwrite::DirectWriteText::new() {
            Ok(text) => text,
            Err(err) => {
                eprintln!("DirectWrite unavailable: {err}");
                std::process::exit(1);
            }
        };

        let event_loop = match EventLoop::new() {
            Ok(event_loop) => event_loop,
            Err(err) => {
                eprintln!("could not create an event loop: {err}");
                std::process::exit(1);
            }
        };
        // Wait rather than poll: a text editor has nothing to do between events, and polling would
        // burn a core and distort the very latency numbers this slice exists to measure.
        event_loop.set_control_flow(ControlFlow::Wait);

        let mut editor = Editor::new(SAMPLE, text);
        if let Err(err) = event_loop.run_app(&mut editor) {
            eprintln!("event loop stopped: {err}");
            std::process::exit(1);
        }
        if let Some(err) = editor.gpu_error() {
            eprintln!("could not start the renderer: {err}");
            std::process::exit(1);
        }

        // Printed on exit so a manual session doubles as a measurement run, which is how the
        // battery numbers in the PR body are produced.
        if !editor.latency().is_empty() {
            println!("\nkeystroke -> present");
            print!("{}", editor.latency().report());
        }
    }
}
