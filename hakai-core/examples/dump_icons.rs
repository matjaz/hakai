//! `cargo run --example dump_icons` — renders every tool icon to PNG for visual review.
//! No window, no compositor, runs anywhere.

use std::path::PathBuf;

use hakai_core::icons::ToolIcons;

fn main() {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/dump"));
    std::fs::create_dir_all(&dir).expect("could not create the output directory");

    let write = |name: &str, icon: &hakai_core::icons::ToolIcon| {
        let path = dir.join(format!("icon-{name}.png"));
        icon.pixmap
            .save_png(&path)
            .unwrap_or_else(|e| panic!("failed to save {}: {e}", path.display()));
        println!(
            "wrote {} (hotspot {:.2},{:.2}{})",
            path.display(),
            icon.hotspot.0,
            icon.hotspot.1,
            if icon.pivot.is_some() { ", has pivot" } else { "" }
        );
    };

    let mut icons = ToolIcons::new();
    write("1-hammer", icons.hammer());
    write("2-chainsaw-idle", icons.chain_saw(false));
    write("2-chainsaw-cutting", icons.chain_saw(true));
    write("3-machinegun", icons.machine_gun());
    write("4-flamethrower", icons.flame_thrower());
    write("5-colorthrower", icons.color_thrower());
    write("6-phaser", icons.phaser());
    write("7-stamp-up", icons.stamp(false));
    write("7-stamp-down", icons.stamp(true));
    write("8-termites", icons.termite_hand());
    write("9-washer", icons.washer());

    println!("done — wrote every tool icon to {}", dir.display());
}
