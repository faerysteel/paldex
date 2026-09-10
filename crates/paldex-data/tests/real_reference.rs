//! Integration tests joining pak-derived reference data against a real save.
//! Every distinct `CharacterID` in the fixture world must resolve.
//!
//! Both the pak and the save are resolved from the local install (or the
//! `PALDEX_TEST_PAK` / `PALDEX_TEST_SAVE_DIR` overrides). Tests skip rather
//! than fail when either is absent, so CI stays green.

use std::collections::BTreeSet;
use std::path::PathBuf;

use paldex_data::{Pak, ReferenceData, ReferenceIndex};
use paldex_gvas::{StructValue, Value};

fn real_pak_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("PALDEX_TEST_PAK") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let candidate = PathBuf::from(std::env::var("HOME").ok()?)
        .join("Library/Application Support/CrossOver/Bottles/Steam/drive_c/Program Files (x86)/Steam/steamapps/common/Palworld/Pal/Content/Paks/Pal-Windows.pak");
    candidate.is_file().then_some(candidate)
}

fn reference_index() -> Option<ReferenceIndex> {
    let mut pak = Pak::open(&real_pak_path()?).ok()?;
    ReferenceIndex::extract(&mut pak, "en").ok()
}

/// Every distinct `CharacterID` in the real world's character map.
fn save_character_ids() -> Option<BTreeSet<String>> {
    let root = if let Ok(dir) = std::env::var("PALDEX_TEST_SAVE_DIR") {
        paldex_locate::resolve_manual(std::path::Path::new(&dir)).ok()?.into_iter().next()
    } else {
        paldex_locate::discover().into_iter().next()
    }?;
    let world = root.worlds().into_iter().find(paldex_locate::World::is_trackable)?;
    let raw = std::fs::read(world.path.join("Level.sav")).ok()?;
    let (gvas, _) = paldex_sav::decompress(&raw).ok()?;
    let parsed = paldex_gvas::parse(&gvas).ok()?;

    let Some(Value::Struct { value: StructValue::Properties(world_data), .. }) =
        parsed.get("worldSaveData")
    else {
        return None;
    };
    let Some(Value::Map(entries)) =
        world_data.iter().find(|p| p.name == "CharacterSaveParameterMap").map(|p| &p.value)
    else {
        return None;
    };

    let result = paldex_model::decode_character_map(entries);
    Some(result.pals.into_iter().map(|p| p.character_id).collect())
}

macro_rules! require {
    ($e:expr, $what:expr) => {
        match $e {
            Some(v) => v,
            None => {
                eprintln!("skipping: {} unavailable", $what);
                return;
            }
        }
    };
}

#[test]
fn extracts_a_plausible_reference_index_from_the_real_pak() {
    let index = require!(reference_index(), "game pak");

    // Palworld ships a few hundred named species (base forms plus variants).
    // A wildly different number means the text-table parse desynced.
    assert!(
        (150..1000).contains(&index.species_count()),
        "species count {} outside the plausible range",
        index.species_count()
    );
    assert!(index.passive_count() > 100, "expected many skill names");
    assert!(index.item_count() > 500, "expected many item names");

    // Spot checks against names visible in-game. These are the mapping the
    // whole feature exists to provide: save-side internal id -> display name.
    for (id, expected) in [
        ("SheepBall", "Lamball"),
        ("PinkCat", "Cattiva"),
        ("AmaterasuWolf", "Kitsun"),
        ("BadCatgirl", "Nyafia"),
    ] {
        let got = index.species(id).map(|s| s.display_name.as_str());
        assert_eq!(got, Some(expected), "{id} should resolve to {expected}");
    }

    // Variant prefixes must normalize to the same species.
    assert_eq!(
        index.species("BOSS_AmaterasuWolf").map(|s| s.display_name.as_str()),
        Some("Kitsun")
    );
}

/// Every character identifier in the save must resolve against the pak.
#[test]
fn every_character_id_in_the_real_save_resolves() {
    let index = require!(reference_index(), "game pak");
    let ids = require!(save_character_ids(), "real save");

    assert!(!ids.is_empty(), "fixture world should contain characters");

    let unresolved: Vec<&String> = ids.iter().filter(|id| !index.is_known(id)).collect();
    eprintln!("{} distinct CharacterIDs in save", ids.len());
    eprintln!(
        "  pals: {}, human NPCs: {}",
        ids.iter().filter(|id| index.is_pal(id)).count(),
        ids.iter().filter(|id| index.is_human_npc(id)).count()
    );
    for id in &unresolved {
        eprintln!("  unresolved: {id}");
    }

    assert!(
        unresolved.is_empty(),
        "{} CharacterID(s) did not resolve against the pak — see list above",
        unresolved.len()
    );
}

/// Pak-derived classification separates known human NPCs from Pal species.
#[test]
fn known_human_npcs_are_classified_out_of_the_pal_roster() {
    let index = require!(reference_index(), "game pak");

    for id in ["Hunter_Rifle", "Viking", "Believer_CrossBow"] {
        assert!(index.is_human_npc(id), "{id} should classify as a human NPC");
        assert!(!index.is_pal(id), "{id} should not be a Pal");
    }
    for id in ["SheepBall", "AmaterasuWolf", "BOSS_Anubis"] {
        assert!(index.is_pal(id), "{id} should be a Pal");
        assert!(!index.is_human_npc(id), "{id} should not be an NPC");
    }
}

/// Icons must decode to real PNGs at the expected size, and be correctly
/// paired with their species rather than shifted.
#[test]
fn icons_decode_from_the_real_pak() {
    let path = require!(real_pak_path(), "game pak");
    let mut pak = Pak::open(&path).expect("open pak");
    let index = ReferenceIndex::extract(&mut pak, "en").expect("extract");

    assert!(index.icon_count() > 300, "expected a few hundred icons");

    for id in ["SheepBall", "PinkCat", "AmaterasuWolf", "Anubis"] {
        let icon_path = index.icon_path(id).unwrap_or_else(|| panic!("{id} should have an icon"));
        // The path must name the species it belongs to -- a shifted index
        // would silently show the wrong artwork.
        assert!(
            icon_path.to_ascii_lowercase().contains(&id.to_ascii_lowercase()),
            "{id} resolved to unrelated icon {icon_path}"
        );

        let png = paldex_data::load_icon_png(&mut pak, icon_path)
            .unwrap_or_else(|e| panic!("decoding {id}: {e}"));
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "{id} should be a PNG");
        // IHDR width/height are big-endian at a fixed offset.
        let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((w, h), (128, 128), "{id} icon should be 128x128");
    }

    // Variant prefixes share the base species' artwork.
    assert_eq!(index.icon_path("BOSS_SheepBall"), index.icon_path("SheepBall"));
}

/// Every distinct Pal species in the real save should have artwork.
#[test]
fn most_species_in_the_real_save_have_icons() {
    let path = require!(real_pak_path(), "game pak");
    let mut pak = Pak::open(&path).expect("open pak");
    let index = ReferenceIndex::extract(&mut pak, "en").expect("extract");
    let ids = require!(save_character_ids(), "real save");

    let pals: Vec<&String> = ids.iter().filter(|id| index.is_pal(id)).collect();
    let with_icon = pals.iter().filter(|id| index.icon_path(id).is_some()).count();
    eprintln!("{with_icon}/{} distinct Pal species have an icon", pals.len());
    assert!(
        with_icon * 100 / pals.len().max(1) >= 90,
        "expected most species to have artwork, got {with_icon}/{}",
        pals.len()
    );
}

/// Every language the pak ships should parse; a language with no rows means
/// the path convention or the parser is wrong for it.
#[test]
fn every_shipped_language_extracts() {
    let path = require!(real_pak_path(), "game pak");
    let mut pak = Pak::open(&path).expect("open pak");

    for lang in paldex_data::TEXT_LANGUAGES {
        let index = ReferenceIndex::extract(&mut pak, lang)
            .unwrap_or_else(|e| panic!("extracting {lang}: {e}"));
        assert!(
            index.species_count() > 100,
            "{lang}: only {} species",
            index.species_count()
        );
    }
}
