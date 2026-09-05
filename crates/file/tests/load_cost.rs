//! What loading and saving cost on the thread that draws (MDEDIT-151).
//!
//! Ignored by default, like `decode_cost` and `frame_cost`: a measurement, not an assertion.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "a measurement harness: it prints numbers and asserts only that the work happened"
)]
use std::time::Instant;

/// The best and worst of the iterations so far.
type Spread = (f64, f64);

fn note(spread: Spread, started: Instant) -> Spread {
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    (spread.0.min(ms), spread.1.max(ms))
}

#[test]
#[ignore = "a measurement, not an assertion"]
fn what_load_and_save_cost() {
    let dir = tempfile::TempDir::new().unwrap();
    println!(
        "{:>10}  {:>19}  {:>19}  {:>19}",
        "size", "load", "save unedited", "save edited"
    );
    println!(
        "{:>10}  {:>9}{:>10}  {:>9}{:>10}  {:>9}{:>10}",
        "", "best", "worst", "best", "worst", "best", "worst"
    );
    for kb in [245u64, 1024, 4096, 16384] {
        let text = "A paragraph of ordinary prose about the alpha channel.\n\n"
            .repeat((kb * 1024 / 56) as usize);
        // One byte different and the same length, so `save` cannot take the byte-identical fast
        // path and has to encode the buffer — which is what saving after typing actually does.
        let mut edited = text.clone();
        edited.replace_range(0..1, "B");

        let path = dir.path().join(format!("{kb}.md"));
        std::fs::write(&path, &text).unwrap();

        // Best *and* worst, for the reason `decode_cost` prints them: this machine drifts far
        // enough between runs that a lone figure from here is not reproducible.
        let mut load = (f64::MAX, 0.0_f64);
        let mut unedited = (f64::MAX, 0.0_f64);
        let mut rewritten = (f64::MAX, 0.0_f64);
        for _ in 0..3 {
            let started = Instant::now();
            let loaded = mdedit_file::load(&path).unwrap();
            load = note(load, started);
            // Or a load that returned nothing would report a very good number indeed.
            assert_eq!(loaded.text().len(), text.len());

            // The buffer as read: `save` compares it with what it holds and writes the original
            // bytes straight back. This is what saving a file nobody has typed into costs.
            let started = Instant::now();
            mdedit_file::save(&path, loaded.text(), &loaded).unwrap();
            unedited = note(unedited, started);

            let started = Instant::now();
            mdedit_file::save(&path, &edited, &loaded).unwrap();
            rewritten = note(rewritten, started);

            // Untimed, and outside the measurement on purpose: the edited save left a different
            // byte on disk, and without this the next iteration's "unedited" save would find the
            // buffer and the file already disagreeing and encode instead of taking the fast path.
            std::fs::write(&path, &text).unwrap();
        }
        println!(
            "{kb:>8} KB  {:>9.1}{:>10.1}  {:>9.1}{:>10.1}  {:>9.1}{:>10.1}",
            load.0, load.1, unedited.0, unedited.1, rewritten.0, rewritten.1
        );
    }
}
