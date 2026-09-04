//! `cargo run --example simulate` — drives all nine tools headlessly and reports how much
//! of the damage layer each one covers, plus the five rules-of-the-original interaction
//! checks. Mirrors the macOS build's `hakai --simulate`. No window, no compositor.

use hakai_core::simulation::{check_interactions, run};
use hakai_core::tools::ToolId;

fn main() {
    let results = run();

    println!("tool             coverage  particles  termites  note");
    for r in &results {
        println!(
            "{:<2} {:<14} {:>7.2}%  {:>9}  {:>8}  {}",
            r.tool.key_digit(),
            r.tool.display_name(),
            r.coverage * 100.0,
            r.particles,
            r.termites,
            r.note
        );
    }
    println!();

    let mut failures = 0;
    for check in check_interactions() {
        println!("{} {} — {}", if check.passed { "OK  " } else { "FAIL" }, check.rule, check.detail);
        if !check.passed {
            failures += 1;
        }
    }

    // Every tool has to draw something; the only exception is the washer, which erases.
    for r in &results {
        if r.coverage < 0.001 && r.tool != ToolId::Washer {
            println!("FAIL {} did not change the damage layer", r.tool.display_name());
            failures += 1;
        }
    }

    if failures == 0 {
        println!("\nall checks passed");
    } else {
        println!("\n{failures} check(s) failed");
    }
    std::process::exit(if failures == 0 { 0 } else { 1 });
}
