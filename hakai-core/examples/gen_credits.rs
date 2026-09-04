//! `cargo run --example gen_credits` — regenerates the repo-root `CREDITS.md` from
//! `assets/manifest.json` (the bundled sound metadata — the same file `credits.rs`'s
//! in-app acknowledgements panel reads, so the two can't drift apart) plus this file's own
//! small `FONTS` table. Mirrors the macOS build's `tools/gen_credits.py` / `make credits`,
//! ported to Rust rather than kept as a third Python script — Phase 8's own CI is meant to
//! be `cargo test`-only (see `OMARCHY-PORT.md`), and this way there's exactly one place
//! that knows the manifest's schema, not two implementations that could disagree.
//!
//! Genuinely new territory versus the macOS original, not a straight port: this project
//! bundles two OFL fonts of its own (`assets/fonts/ATTRIBUTION.md` in both crates) that the
//! macOS build never needed, since it drew text with system fonts via CoreText. `FONTS`
//! below is that same information, structured — kept in sync with the two
//! `ATTRIBUTION.md` files by hand, since there are only ever two font families to update.
//!
//! `--check` exits non-zero if `CREDITS.md` is out of sync with its sources, without
//! writing — for CI.
//!
//! Usage:
//!   cargo run --example gen_credits                    # writes ../CREDITS.md
//!   cargo run --example gen_credits -- --check
//!   cargo run --example gen_credits -- --check <path>   # check a specific file instead

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::Deserialize;

const MANIFEST_JSON: &str = include_str!("../assets/manifest.json");

#[derive(Deserialize, Clone)]
struct SoundEntry {
    origin: String,
    license: String,
    author: String,
    source_title: Option<String>,
    source_page: Option<String>,
    generator: Option<String>,
    modified: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    sounds: BTreeMap<String, SoundEntry>,
}

struct Font {
    name: &'static str,
    files: &'static [&'static str],
    source: &'static str,
    license_file: &'static str,
    copyright: &'static str,
    used_for: &'static str,
}

/// Transcribed from the two `assets/fonts/ATTRIBUTION.md` files — kept here as plain data,
/// not a JSON manifest, since there are only ever two font families to keep in sync by
/// hand (unlike the sound manifest, which `tools/fetch_sounds.py`/`synth_sounds.py`
/// actually generate).
const FONTS: &[Font] = &[
    Font {
        name: "Archivo Black",
        files: &["hakai-core/assets/fonts/ArchivoBlack-Regular.ttf"],
        source: "Google Fonts — https://fonts.gstatic.com/s/archivoblack/v23/HTxqL289NzCGg4MzN6KJ7eW6OYs.ttf",
        license_file: "hakai-core/assets/fonts/ArchivoBlack-OFL.txt",
        copyright: "2017 The Archivo Black Project Authors (github.com/Omnibus-Type/ArchivoBlack)",
        used_for: "the stamp decal's baked-in text (`DecalFactory::stamp_print`), standing in for the macOS build's `NSFont.systemFont(weight: .black)`",
    },
    Font {
        name: "JetBrains Mono",
        files: &["hakai/assets/fonts/JetBrainsMono-Regular.ttf", "hakai/assets/fonts/JetBrainsMono-Bold.ttf"],
        source: "Google Fonts — https://fonts.gstatic.com/s/jetbrainsmono/v24/tDbY2o-flEEny0FZhsfKu5WU4zr3E_BX0PnT8RD8yKxjPQ.ttf (regular), .../tDbY2o-flEEny0FZhsfKu5WU4zr3E_BX0PnT8RD8L6tjPQ.ttf (bold)",
        license_file: "hakai/assets/fonts/JetBrainsMono-OFL.txt",
        copyright: "2020 The JetBrains Mono Project Authors (github.com/JetBrains/JetBrainsMono)",
        used_for: "the whole HUD — tool-name label, toast, palette digits/readout, credits panel — standing in for the macOS build's `NSFont.monospacedSystemFont`",
    },
];

const NEEDS_ATTRIBUTION: &str = "cc by";

fn license_url(license: &str) -> Option<&'static str> {
    match license.to_lowercase().as_str() {
        "cc by 4.0" => Some("https://creativecommons.org/licenses/by/4.0/"),
        "cc by 3.0" => Some("https://creativecommons.org/licenses/by/3.0/"),
        "cc by-sa 4.0" => Some("https://creativecommons.org/licenses/by-sa/4.0/"),
        "cc0" => Some("https://creativecommons.org/publicdomain/zero/1.0/"),
        "public domain" => Some("https://en.wikipedia.org/wiki/Public_domain"),
        _ => None,
    }
}

/// `File:Hollow stick hit 2 (Gravity Sound).wav` → `Hollow stick hit 2` — matches
/// `credits.rs`'s own `short_title` (duplicated rather than shared: that one is a private
/// function in a library crate, and duplicating six lines here is simpler and lower-risk
/// than widening that crate's public API just for this one-off tool).
fn short_title(raw: &str) -> String {
    let s = raw.strip_prefix("File:").unwrap_or(raw);
    let s = match s.rfind('.') {
        Some(dot) => &s[..dot],
        None => s,
    };
    let s = if s.ends_with(')') { s.rfind('(').map(|open| &s[..open]).unwrap_or(s) } else { s };
    s.trim().to_string()
}

fn render(manifest: &Manifest) -> String {
    let sounds = &manifest.sounds;
    let mut out = String::new();

    out.push_str("# Credits and licences\n\n");
    out.push_str(
        "This file is **generated** — from `hakai-core/assets/manifest.json` and this \
         file's own font table — by `cargo run --example gen_credits` (run from \
         `hakai-core/`). Do not edit it by hand.\n\n",
    );

    out.push_str("## Graphics\n\n");
    out.push_str(
        "Every decal, tool icon, sprite and cursor is **generated procedurally at \
         runtime** (`hakai-core/src/decals.rs`, `icons.rs`, `sprites.rs`) — there are no \
         external image assets, so there is nothing here to attribute.\n\n",
    );
    out.push_str(
        "> The same sound/font information below is also shown inside the app itself — \
         press **C** while it runs — because a file in a source repository doesn't reach \
         someone who installs a build. Both are built from the same `manifest.json` this \
         file reads.\n\n",
    );

    out.push_str("## Fonts\n\n");
    for font in FONTS {
        out.push_str(&format!("### {}\n\n", font.name));
        out.push_str(&format!("- Files: {}\n", font.files.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")));
        out.push_str(&format!("- Source: {}\n", font.source));
        out.push_str(&format!("- License: SIL Open Font License 1.1 — full text in `{}`\n", font.license_file));
        out.push_str(&format!("- Copyright {}\n", font.copyright));
        out.push_str(&format!("- Used for: {}\n\n", font.used_for));
    }

    out.push_str("## Audio\n\n");
    out.push_str(&format!("{} sounds in total. All are mono 44.1 kHz 16-bit WAV.\n\n", sounds.len()));

    let attribution_needed = sounds.values().filter(|e| e.license.to_lowercase().contains(NEEDS_ATTRIBUTION)).count();
    if attribution_needed > 0 {
        out.push_str(&format!("**{attribution_needed} sounds require the author to be named** (CC BY). The credits are below.\n\n"));
    }

    // Grouped by licence, sorted by name — matches `tools/gen_credits.py`'s own
    // `groups.keys.sorted()`, which happens to put "CC BY 4.0" (the one licence that
    // actually carries an attribution obligation) first: `' '` (in "CC BY") sorts before
    // `'0'` (in "CC0"), which sorts before `'P'` (in "Public domain").
    let mut by_license: BTreeMap<&str, Vec<(&str, &SoundEntry)>> = BTreeMap::new();
    for (name, entry) in sounds {
        by_license.entry(entry.license.as_str()).or_default().push((name.as_str(), entry));
    }

    for (license, mut entries) in by_license {
        entries.sort_by_key(|(name, _)| *name);
        match license_url(license) {
            Some(url) => out.push_str(&format!("### {license} — <{url}>\n\n")),
            None => out.push_str(&format!("### {license}\n\n")),
        }

        if license.to_lowercase().contains(NEEDS_ATTRIBUTION) {
            out.push_str("This licence **requires the author to be named**. The credit stays for as long as these sounds are distributed with the app.\n\n");
        }

        out.push_str("| Sound | Author | Source |\n|-------|--------|--------|\n");
        for (name, entry) in &entries {
            let source = if entry.origin == "synth" {
                format!("`{}`", entry.generator.as_deref().unwrap_or("synthesis"))
            } else {
                let title = entry.source_title.as_deref().map(short_title).unwrap_or_default();
                match &entry.source_page {
                    Some(page) => format!("[{title}]({page})"),
                    None => title,
                }
            };
            out.push_str(&format!("| `{name}` | {} | {source} |\n", entry.author));
        }
        out.push('\n');

        // The modification notice, once per distinct kind of processing rather than
        // repeated on every row.
        let notices: BTreeSet<&str> = entries.iter().filter_map(|(_, e)| e.modified.as_deref()).collect();
        if !notices.is_empty() {
            out.push_str("**Changes made to the originals.** Every file above was processed; none is a verbatim copy:\n\n");
            for notice in notices {
                out.push_str(&format!("* {notice}\n"));
            }
            out.push('\n');
        }
    }

    out.push_str("## The original application\n\n");
    out.push_str(
        "Desktop Destroyer / Desktop Games was written by **Miroslav Němeček** \
         (<http://www.breatharian.eu/Petr/en/program/misc.htm>). This project is an \
         independent Linux/Wayland reimplementation, ported in turn from an independent \
         macOS implementation of the same idea. **No asset from the original is included \
         in this repository** — its sounds and bitmaps live in a proprietary `PET` \
         container inside `DESKTOP.EXE`.\n\n",
    );

    out
}

fn main() {
    let manifest: Manifest = serde_json::from_str(MANIFEST_JSON).expect("bundled hakai-core/assets/manifest.json is malformed");
    let expected = render(&manifest);

    let mut check = false;
    let mut out_path: Option<PathBuf> = None;
    for arg in std::env::args().skip(1) {
        if arg == "--check" {
            check = true;
        } else {
            out_path = Some(PathBuf::from(arg));
        }
    }
    // Defaults to the repo root's `CREDITS.md` — one level up from this crate, since
    // `hakai`/`hakai-core` share one repo but aren't a formal Cargo workspace (see
    // `OMARCHY-PORT.md`'s own note on why not).
    let out_path = out_path.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../CREDITS.md"));

    if check {
        let actual = std::fs::read_to_string(&out_path).unwrap_or_default();
        if actual != expected {
            eprintln!("{} is out of sync with its sources — run: cargo run --example gen_credits (from hakai-core/)", out_path.display());
            std::process::exit(1);
        }
        println!("{} is in sync ({} sounds, {} fonts)", out_path.display(), manifest.sounds.len(), FONTS.len());
        return;
    }

    std::fs::write(&out_path, &expected).unwrap_or_else(|e| panic!("failed to write {}: {e}", out_path.display()));
    let attribution_needed = manifest.sounds.values().filter(|e| e.license.to_lowercase().contains(NEEDS_ATTRIBUTION)).count();
    println!("{} written — {} sounds ({attribution_needed} requiring attribution), {} fonts", out_path.display(), manifest.sounds.len(), FONTS.len());
}
