//! What loading and saving cost on the thread that draws (MDEDIT-151).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout
)]
use std::time::Instant;

#[test]
#[ignore = "a measurement, not an assertion"]
fn what_load_and_save_cost() {
    let dir = tempfile::TempDir::new().unwrap();
    println!(
        "{:>10}  {:>10}  {:>10}  {:>10}  {:>10}",
        "size", "load best", "load worst", "save best", "save worst"
    );
    for kb in [245u64, 1024, 4096, 16384] {
        let text = "A paragraph of ordinary prose about the alpha channel.\n\n"
            .repeat((kb * 1024 / 56) as usize);
        let path = dir.path().join(format!("{kb}.md"));
        std::fs::write(&path, &text).unwrap();

        // Both ends, for the reason `decode_cost` prints them: this machine drifts far enough
        // between runs that a lone figure from here is not reproducible.
        let mut load = (f64::MAX, 0.0_f64);
        let mut save = (f64::MAX, 0.0_f64);
        for _ in 0..3 {
            let started = Instant::now();
            let loaded = mdedit_file::load(&path).unwrap();
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            load = (load.0.min(ms), load.1.max(ms));

            let started = Instant::now();
            mdedit_file::save(&path, &text, &loaded).unwrap();
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            save = (save.0.min(ms), save.1.max(ms));
        }
        println!(
            "{kb:>8} KB  {:>10.1}  {:>10.1}  {:>10.1}  {:>10.1}",
            load.0, load.1, save.0, save.1
        );
    }
}
