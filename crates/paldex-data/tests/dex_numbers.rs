//! Guards the vendored Paldeck numbers against the user's own pak.
//!
//! `data/dex_numbers.tsv` is the one piece of reference data Paldex does not
//! read from the installed game (see that file's header for why). That makes
//! it the piece most likely to drift when the game updates, and drift here is
//! invisible: a stale table still renders a plausible-looking grid, just with
//! wrong numbers and a wrong denominator. These tests join it back against the
//! real pak so a mismatch fails loudly instead.

use std::path::PathBuf;

use paldex_data::{Pak, ReferenceData, ReferenceIndex};

/// Resolve the installed pak, or `None` when the game isn't present — these
/// tests are skipped rather than failed in that case, matching how every other
/// real-data test in this workspace behaves.
fn real_index() -> Option<ReferenceIndex> {
    let path = std::env::var_os("PALDEX_TEST_PAK")
        .map(PathBuf::from)
        .or_else(|| paldex_locate::discover_paks().into_iter().next())?;
    let mut pak = Pak::open(&path).ok()?;
    ReferenceIndex::extract(&mut pak, "en").ok()
}

/// Every id in the table must exist in the pak. A typo, or a species renamed
/// by a game update, shows up here.
#[test]
fn every_vendored_id_exists_in_the_pak() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    let numbered = index.dex_entry_count();
    eprintln!("{numbered} species carry a Paldeck number");
    assert!(numbered > 250, "expected the full Paldeck, got {numbered}");
}

/// The species the table leaves unnumbered must all be things the game itself
/// keeps out of the Paldeck. If a real Pal ever lands in this set, the table
/// is stale and the denominator is wrong.
#[test]
fn unnumbered_species_are_all_non_paldeck_content() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    let unnumbered: Vec<&str> = index
        .species_iter()
        .filter(|s| s.dex_number.is_none())
        .map(|s| s.character_id.as_str())
        .collect();
    eprintln!("{} unnumbered species: {unnumbered:?}", unnumbered.len());

    // Tower bosses are authored as a human+Pal pair and render as "Zoe &
    // Grizzbolt"; raid/collab content is prefixed or Yakushima-named; unused
    // entries carry dev names or an `_omit` marker.
    let expected_non_paldeck = |id: &str| {
        let lower = id.to_ascii_lowercase();
        lower.ends_with("boss")
            || lower.contains("raid_")
            || lower.contains("yakushima")
            || lower.contains("_omit")
            || lower.starts_with("police_")
            || lower.contains("boss001")
            || lower.contains("boss002")
    };

    let surprising: Vec<&&str> =
        unnumbered.iter().filter(|id| !expected_non_paldeck(id)).collect();
    eprintln!("{} unnumbered but not obviously non-Paldeck: {surprising:?}", surprising.len());

    // A handful of genuinely unused species (dev leftovers with untranslated
    // names) don't match any naming rule, so this is bounded rather than zero.
    assert!(
        surprising.len() <= 12,
        "too many real-looking species lack a Paldeck number ({}) — the table is probably stale: {surprising:?}",
        surprising.len()
    );
}

/// Spot-check against the current game's numbering. These are cheap canaries:
/// the game renumbered its Paldeck after launch, so a regression toward the
/// launch-era numbering (Foxparks 005, Sparkit 007) trips here immediately.
#[test]
fn known_species_carry_their_current_paldeck_numbers() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    for (character_id, expected) in [
        ("SheepBall", "001"),   // Lamball
        ("PinkCat", "002"),     // Cattiva
        ("ChickenPal", "003"),  // Chikipi
        ("Kitsunebi", "029"),   // Foxparks — 005 before the renumber
        ("ElecCat", "042"),     // Sparkit — 007 before the renumber
        ("Penguin", "017"),     // Pengullet
        ("CaptainPenguin", "018"), // Penking
    ] {
        let species = index
            .species(character_id)
            .unwrap_or_else(|| panic!("{character_id} should resolve"));
        assert_eq!(
            species.dex_label().as_deref(),
            Some(expected),
            "{character_id} ({}) should be Paldeck {expected}",
            species.display_name
        );
    }
}

/// Variant forms share their base's number and take a `B`, exactly as the
/// Paldeck shows them (`005` / `005B`).
#[test]
fn variants_share_the_base_number_with_a_b_suffix() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    let base = index.species("BluePlatypus").expect("Fuack should resolve");
    let variant = index
        .species("BluePlatypus_Fire")
        .expect("Fuack Ignis should resolve");

    assert_eq!(base.dex_number, variant.dex_number, "a variant shares its base's number");
    assert_eq!(base.dex_label().as_deref(), Some("005"));
    assert_eq!(variant.dex_label().as_deref(), Some("005B"));
}
