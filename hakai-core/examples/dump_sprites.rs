//! `cargo run --example dump_sprites` — renders every sprite type/variant to PNG for
//! visual review. No window, no compositor, runs anywhere.

use std::path::PathBuf;

use hakai_core::decals::DecalFactory;
use hakai_core::sprites::SpriteFactory;

fn main() {
    let dir: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/dump"));
    std::fs::create_dir_all(&dir).expect("could not create the output directory");

    let write = |name: String, pixmap: &tiny_skia::Pixmap| {
        let path = dir.join(format!("sprite-{name}.png"));
        pixmap
            .save_png(&path)
            .unwrap_or_else(|e| panic!("failed to save {}: {e}", path.display()));
        println!("wrote {}", path.display());
    };

    let mut sprites = SpriteFactory::new();

    for f in 0..SpriteFactory::FLAME_FRAMES {
        write(format!("flame-{f}"), sprites.flame(f));
    }
    for f in 0..SpriteFactory::FLAME_FRAMES {
        write(format!("standing_flame-{f}"), sprites.standing_flame(f));
    }
    for f in 0..SpriteFactory::TERMITE_FRAMES {
        write(format!("termite-{f}"), sprites.termite(f));
    }
    let paint_colors = DecalFactory::DEFAULT_PAINT_COLORS.map(|(_, r, g, b)| (r, g, b));
    for c in 0..8 {
        write(format!("droplet-{c}"), sprites.droplet(c, &paint_colors));
    }
    write("shell".to_string(), sprites.shell());
    write("flash".to_string(), sprites.flash());
    write("beam".to_string(), sprites.beam());
    for f in 0..SpriteFactory::SPRAY_FRAMES {
        write(format!("spray-{f}"), sprites.spray(f));
    }

    println!("done — wrote every sprite type to {}", dir.display());
}
