//! `cargo run --example app_icon -- <out.png>` — the 1024 px master for `AppIcon.icns`.
//!
//! The same procedural cracked monitor PKGBUILD installs on Linux (`make_app_icon`),
//! rendered at macOS's largest icon size. The artwork already keeps a margin roughly where
//! Apple's icon grid wants it, so no extra padding.

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "app_icon.png".into());
    hakai_core::icons::make_app_icon(1024)
        .save_png(&out)
        .unwrap_or_else(|e| panic!("failed to save {out}: {e}"));
    println!("wrote {out}");
}
