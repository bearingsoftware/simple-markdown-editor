//! Probe 3's native reference: the same analysis and edit the Node bench times, in one process.
use std::time::Instant;

use sme_vscode_core::{Session, analyze, analyze_count, fixture};

fn stats(mut xs: Vec<f64>) -> String {
    xs.sort_by(f64::total_cmp);
    let q = |p: f64| xs[((p * xs.len() as f64).ceil() as usize).clamp(1, xs.len()) - 1];
    format!("median {:.3} ms  p99 {:.3} ms  (n={})", q(0.5), q(0.99), xs.len())
}

fn time(n: usize, mut f: impl FnMut()) -> String {
    for _ in 0..3 {
        f();
    }
    let mut out = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        f();
        out.push(t.elapsed().as_secs_f64() * 1e3);
    }
    stats(out)
}

fn main() {
    for (label, size, runs) in [("100KB", 100 * 1024, 100), ("1MB", 1024 * 1024, 40)] {
        let text = fixture(size);
        println!("{label} ({} bytes)", text.len());
        println!("  analyze_count  {}", time(runs, || { std::hint::black_box(analyze_count(&text)); }));
        println!("  analyze_flat   {}", time(runs, || { std::hint::black_box(analyze(&text)); }));
        println!("  session_new    {}", time(runs, || { std::hint::black_box(Session::new(&text)); }));
        println!("  clone_1x_string {}", time(runs, || { std::hint::black_box(text.clone()); }));
        let mut s = Session::new(&text);
        let mid = text[text.len() / 2..].find("Some prose").unwrap() + text.len() / 2 + 5;
        let (mut ins, mut rec) = (Vec::new(), Vec::new());
        for i in 0..runs + 3 {
            let t = Instant::now();
            s.replace(mid as u32, mid as u32, "x").ok();
            let a = t.elapsed().as_secs_f64() * 1e3;
            let t = Instant::now();
            s.reconcile();
            let b = t.elapsed().as_secs_f64() * 1e3;
            s.undo();
            if i >= 3 {
                ins.push(a);
                rec.push(b);
            }
        }
        println!("  edit_replace   {}", stats(ins));
        println!("  edit_reconcile {}", stats(rec));
    }
}
