//! Decodes a `Players/<uid>.sav` file into [`PlayerProgress`].
//!
//! Unlike [`crate::rawdata::character`], this isn't a `RawData` blob nested
//! inside `Level.sav` — it's its own top-level GVAS file, one per player, with
//! the interesting data hanging off a `SaveData` → `RecordData` struct.
//! Verified against a real save via `crates/paldex-gvas/examples/dump_player.rs`.

use paldex_gvas::Value;

use crate::gvas_ext::{
    find, find_container_id, find_guid, find_int, find_map_int, find_map_true_keys,
    find_name_array, find_struct_properties, struct_properties, u32_or_zero,
};
use crate::types::{BossFlags, Collectibles, MiscCounters, PlayerProgress, QuestState};

/// Decode an already-parsed `Players/<uid>.sav`'s [`paldex_gvas::Root`].
///
/// # Errors
///
/// Returns an error string (never panics) if `SaveData` or `RecordData` is
/// missing or not the expected generic-struct shape — a save this reader
/// can't make sense of at all, as opposed to individual fields within it
/// being absent, which is normal and handled per-field.
pub fn decode_player(root: &paldex_gvas::Root) -> Result<PlayerProgress, String> {
    let save_data = match root.get("SaveData") {
        Some(v) => struct_properties(v).ok_or("SaveData is not a generic struct")?,
        None => return Err("no SaveData field".to_owned()),
    };
    let player_uid = find_guid(save_data, "PlayerUId").ok_or("SaveData missing PlayerUId")?;
    let record = find_struct_properties(save_data, "RecordData").unwrap_or(&[]);

    Ok(PlayerProgress {
        player_uid,
        paldeck_unlocked: find_map_true_keys(record, "PaldeckUnlockFlag"),
        capture_counts: find_map_int(record, "PalCaptureCount"),
        // A `Map<Name, Int>` tier, not a bool set — reading it with
        // `find_map_true_keys` silently yielded an always-empty set.
        capture_bonus_tiers: find_map_int(record, "PalCaptureBonusCount"),
        unlocked_tech: find_name_array(save_data, "UnlockedRecipeTechnologyNames"),
        tech_points: u32_or_zero(find_int(save_data, "TechnologyPoint")),
        boss_tech_points: u32_or_zero(find_int(save_data, "bossTechnologyPoint")),
        fast_travel_unlocked: find_map_true_keys(record, "FastTravelPointUnlockFlag"),
        bosses: BossFlags {
            normal_defeated: find_map_true_keys(record, "NormalBossDefeatFlag"),
            tower_defeated: find_map_true_keys(record, "TowerBossDefeatFlag"),
            specific_defeated: find_map_true_keys(record, "SpecificBossDefeatFlag"),
            tower_defeat_counts: find_map_int(record, "TowerBossDefeatCount"),
            raid_defeat_counts: find_map_int(record, "RaidBossDefeatCount"),
            predator_defeat_count: u32_or_zero(find_int(record, "PredatorDefeatCount")),
        },
        collectibles: Collectibles {
            relics_obtained: find_map_true_keys(record, "RelicObtainForInstanceFlag"),
            relic_possess_counts: find_map_int(record, "RelicPossessNumMap"),
            relic_possess_total: u32_or_zero(find_int(record, "RelicPossessNum")),
            notes_obtained: find_map_true_keys(record, "NoteObtainForInstanceFlag"),
            treasures_found: u32_or_zero(find_int(record, "FoundTreasureCount")),
        },
        quests: QuestState {
            ordered_quest_ids: find_ordered_quest_names(save_data),
            // Verified Array<Name>, same shape as UnlockedRecipeTechnologyNames.
            completed_quest_ids: find_name_array(save_data, "CompletedQuestArray_FullRelease"),
        },
        stats: MiscCounters {
            pal_butcher_count: sum_map_int(record, "PalButcherCount"),
            pal_rankup_count: sum_map_int(record, "PalRankupCount"),
            mutation_count: u32_or_zero(find_int(record, "MutationCount")),
            awakening_count: u32_or_zero(find_int(record, "AwakeningCount")),
            camp_conquered_count: u32_or_zero(find_int(record, "CampConqueredCount")),
            oilrig_clear_count: u32_or_zero(find_int(record, "OilrigClearCount")),
            normal_dungeon_clear_count: u32_or_zero(find_int(record, "NormalDungeonClearCount")),
            fixed_dungeon_clear_count: u32_or_zero(find_int(record, "FixedDungeonClearCount")),
            tribe_capture_count: u32_or_zero(find_int(record, "TribeCaptureCount")),
        },
        party_container_id: find_container_id(save_data, "OtomoCharacterContainerId"),
        box_container_id: find_container_id(save_data, "PalStorageContainerId"),
    })
}

/// Sums a `Map<Name, Int>`'s values into one total — used for per-species
/// counters ([`MiscCounters`] tracks totals, not per-species breakdowns).
fn sum_map_int(props: &[paldex_gvas::Property], name: &str) -> u32 {
    find_map_int(props, name).values().sum()
}

/// `OrderedQuestArray_FullRelease`: `Array<Struct(PalOrderedQuestSaveData)>`,
/// each with a `QuestName: Name` field — unlike `CompletedQuestArray_FullRelease`,
/// which is a plain `Array<Name>`.
fn find_ordered_quest_names(save_data: &[paldex_gvas::Property]) -> Vec<String> {
    match find(save_data, "OrderedQuestArray_FullRelease") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| {
                let props = struct_properties(v)?;
                match find(props, "QuestName") {
                    Some(Value::Name(s)) => Some(s.clone()),
                    _ => None,
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}
