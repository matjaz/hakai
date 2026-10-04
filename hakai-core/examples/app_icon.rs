//! `cargo run --example app_icon -- <out.png> [px]` — the procedural app icon (a cracked
//! monitor, `icons::make_app_icon`) at any size. The Android package renders each launcher
//! density straight from it rather than scaling one bitmap.

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "app_icon.png".into());
    let px = args.next().and_then(|s| s.parse().ok()).unwrap_or(256);
    hakai_core::icons::make_app_icon(px)
        .save_png(&out)
        .unwrap_or_else(|e| panic!("failed to save {out}: {e}"));
    println!("wrote {out}");
}
