//! Guards the Paldeck numbers now read from the user's own pak.
//!
//! `ZukanIndex` is a numeric `DataTable` field, so it only became readable once
//! the bundled `Mappings.usmap` landed; before that a vendored table stood in
//! for it. These tests are the descendants of the ones that guarded that table,
//! and they exist for the same reason: drift here is invisible, because a wrong
//! number still renders a plausible-looking grid with a wrong denominator.
//!
//! The numbers these assert were verified to reproduce the retired vendored
//! table exactly — 288 entries, zero disagreements — before it was deleted.

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

/// The whole Paldeck should carry a number. A parameter table that moved or a
/// schema that stopped matching shows up here as a collapsed count.
#[test]
fn the_full_paldeck_carries_numbers() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    assert!(index.warnings().is_empty(), "extraction warnings: {:?}", index.warnings());

    let numbered = index.dex_entry_count();
    eprintln!("{numbered} species carry a Paldeck number");
    assert!(numbered > 250, "expected the full Paldeck, got {numbered}");
}

/// The species left unnumbered must all be things the game itself keeps out of
/// the Paldeck. If a real Pal lands in this set, the denominator is wrong.
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
        "too many real-looking species lack a Paldeck number ({}): {surprising:?}",
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
        ("SheepBall", "001"),      // Lamball
        ("PinkCat", "002"),        // Cattiva
        ("ChickenPal", "003"),     // Chikipi
        ("Kitsunebi", "029"),      // Foxparks — 005 before the renumber
        ("ElecCat", "042"),        // Sparkit — 007 before the renumber
        ("Penguin", "017"),        // Pengullet
        ("CaptainPenguin", "018"), // Penking
        ("Anubis", "139"),
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

/// The alpha form of a species stores `ZukanIndex = -1`, and normalizes onto
/// the same key as its base. Applying rows in file order let it erase the base
/// form's number for 36 species; this pins the precedence that fixed it.
#[test]
fn alpha_forms_do_not_erase_their_base_species_number() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    for (id, expected) in [("AmaterasuWolf", 111), ("Yeti", 134), ("SifuDog", 155)] {
        let species = index.species(id).unwrap_or_else(|| panic!("{id} should resolve"));
        assert_eq!(
            species.dex_number,
            Some(expected),
            "{id} lost its Paldeck number to a variant row"
        );
        // The alpha resolves to the same species entry, so it reads the same.
        assert_eq!(
            index.species(&format!("BOSS_{id}")).and_then(|s| s.dex_number),
            Some(expected)
        );
    }
}
