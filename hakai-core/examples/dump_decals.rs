//! `cargo run --example dump_decals` — renders every decal type/variant to PNG for visual
//! review. Mirrors the macOS build's `make assets` / `--dump-assets`. No window, no
//! compositor, runs anywhere.

use std::path::PathBuf;

use hakai_core::DecalFactory;

fn main() {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/dump"));
    std::fs::create_dir_all(&dir).expect("could not create the output directory");

    let write = |name: String, pixmap: &tiny_skia::Pixmap| {
        let path = dir.join(format!("{name}.png"));
        pixmap
            .save_png(&path)
            .unwrap_or_else(|e| panic!("failed to save {}: {e}", path.display()));
        println!("wrote {}", path.display());
    };

    let mut decals = DecalFactory::new();

    for v in 0..DecalFactory::CRACK_VARIANTS {
        write(format!("crack-{v}"), decals.crack(v));
    }
    for v in 0..DecalFactory::BULLET_HOLE_VARIANTS {
        write(format!("bullet_hole-{v}"), decals.bullet_hole(v));
    }
    for v in 0..DecalFactory::SCORCH_VARIANTS {
        write(format!("scorch-{v}"), decals.scorch(v));
    }
    for (c, (name, ..)) in DecalFactory::DEFAULT_PAINT_COLORS.iter().enumerate() {
        write(format!("paint-{name}"), decals.paint_splat(c as i64, 0));
    }
    for v in 0..DecalFactory::PHASER_VARIANTS {
        write(format!("phaser-{v}"), decals.phaser_hit(v));
    }
    for v in 0..DecalFactory::STAMP_VARIANTS {
        write(format!("stamp-{v}"), decals.stamp_print(v));
    }
    for v in 0..DecalFactory::BITE_VARIANTS {
        write(format!("bite-{v}"), decals.bite(v));
    }
    for v in 0..DecalFactory::BLOOD_VARIANTS {
        write(format!("blood-{v}"), decals.blood(v));
    }
    for v in 0..DecalFactory::SAW_CUT_VARIANTS {
        write(format!("saw_cut-{v}"), decals.saw_cut(v));
    }
    for v in 0..6 {
        write(format!("sliver-{v}"), decals.sliver(v));
    }

    println!("done — wrote every decal type to {}", dir.display());
}
