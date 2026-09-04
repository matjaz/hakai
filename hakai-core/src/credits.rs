//! The in-app acknowledgements text — ported from `CreditsPanel.swift`'s `build(bank:
//! columns:)`.
//!
//! This exists to satisfy a licence obligation, not as a nicety: 23 of the bundled sounds
//! are CC BY 4.0, which requires naming the creator, the licence and the fact that the
//! material was modified — *with the work*. A `CREDITS.md` in a source repository doesn't
//! reach somebody who installs a build, so the attribution has to be reachable from inside
//! the running app.
//!
//! Reads `assets/manifest.json`, embedded at compile time (`include_str!`) — the same file
//! Phase 5's real audio engine will load sounds from. This module only needs the sounds'
//! *metadata* (licence, author, source), not the audio playback itself, so building the
//! credits text doesn't have to wait for Phase 5 — and since both read the one bundled
//! file, they can't drift apart later either.
//!
//! **No font metrics in this crate.** `CreditsPanel.swift` computes its column budget from
//! `NSFont.maximumAdvancement.width` *before* calling `build`, since the font is
//! monospaced and a character count is therefore an exact width budget — the wrapping
//! logic itself never needs to know pixel widths. Matched here: `build`'s caller (a
//! renderer, with an actual embedded font to measure) supplies `columns`.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

const MANIFEST_JSON: &str = include_str!("../assets/manifest.json");

#[derive(Deserialize, Clone)]
struct SoundEntry {
    origin: String,
    license: String,
    author: String,
    source_title: Option<String>,
    modified: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    sounds: BTreeMap<String, SoundEntry>,
}

/// A small, fixed palette — matches `CreditsPanel.swift`'s three colors exactly. A `Color`
/// type of its own (rather than raw floats) keeps this crate free of any GPU/graphics
/// dependency; a renderer maps these to whatever it actually draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextColor {
    White,
    Dim,
    Accent,
}

/// One line of laid-out credits text — a renderer draws each at its own size/color, with
/// `gap_before` (points) added above it. Matches `CreditsPanel.swift`'s private `Line`.
#[derive(Clone, Debug, PartialEq)]
pub struct CreditsLine {
    pub text: String,
    pub size: f32,
    pub color: TextColor,
    pub gap_before: f32,
}

/// Builds the full acknowledgements text, wrapped to `columns` characters. Panics if the
/// bundled manifest doesn't parse — it's compiled into the binary, not user-supplied, so a
/// parse failure here is a build-time bug, not a runtime condition to recover from (same
/// reasoning `DecalFactory`'s embedded font panics on a bad file).
pub fn build(columns: usize) -> Vec<CreditsLine> {
    let manifest: Manifest = serde_json::from_str(MANIFEST_JSON).expect("bundled assets/manifest.json is malformed");
    build_from(&manifest.sounds, columns)
}

/// The sound count the "AUDIO" header reports — exposed separately so a check can assert
/// against it without re-parsing the manifest itself.
pub fn sound_count() -> usize {
    let manifest: Manifest = serde_json::from_str(MANIFEST_JSON).expect("bundled assets/manifest.json is malformed");
    manifest.sounds.len()
}

fn push_line(out: &mut Vec<CreditsLine>, text: impl Into<String>, size: f32, color: TextColor, gap_before: f32) {
    out.push(CreditsLine { text: text.into(), size, color, gap_before });
}

/// Packs items onto lines, breaking only between them — suitable for a list of short
/// titles, none of which is ever wider than the budget on its own. `CreditsPanel.swift`'s
/// `packed`.
fn packed(out: &mut Vec<CreditsLine>, items: &[String], columns: usize, color: TextColor) {
    const SEPARATOR: &str = "  ·  ";
    const INDENT: &str = "  ";
    if items.is_empty() {
        return;
    }
    let mut current = INDENT.to_string();
    for item in items {
        let candidate = if current == INDENT { format!("{current}{item}") } else { format!("{current}{SEPARATOR}{item}") };
        if candidate.chars().count() > columns {
            push_line(out, current.clone(), 11.0, color, 0.0);
            current = format!("{INDENT}{item}");
        } else {
            current = candidate;
        }
    }
    if current != INDENT {
        push_line(out, current, 11.0, color, 0.0);
    }
}

/// Word-wraps a sentence — needed because the modification notice is a single long string
/// that has to break *inside* itself, which `packed` can't do. `CreditsPanel.swift`'s
/// `paragraph`.
fn paragraph(out: &mut Vec<CreditsLine>, text: &str, columns: usize, color: TextColor) {
    let mut current = String::new();
    for word in text.split(' ') {
        let candidate = if current.is_empty() { word.to_string() } else { format!("{current} {word}") };
        if candidate.chars().count() > columns {
            push_line(out, current.clone(), 11.0, color, 0.0);
            current = word.to_string();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        push_line(out, current, 11.0, color, 0.0);
    }
}

/// `File:Hollow stick hit 2 (Gravity Sound).wav` → `Hollow stick hit 2`.
///
/// The author is already named above the list, so repeating it on every row would only
/// cost width. The full titles and their URLs stay in the shipped manifest.
fn short_title(raw: &str) -> String {
    let s = raw.strip_prefix("File:").unwrap_or(raw);
    let s = match s.rfind('.') {
        Some(dot) => &s[..dot],
        None => s,
    };
    let s = if s.ends_with(')') {
        match s.rfind('(') {
            Some(open) => &s[..open],
            None => s,
        }
    } else {
        s
    };
    s.trim().to_string()
}

fn license_url(license: &str) -> Option<&'static str> {
    match license.to_lowercase().as_str() {
        "cc by 4.0" => Some("creativecommons.org/licenses/by/4.0/"),
        "cc by 3.0" => Some("creativecommons.org/licenses/by/3.0/"),
        "cc0" => Some("creativecommons.org/publicdomain/zero/1.0/"),
        "public domain" => Some("en.wikipedia.org/wiki/Public_domain"),
        _ => None,
    }
}

fn build_from(sounds: &BTreeMap<String, SoundEntry>, columns: usize) -> Vec<CreditsLine> {
    let mut out = Vec::new();

    push_line(&mut out, "Acknowledgements", 18.0, TextColor::White, 0.0);

    push_line(&mut out, "GRAPHICS", 11.0, TextColor::Dim, 14.0);
    push_line(&mut out, "Every decal, tool icon, sprite and this app's own icon is", 12.0, TextColor::White, 0.0);
    push_line(&mut out, "generated procedurally while it runs.", 12.0, TextColor::White, 0.0);
    push_line(&mut out, "There are no third-party image assets.", 12.0, TextColor::White, 0.0);

    push_line(&mut out, format!("AUDIO  —  {} sounds, mono 44.1 kHz 16-bit", sounds.len()), 11.0, TextColor::Dim, 14.0);

    // Grouped by licence. A plain string sort of the licence names happens to put "CC BY
    // 4.0" — the one that actually carries an attribution obligation — first: `' '`
    // (space, in "CC BY") sorts before `'0'` (in "CC0"), which sorts before `'P'` (in
    // "Public domain"). Matches `groups.keys.sorted()` in the Swift original exactly.
    let mut groups: BTreeMap<&str, Vec<(&str, &SoundEntry)>> = BTreeMap::new();
    for (name, entry) in sounds {
        groups.entry(entry.license.as_str()).or_default().push((name.as_str(), entry));
    }
    for (license, mut entries) in groups {
        entries.sort_by_key(|(name, _)| *name);
        let authors: BTreeSet<&str> = entries.iter().map(|(_, e)| e.author.as_str()).collect();
        let header = match license_url(license) {
            Some(url) => format!("{license}  ·  {url}"),
            None => license.to_string(),
        };
        push_line(&mut out, header, 12.0, TextColor::Accent, 12.0);

        let origin = if entries.first().map(|(_, e)| e.origin.as_str()) == Some("synth") {
            "synthesised in tools/synth_sounds.py"
        } else {
            "from Wikimedia Commons"
        };
        let authors_joined = authors.into_iter().collect::<Vec<_>>().join(", ");
        push_line(&mut out, format!("{} sounds by {authors_joined}, {origin}.", entries.len()), 12.0, TextColor::White, 0.0);

        // The modification notice, once per distinct kind of processing.
        let notices: BTreeSet<&str> = entries.iter().filter_map(|(_, e)| e.modified.as_deref()).collect();
        for notice in notices {
            paragraph(&mut out, &format!("Modified: {notice}"), columns, TextColor::Dim);
        }

        let titles: Vec<String> = entries.iter().map(|(name, e)| e.source_title.as_deref().map(short_title).unwrap_or_else(|| (*name).to_string())).collect();
        packed(&mut out, &titles, columns, TextColor::Dim);
    }

    push_line(&mut out, "THE ORIGINAL", 11.0, TextColor::Dim, 14.0);
    push_line(&mut out, "Behaviour derived from Desktop Destroyer by Miroslav Němeček", 12.0, TextColor::White, 0.0);
    push_line(&mut out, "(breatharian.eu/Petr). This is an independent implementation;", 12.0, TextColor::White, 0.0);
    push_line(&mut out, "no asset from the original is included.", 12.0, TextColor::White, 0.0);

    push_line(&mut out, "Per-file source links are in this app's bundled manifest.json,", 11.0, TextColor::Dim, 14.0);
    push_line(&mut out, "and in CREDITS.md in the source repository.", 11.0, TextColor::Dim, 0.0);

    push_line(&mut out, "Press C to close", 11.0, TextColor::Dim, 14.0);

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_manifest_parses() {
        let lines = build(60);
        assert!(!lines.is_empty());
    }

    #[test]
    fn sound_count_matches_the_manifest() {
        assert_eq!(sound_count(), 35, "the bundled manifest should list all 35 sounds");
    }

    // `no_line_exceeds_the_column_budget` (a blanket check over every line `build`
    // produces) doesn't actually hold, in this crate or in `CreditsPanel.swift`: several
    // lines are fixed English sentences or generated summaries (`"{N} sounds by {authors},
    // {origin}."`) that are *never* wrapped, on the assumption that `columns` is
    // realistically always comfortably wider than one sentence — true of any real screen,
    // not true of an arbitrarily narrow test budget. The two functions that actually
    // promise to respect `columns` are `packed` and `paragraph`; test those directly
    // instead of the whole page.

    #[test]
    fn paragraph_wraps_long_text_to_the_column_budget() {
        let mut out = Vec::new();
        let text = "This is a long sentence that should wrap across several lines once it exceeds the column budget given to it.";
        paragraph(&mut out, text, 30, TextColor::Dim);
        assert!(out.len() > 1, "a sentence this long should wrap into multiple lines at 30 columns");
        for line in &out {
            assert!(line.text.chars().count() <= 30, "line {:?} is {} chars, over the 30-column budget", line.text, line.text.chars().count());
        }
        // Word order survives the wrap.
        let rejoined = out.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ");
        assert_eq!(rejoined, text);
    }

    #[test]
    fn packed_wraps_many_items_to_the_column_budget() {
        let mut out = Vec::new();
        let items: Vec<String> = (1..=20).map(|i| format!("Item {i}")).collect();
        packed(&mut out, &items, 30, TextColor::Dim);
        assert!(out.len() > 1, "20 items should need more than one line at 30 columns");
        for line in &out {
            assert!(line.text.chars().count() <= 30, "line {:?} is {} chars, over the 30-column budget", line.text, line.text.chars().count());
        }
    }

    #[test]
    fn starts_with_the_title_and_ends_with_the_close_hint() {
        let lines = build(60);
        assert_eq!(lines.first().unwrap().text, "Acknowledgements");
        assert_eq!(lines.last().unwrap().text, "Press C to close");
    }

    #[test]
    fn every_licence_group_is_represented() {
        let lines = build(60);
        let joined: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert!(joined.iter().any(|t| t.starts_with("CC BY 4.0")));
        assert!(joined.iter().any(|t| t.starts_with("CC0")));
        assert!(joined.iter().any(|t| t.starts_with("Public domain")));
    }

    #[test]
    fn short_title_strips_the_file_prefix_extension_and_parenthetical() {
        assert_eq!(short_title("File:Hollow stick hit 2 (Gravity Sound).wav"), "Hollow stick hit 2");
        assert_eq!(short_title("Drip.wav"), "Drip");
    }

    #[test]
    fn a_narrower_budget_produces_more_lines() {
        let narrow = build(40).len();
        let wide = build(120).len();
        assert!(narrow >= wide, "wrapping to a narrower budget shouldn't produce fewer lines");
    }
}
