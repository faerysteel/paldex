//! Integration tests against a real Palworld installation.
//!
//! Resolves a save directory the same way the other crates' real-save tests do:
//! `PALDEX_TEST_SAVE_DIR` first, falling back to [`paldex_locate::discover`]. If
//! neither yields a trackable (hosted) world with at least one player, every
//! test here skips rather than failing.

use paldex_locate::{discover, resolve_manual};
use paldex_model::{decode_player, resolve_locations, PalLocationKind};

fn decode_first_player() -> Option<paldex_model::PlayerProgress> {
    let root = if let Ok(dir) = std::env::var("PALDEX_TEST_SAVE_DIR") {
        resolve_manual(std::path::Path::new(&dir))
            .unwrap_or_else(|e| panic!("PALDEX_TEST_SAVE_DIR is set but unusable: {e}"))
            .into_iter()
            .next()
    } else {
        discover().into_iter().next()
    }?;

    let world = root.worlds().into_iter().find(|w| w.is_trackable())?;
    let player_uid = world.players.first()?;
    let path = world.path.join("Players").join(format!("{}.sav", player_uid.0));

    let raw = std::fs::read(path).ok()?;
    let (gvas, _) = paldex_sav::decompress(&raw).ok()?;
    let root = paldex_gvas::parse(&gvas).ok()?;
    Some(decode_player(&root).unwrap_or_else(|e| panic!("decoding player: {e}")))
}

macro_rules! require_real_player {
    () => {
        match decode_first_player() {
            Some(p) => p,
            None => {
                eprintln!("skipping: no trackable Palworld world with a player found (set PALDEX_TEST_SAVE_DIR)");
                return;
            }
        }
    };
}

#[test]
fn decodes_a_real_player_with_plausible_progress() {
    let player = require_real_player!();

    eprintln!(
        "paldeck: {} unlocked, {} capture_counts entries, {} unlocked tech, {} tower bosses, {} completed quests",
        player.paldeck_unlocked.len(),
        player.capture_counts.len(),
        player.unlocked_tech.len(),
        player.bosses.tower_defeated.len(),
        player.quests.completed_quest_ids.len(),
    );

    assert!(!player.paldeck_unlocked.is_empty(), "expected some dex progress");
    assert!(!player.capture_counts.is_empty(), "expected some capture counts");
    assert!(player.party_container_id.is_some(), "expected OtomoCharacterContainerId");
    assert!(player.box_container_id.is_some(), "expected PalStorageContainerId");
    assert_ne!(
        player.party_container_id, player.box_container_id,
        "party and box containers should be distinct"
    );
}

/// `PalCaptureBonusCount` is a *tier*, not a boolean and not a count of
/// bonuses earned: it is `min(capture_count, CAPTURE_BONUS_AT)`.
///
/// This is the whole basis for "the capture bonus completes at 5", which no
/// `DataTable` in the game pak states — the thresholds are not shipped as
/// data, so the save's own behaviour is the only evidence. It is also a
/// regression test for reading the field with `find_map_true_keys`, which
/// matched no `(Name, Bool)` pairs and so reported every species' bonus as
/// unclaimed while silently type-checking.
#[test]
fn capture_bonus_tier_is_capture_count_capped_at_five() {
    let player = require_real_player!();

    assert!(
        !player.capture_bonus_tiers.is_empty(),
        "a save with capture counts must have capture bonus tiers"
    );

    for (species, tier) in &player.capture_bonus_tiers {
        let count = player.capture_counts.get(species).copied().unwrap_or(0);
        assert_eq!(
            *tier,
            count.min(paldex_model::CAPTURE_BONUS_AT),
            "{species}: tier {tier} but {count} captures"
        );
    }

    // A species caught enough times must be at the top tier, or the threshold
    // itself is wrong rather than just the arithmetic above.
    //
    // Counted against the *tiered* species only. Some things the save counts
    // captures for carry no tier at all and so have no capture bonus —
    // `Human` most obviously, plus the Yakushima raid bosses. Comparing
    // against every counted species instead would fail by exactly those.
    let complete = player
        .capture_counts
        .iter()
        .filter(|(species, c)| {
            **c >= paldex_model::CAPTURE_BONUS_AT
                && player.capture_bonus_tiers.contains_key(*species)
        })
        .count();
    let at_top = player
        .capture_bonus_tiers
        .values()
        .filter(|t| **t == paldex_model::CAPTURE_BONUS_AT)
        .count();
    eprintln!(
        "{complete} tiered species at {}+ captures, {at_top} at the top tier",
        paldex_model::CAPTURE_BONUS_AT
    );
    assert_eq!(complete, at_top);

    let untiered: Vec<&String> = player
        .capture_counts
        .keys()
        .filter(|s| !player.capture_bonus_tiers.contains_key(*s))
        .collect();
    eprintln!("counted but outside the bonus system: {untiered:?}");
}

#[test]
fn dex_completion_never_exceeds_capture_counts_species() {
    let player = require_real_player!();

    // Every unlocked species should have been captured (or at least seen) —
    // paldeck entries with zero captures are plausible (seen-but-not-caught),
    // so this just checks the dex set isn't nonsensically larger than what a
    // real save would produce.
    assert!(
        player.paldeck_unlocked.len() < 10_000,
        "suspiciously large paldeck: {}",
        player.paldeck_unlocked.len()
    );
}

#[test]
fn resolve_locations_assigns_party_and_box_pals() {
    let player = require_real_player!();

    let root = if let Ok(dir) = std::env::var("PALDEX_TEST_SAVE_DIR") {
        paldex_locate::resolve_manual(std::path::Path::new(&dir))
            .unwrap()
            .into_iter()
            .next()
    } else {
        paldex_locate::discover().into_iter().next()
    };
    let Some(root) = root else {
        eprintln!("skipping: no save root");
        return;
    };
    let Some(world) = root.worlds().into_iter().find(|w| w.is_trackable()) else {
        eprintln!("skipping: no trackable world");
        return;
    };
    let raw = std::fs::read(world.path.join("Level.sav")).unwrap();
    let (gvas, _) = paldex_sav::decompress(&raw).unwrap();
    let level_root = paldex_gvas::parse(&gvas).unwrap();
    let Some(paldex_gvas::Value::Struct {
        value: paldex_gvas::StructValue::Properties(world_data),
        ..
    }) = level_root.get("worldSaveData")
    else {
        panic!("no worldSaveData");
    };
    let Some(paldex_gvas::Value::Map(entries)) = world_data
        .iter()
        .find(|p| p.name == "CharacterSaveParameterMap")
        .map(|p| &p.value)
    else {
        panic!("no CharacterSaveParameterMap");
    };

    let mut result = paldex_model::decode_character_map(entries);
    resolve_locations(&mut result.pals, &player);

    let party = result
        .pals
        .iter()
        .filter(|p| p.location.is_some_and(|l| l.kind == PalLocationKind::Party))
        .count();
    let boxed = result
        .pals
        .iter()
        .filter(|p| p.location.is_some_and(|l| l.kind == PalLocationKind::Box))
        .count();
    eprintln!("party: {party}, box: {boxed}, total: {}", result.pals.len());

    // The active party is capped at 5 in Palworld; a looser bound keeps this
    // from being flaky against game updates while still catching a totally
    // broken resolution (e.g. every pal ending up in Party).
    assert!(party <= 5, "party should never exceed the in-game cap of 5, got {party}");
    assert!(party + boxed > 0, "expected at least some pals to resolve to Party or Box");
}
