//! `cargo run --example app_icon -- <out.png> [px]` — the app icon for the release tarball.
//!
//! The same procedural cracked monitor PKGBUILD installs (via hakai-core's `dump_icons`),
//! built here instead so `package.sh` reuses this crate's already-compiled hakai-core
//! rather than compiling it a second time in hakai-core's own target dir.

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "hakai.png".into());
    let px = args.next().and_then(|s| s.parse().ok()).unwrap_or(256);
    hakai_core::icons::make_app_icon(px)
        .save_png(&out)
        .unwrap_or_else(|e| panic!("failed to save {out}: {e}"));
    println!("wrote {out}");
}
