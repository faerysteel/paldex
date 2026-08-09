//! Integration tests for the `.usmap`-driven `DataTable` reader against the
//! real installed pak.
//!
//! These cover what unversioned property serialization hid until the mappings
//! file landed: base stats, elements, work suitabilities and rarity.
//!
//! The strongest assertion here is not any individual value but
//! [`decodes_both_parameter_tables_exactly`]. Unversioned values are untagged,
//! so one wrong field size desynchronizes every byte after it — a decode that
//! lands exactly on the export's declared length effectively cannot be wrong.

use std::path::PathBuf;

use paldex_data::datatable;
use paldex_data::unversioned::Value;
use paldex_data::usmap::Usmap;
use paldex_data::{Pak, ReferenceData, ReferenceIndex};

const MAPPINGS: &[u8] = include_bytes!("../data/Mappings.usmap");

const MONSTER_TABLES: &[&str] = &[
    "Pal/Content/Pal/DataTable/Character/DT_PalMonsterParameter",
    "Pal/Content/Pal/DataTable/Character/DT_PalMonsterParameter_Common",
];

fn real_pak() -> Option<Pak> {
    let path = std::env::var_os("PALDEX_TEST_PAK")
        .map(PathBuf::from)
        .or_else(|| paldex_locate::discover_paks().into_iter().next())?;
    Pak::open(&path).ok()
}

fn real_index() -> Option<ReferenceIndex> {
    let mut pak = real_pak()?;
    ReferenceIndex::extract(&mut pak, "en").ok()
}

/// The bundled mappings must match what `tools/usmap/regen-usmap.sh` produces.
///
/// Counts rather than a hash: the dumper emits its name table in hash-map
/// order, so two good runs differ in sha256 but never in these totals.
#[test]
fn bundled_usmap_parses_with_the_expected_shape() {
    let usmap = Usmap::parse(MAPPINGS).expect("bundled Mappings.usmap should parse");
    assert_eq!(usmap.name_count(), 54_109);
    assert_eq!(usmap.enum_count(), 2_099);

    // An all-empty enum table is the signature of a wrong `UEnum::Names()`
    // offset in the dumper — it still produces a structurally valid file.
    assert_eq!(
        usmap.enum_entries("EPalElementType").map(<[String]>::len),
        Some(11),
        "element enum should be populated"
    );
    assert_eq!(usmap.enum_value("EPalElementType", 2), Some("Fire"));

    // `EInterpCurveMode` is an engine enum whose values are known independently
    // of Palworld, so decoding it right is evidence rather than coincidence.
    assert_eq!(usmap.enum_value("EInterpCurveMode", 0), Some("CIM_Linear"));

    let row = usmap
        .get_struct("PalCharacterParameterDatabaseRow")
        .expect("the row schema must be present");
    assert_eq!(row.super_name.as_deref(), Some("TableRowBase"));
    assert_eq!(row.property_count, 90);
}

/// The exactness gate: both real tables must decode to the byte.
#[test]
fn decodes_both_parameter_tables_exactly() {
    let Some(mut pak) = real_pak() else {
        eprintln!("skipping: no game pak found");
        return;
    };
    let usmap = Usmap::parse(MAPPINGS).expect("usmap");

    for base in MONSTER_TABLES {
        let uasset = pak.read(&format!("{base}.uasset")).expect("read uasset");
        let uexp = pak.read(&format!("{base}.uexp")).expect("read uexp");

        // `read` itself errors unless the walk ends on the exact declared
        // length, so reaching `Ok` here is the assertion.
        let table = datatable::read(&uasset, &uexp, &usmap)
            .unwrap_or_else(|e| panic!("decoding {base}: {e}"));

        assert_eq!(table.row_struct, "PalCharacterParameterDatabaseRow");
        assert!(table.rows.len() > 500, "{base}: only {} rows", table.rows.len());
        eprintln!("{base}: {} rows", table.rows.len());
    }
}

/// Every row must expose the fields the tracker depends on, with values in
/// range. A schema drift that shifted properties by one slot would still
/// decode to the exact length but produce nonsense here.
#[test]
fn decoded_rows_have_plausible_values_throughout() {
    let Some(mut pak) = real_pak() else {
        eprintln!("skipping: no game pak found");
        return;
    };
    let usmap = Usmap::parse(MAPPINGS).expect("usmap");
    let uasset = pak.read(&format!("{}.uasset", MONSTER_TABLES[0])).expect("uasset");
    let uexp = pak.read(&format!("{}.uexp", MONSTER_TABLES[0])).expect("uexp");
    let table = datatable::read(&uasset, &uexp, &usmap).expect("decode");

    let elements: Vec<&str> = usmap
        .enum_entries("EPalElementType")
        .expect("element enum")
        .iter()
        .map(String::as_str)
        .collect();

    let mut numbered = 0;
    for row in &table.rows {
        let dex = row.properties.get("ZukanIndex").and_then(Value::as_i32);
        assert!(dex.is_some(), "{} has no ZukanIndex", row.name);
        if dex.is_some_and(|d| d > 0) {
            numbered += 1;
            assert!(dex.unwrap() < 1000, "{}: implausible dex {dex:?}", row.name);
        }

        // Every Pal has positive HP; a desync would show up as zero or wild.
        let hp = row.properties.get("Hp").and_then(Value::as_i32).unwrap_or(0);
        assert!((1..=10_000).contains(&hp), "{}: implausible Hp {hp}", row.name);

        // Elements must decode to real enum entries, not raw integers.
        for key in ["ElementType1", "ElementType2"] {
            if let Some(value) = row.properties.get(key) {
                let name = value.as_str().unwrap_or_else(|| {
                    panic!("{}: {key} decoded as {value:?}, not an enum entry", row.name)
                });
                assert!(
                    name.is_empty() || elements.contains(&name),
                    "{}: {key} = {name:?} is not an EPalElementType",
                    row.name
                );
            }
        }
    }
    eprintln!("{numbered}/{} rows carry a Paldeck number", table.rows.len());
    assert!(numbered > 250);
}

/// Spot-checks against what the game shows in its own Paldeck. These are the
/// values the vendored table could never supply.
#[test]
fn species_stats_match_the_in_game_paldeck() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    let anubis = index.species("Anubis").expect("Anubis should resolve");
    assert_eq!(anubis.elements, vec!["Earth"]);
    assert_eq!(anubis.work_suitabilities.get("Handcraft"), Some(&6));
    assert_eq!(anubis.work_suitabilities.get("Mining"), Some(&6));
    assert_eq!(anubis.work_suitabilities.get("Transport"), Some(&4));
    // A job it cannot do must be absent rather than present-and-zero.
    assert!(!anubis.work_suitabilities.contains_key("Watering"));
    assert!(anubis.stats.hp > 0 && anubis.stats.melee_attack > 0);

    // Lamball: the starter, Neutral, with a small spread of jobs.
    let lamball = index.species("SheepBall").expect("Lamball should resolve");
    assert_eq!(lamball.elements, vec!["Normal"]);
    assert!(lamball.work_suitabilities.contains_key("Handcraft"));

    // Penking is dual-element — the second slot must survive and must not be
    // the enum's "None" placeholder.
    let penking = index.species("CaptainPenguin").expect("Penking should resolve");
    assert_eq!(penking.elements.len(), 2, "Penking is dual-element: {:?}", penking.elements);
    assert!(!penking.elements.iter().any(|e| e == "None" || e.is_empty()));
}

/// Across the whole index the derived fields must be broadly populated — a
/// silent regression to all-defaults would still pass the spot checks above if
/// they happened to target the surviving rows.
#[test]
fn derived_fields_are_populated_across_the_index() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    let numbered: Vec<_> = index.species_iter().filter(|s| s.dex_number.is_some()).collect();
    assert!(numbered.len() > 250);

    let with_element = numbered.iter().filter(|s| !s.elements.is_empty()).count();
    let with_stats = numbered.iter().filter(|s| s.stats.hp > 0).count();
    let with_work = numbered.iter().filter(|s| !s.work_suitabilities.is_empty()).count();
    let with_rarity = numbered.iter().filter(|s| s.rarity > 0).count();
    eprintln!(
        "of {} numbered species: {with_element} have elements, {with_stats} stats, \
         {with_work} work suitabilities, {with_rarity} rarity",
        numbered.len()
    );

    assert_eq!(with_stats, numbered.len(), "every Paldeck species has base stats");
    assert_eq!(with_rarity, numbered.len(), "every Paldeck species has a rarity");
    assert_eq!(
        with_element,
        numbered.len() - ELEMENTLESS_SPECIES.len(),
        "only the known elementless species may lack an element"
    );
    // A few species genuinely cannot work (raid-only forms that still carry a
    // number), so this is a strong majority rather than all.
    assert!(with_work * 100 / numbered.len() >= 90);
}

/// The shipped data leaves `ElementType1` unwritten for eight rows — every
/// `WorldTreeDragon` form and the Yakushima raid bosses — so their element
/// reads as the enum's `None`. Only this one carries a Paldeck number, so it
/// is the sole legitimate exception among numbered species. It is named rather
/// than tolerated by a fuzzy threshold: if a *second* species ever turns up
/// elementless, that is a decode regression and should fail.
const ELEMENTLESS_SPECIES: &[&str] = &["WorldTreeDragon"];

/// Dual-element species must keep both slots, and single-element species must
/// drop the `None` placeholder the second slot always holds.
#[test]
fn element_slots_are_normalized() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    let numbered: Vec<_> = index.species_iter().filter(|s| s.dex_number.is_some()).collect();

    let elementless: Vec<&str> = numbered
        .iter()
        .filter(|s| s.elements.is_empty())
        .map(|s| s.character_id.as_str())
        .collect();
    assert_eq!(
        elementless, ELEMENTLESS_SPECIES,
        "unexpected elementless species — likely a decode regression"
    );

    assert!(
        numbered.iter().all(|s| s.elements.len() <= 2),
        "no species has more than two elements"
    );
    assert!(
        numbered.iter().any(|s| s.elements.len() == 2),
        "some species are dual-element"
    );
    assert!(
        !numbered.iter().any(|s| s.elements.iter().any(|e| e == "None")),
        "the None placeholder must be stripped"
    );
}

/// The enum ids the parameter table stores are internal names, and the UI shows
/// something else for three of the nine elements. Every id actually in use must
/// resolve, or the dex would show `Leaf` where the game says `Grass`.
#[test]
fn every_element_in_use_has_a_label() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    let mut unresolved: Vec<&str> = index
        .species_iter()
        .flat_map(|s| s.elements.iter())
        .filter(|id| index.element_name(id).is_none())
        .map(String::as_str)
        .collect();
    unresolved.sort_unstable();
    unresolved.dedup();
    assert!(unresolved.is_empty(), "elements with no UI label: {unresolved:?}");

    // The three that differ from their enum name are the whole reason this
    // lookup exists, so pin them rather than only asserting non-emptiness.
    assert_eq!(index.element_name("Leaf"), Some("Grass"));
    assert_eq!(index.element_name("Earth"), Some("Ground"));
    assert_eq!(index.element_name("Electricity"), Some("Electric"));
    assert_eq!(index.element_name("Normal"), Some("Neutral"));
    assert_eq!(index.element_name("Fire"), Some("Fire"));
}

/// Same contract for work suitabilities, plus the display order the Paldeck
/// uses — which is the UI table's order, not alphabetical.
#[test]
fn every_work_suitability_in_use_has_a_label_and_an_order() {
    let Some(index) = real_index() else {
        eprintln!("skipping: no game pak found");
        return;
    };

    let mut unresolved: Vec<&str> = index
        .species_iter()
        .flat_map(|s| s.work_suitabilities.keys())
        .filter(|id| index.work_suitability_name(id).is_none())
        .map(String::as_str)
        .collect();
    unresolved.sort_unstable();
    unresolved.dedup();
    assert!(unresolved.is_empty(), "work suitabilities with no UI label: {unresolved:?}");

    assert_eq!(index.work_suitability_name("Handcraft"), Some("Handiwork"));
    assert_eq!(index.work_suitability_name("EmitFlame"), Some("Kindling"));
    assert_eq!(index.work_suitability_name("Deforest"), Some("Lumbering"));

    // Every id in use must be placeable in the display order, otherwise it
    // would silently sort to the end of the icon row.
    let order = index.work_suitability_order();
    let missing: Vec<&str> = index
        .species_iter()
        .flat_map(|s| s.work_suitabilities.keys())
        .filter(|id| !order.iter().any(|o| o.eq_ignore_ascii_case(id)))
        .map(String::as_str)
        .collect();
    assert!(missing.is_empty(), "work suitabilities missing from the display order: {missing:?}");

    // Kindling leads the row in game and Farming closes it; alphabetical order
    // would put Collection first, so this also proves the order is the table's.
    let position = |id: &str| order.iter().position(|o| o == id).expect("id in order");
    assert!(position("EmitFlame") < position("Watering"));
    assert!(position("Watering") < position("Seeding"));
    assert!(position("Transport") < position("MonsterFarm"));
    assert!(position("EmitFlame") < position("Collection"));
}

/// Labels come from the localized table, so a non-English extraction must give
/// non-English labels — otherwise the lookup is silently reading the source
/// language for everyone.
#[test]
fn labels_follow_the_selected_language() {
    let Some(mut pak) = real_pak() else {
        eprintln!("skipping: no game pak found");
        return;
    };
    let Ok(index) = ReferenceIndex::extract(&mut pak, "ja") else {
        eprintln!("skipping: no Japanese text tables");
        return;
    };

    let leaf = index.element_name("Leaf").expect("Leaf should resolve in Japanese");
    assert_ne!(leaf, "Grass", "Japanese extraction returned the English label");
    assert!(!leaf.is_empty());
}
