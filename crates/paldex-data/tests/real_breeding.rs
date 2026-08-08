//! Integration tests for breeding against the real installed pak.
//!
//! Everything the pak determines is asserted exactly. The rank tie-break,
//! which the pak does *not* determine, is pinned by two results confirmed by
//! breeding the pairs in game — see the [`paldex_data::breeding`] module docs
//! for how those two pairs between them eliminate every competing rule.

use std::path::PathBuf;

use paldex_data::{Pak, ReferenceData, ReferenceIndex};

fn real_index() -> Option<ReferenceIndex> {
    let path = std::env::var_os("PALDEX_TEST_PAK")
        .map(PathBuf::from)
        .or_else(|| paldex_locate::discover_paks().into_iter().next())?;
    let mut pak = Pak::open(&path).ok()?;
    ReferenceIndex::extract(&mut pak, "en").ok()
}

macro_rules! index {
    () => {
        match real_index() {
            Some(i) => i,
            None => {
                eprintln!("skipping: no game pak found");
                return;
            }
        }
    };
}

/// Unique combos override everything, and are order-independent. These pairs
/// are checkable against the in-game breeding farm.
#[test]
fn unique_combos_produce_their_documented_child() {
    let index = index!();

    for (a, b, child) in [
        ("LazyDragon", "ElecCat", "LazyDragon_Electric"), // Relaxaurus + Sparkit
        ("Baphomet", "GhostBeast", "Baphomet_Dark"),      // Incineram + Maraith
        ("GrassMammoth", "Yeti", "GrassMammoth_Ice"),     // Mammorest + Wumpo
        ("Bastet", "Penguin", "Bastet_Ice"),              // Katress  + Pengullet
    ] {
        assert_eq!(
            index.breeding_result(a, b),
            Some(child),
            "{a} + {b} should give {child}"
        );
        assert_eq!(
            index.breeding_result(b, a),
            Some(child),
            "{b} + {a} should give the same child — order must not matter"
        );
    }
}

/// The invariant that pins the target formula: breeding a species with itself
/// always returns that species. It holds for every candidate, so a single
/// counter-example means the formula or the candidate pool is wrong.
#[test]
fn self_breeding_is_a_fixed_point_for_every_species() {
    let index = index!();

    let mut checked = 0;
    let mut failures = Vec::new();
    for species in index.species_iter() {
        let id = &species.character_id;
        // Only species that are themselves breeding candidates.
        let Some(child) = index.breeding_result(id, id) else {
            continue;
        };
        checked += 1;
        if !child.eq_ignore_ascii_case(id) {
            failures.push(format!("{id} + {id} = {child}"));
        }
    }
    eprintln!(
        "{checked} species checked for the self-breeding fixed point, {} exceptions: {:?}",
        failures.len(),
        &failures[..failures.len().min(12)]
    );
    assert!(
        checked > 200,
        "expected most species to be breedable, got {checked}"
    );

    // The only species that break it are ones the game will not let into a
    // breeding farm at all — raid bosses and the tower boss. They carry
    // `IgnoreCombi`, so they are excluded from the candidate pool and the
    // generic rule falls through to whatever rank is nearest. Naming them
    // keeps this exact: a fifth exception is a real regression.
    let mut broke: Vec<String> = failures
        .iter()
        .map(|f| f.split(' ').next().unwrap_or_default().to_owned())
        .collect();
    broke.sort();
    assert_eq!(
        broke,
        [
            "KingWhale",
            "RAID_YakushimaBoss001_Green",
            "RAID_YakushimaBoss002",
            "WorldTreeDragon",
        ],
        "unexpected self-breeding failures: {failures:?}"
    );
}

/// Base species — the tribe representatives — must be exact fixed points.
#[test]
fn base_species_breed_true() {
    let index = index!();

    for id in [
        "SheepBall",
        "PinkCat",
        "ChickenPal",
        "Anubis",
        "Penguin",
        "CaptainPenguin",
        "Kitsunebi",
    ] {
        assert_eq!(
            index.breeding_result(id, id).map(str::to_ascii_lowercase),
            Some(id.to_ascii_lowercase()),
            "{id} bred with itself should give {id}"
        );
    }
}

/// Unknown or non-breedable parents must return `None` rather than a guess.
#[test]
fn unknown_parents_return_nothing() {
    let index = index!();
    assert_eq!(index.breeding_result("NotAPal", "AlsoNotAPal"), None);
    assert_eq!(index.breeding_result("SheepBall", "NotAPal"), None);
}

/// Confirmed in game: Lamball + Chikipi produces **Vixy**.
///
/// Target rank 3065 leaves Vixy (`CuteFox`, rank 3060, Paldeck 6) and Teafant
/// (`Ganesha`, rank 3070, Paldeck 11) both at distance 5. Vixy winning
/// disproves two candidate rules outright — "highest `CombiRank`" and "first
/// row in the table", which both predict Teafant (rows 436 vs 362).
///
/// This is ground truth, so it is asserted unconditionally: any tie-break rule
/// that breaks it is wrong regardless of how appealing it looks.
#[test]
fn breeding_lamball_chikipi_is_vixy() {
    let index = index!();
    assert_eq!(index.breeding_result("SheepBall", "ChickenPal"), Some("CuteFox"));
}

/// Confirmed in game: Lamball + Fuack produces **Lifmunk**.
///
/// Target rank 3015 ties Sparkit (`ElecCat`, rank 3010, Paldeck 42) with
/// Lifmunk (`Carbunclo`, rank 3020, Paldeck 4). Lifmunk is the *higher*-ranked
/// of the two, which is what rules out "lowest `CombiRank` wins" — the only
/// rank-based rule still standing after the Vixy result above.
///
/// Together the two confirmed pairs leave the lower Paldeck number as the sole
/// surviving explanation.
#[test]
fn breeding_lamball_fuack_is_lifmunk() {
    let index = index!();
    assert_eq!(
        index.breeding_result("SheepBall", "BluePlatypus"),
        Some("Carbunclo")
    );
}

/// A tie whose candidates order the same way under both `CombiRank` and
/// `ZukanIndex`, kept as an explicit contrast to the two confirmed pairs.
///
/// Lamball + Cattiva targets rank 2905, leaving Monkey (Tanzee, 2900,
/// Paldeck 23) and DreamDemon (Daedream, 2910, Paldeck 22) both at distance 5.
/// Daedream is the higher-ranked *and* the lower-numbered candidate, so this
/// pair agrees with several rules at once — it is kept as a reminder that a
/// passing tie-break test proves nothing unless the pair was chosen to
/// discriminate.
#[test]
fn a_tie_that_cannot_discriminate() {
    let index = index!();
    assert_eq!(
        index.breeding_result("SheepBall", "PinkCat"),
        Some("DreamDemon")
    );
}
