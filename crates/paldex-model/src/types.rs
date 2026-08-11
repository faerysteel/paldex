//! Domain types projected from the raw GVAS property tree.
//!
//! Field mappings and their source, verified against a real `Level.sav` (see
//! `crates/paldex-gvas/examples/survey_characters.rs`):
//!
//! | Field | Source property |
//! |---|---|
//! | `character_id` | `CharacterID` (`Name`), with `BOSS_`/`PREDATOR_` stripped |
//! | `is_boss` / `is_alpha` | `BOSS_`/`PREDATOR_` prefix on `CharacterID` |
//! | `owner` | `OwnerPlayerUId` (`Guid`); all-zero treated as absent |
//! | `level` | `Level` (`Byte`), absent ⇒ 1 |
//! | `rank` | `Rank` (`Byte`), absent ⇒ 1 |
//! | `souls` | `Rank_HP`/`Rank_Attack`/`Rank_Defence`/`Rank_CraftSpeed` (`Byte`), absent ⇒ 0 |
//! | `ivs` | `Talent_HP`/`Talent_Shot`/`Talent_Defense` (`Byte`), absent ⇒ 0 |
//! | `passives` | `PassiveSkillList` (`Array<Name>`) |
//! | `equipped_moves` | `EquipWaza` (`Array<Enum>`) |
//! | `mastered_moves` | `MasteredWaza` (`Array<Enum>`) |
//! | `gender` | `Gender` (`Enum<EPalGenderType>`) |
//! | `is_lucky` | `IsRarePal` (`Bool`), absent ⇒ false |
//! | `nickname` | `NickName` (`Str`), empty ⇒ `None` |
//! | `location` | `SlotId` (`Struct(PalCharacterSlotId)`) |
//!
//! `PREDATOR_` was not observed in this session's fixture world (predator raids
//! are a late-game, transient spawn), but is documented in the implementation
//! plan alongside `BOSS_` and handled the same way.
//!
//! [`PlayerProgress`] comes from a different file entirely — `Players/<uid>.sav`,
//! not `Level.sav` — verified via `crates/paldex-gvas/examples/dump_player.rs`.
//! Its `OtomoCharacterContainerId`/`PalStorageContainerId` fields are exactly the
//! `PalLocation::container_id` values a Pal's `SlotId` carries, confirmed by a
//! byte-for-byte GUID match against a real roster entry — that's how `Pal.location`
//! gets resolved to Party/Box rather than staying an opaque container UUID.

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use uuid::Uuid;

/// The three Pal individual values. Palworld has three, not four —
/// `Talent_Melee` does not exist in current saves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Ivs {
    pub hp: u8,
    pub shot: u8,
    pub defense: u8,
}

/// Souls-condensing point allocation (`Rank_*` fields).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct SoulUpgrades {
    pub hp: u8,
    pub attack: u8,
    pub defense: u8,
    pub craft_speed: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum Gender {
    Male,
    Female,
    Unknown,
}

/// Where a Pal sits, resolved against a specific player's container IDs.
///
/// `SlotId` alone only gives an opaque container UUID + slot index; turning
/// that into "Party" or "Box" requires knowing that player's
/// `OtomoCharacterContainerId`/`PalStorageContainerId` (from their
/// `Players/<uid>.sav`), which is a separate file — see [`PlayerProgress`].
/// A Pal assigned to a base camp's own storage, or one whose container isn't
/// any known player's, resolves to `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PalLocationKind {
    Party,
    Box,
    Other,
}

/// A Pal's raw container reference, exactly as stored (`SlotId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PalLocation {
    pub container_id: Uuid,
    pub slot_index: u32,
    pub kind: PalLocationKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pal {
    pub instance_id: Uuid,
    /// Species key with any `BOSS_`/`PREDATOR_` prefix stripped, e.g. `"Kitsun"`.
    pub character_id: String,
    pub owner: Option<Uuid>,
    pub level: u8,
    /// Souls condensing tier, 1..=5.
    pub rank: u8,
    pub souls: SoulUpgrades,
    pub ivs: Ivs,
    pub passives: Vec<String>,
    pub equipped_moves: Vec<String>,
    pub mastered_moves: Vec<String>,
    pub gender: Gender,
    /// `IsRarePal` — a lucky Pal.
    pub is_lucky: bool,
    /// `BOSS_` prefix on `CharacterID` — an alpha/boss spawn.
    pub is_boss: bool,
    /// `PREDATOR_` prefix on `CharacterID`. Not observed in this session's
    /// fixture world; documented in the plan alongside `is_boss`.
    pub is_predator: bool,
    pub nickname: Option<String>,
    /// `None` for a Pal whose `SlotId` field was absent (e.g. mid-transfer).
    pub location: Option<PalLocation>,
}

/// Boss/dungeon defeat tracking, all keyed by boss/dungeon id.
///
/// Verified shapes from a real `Players/<uid>.sav`'s `RecordData`:
/// `NormalBossDefeatFlag`/`TowerBossDefeatFlag`/`SpecificBossDefeatFlag` are
/// `Map<Name, Bool>`; `TowerBossDefeatCount`/`RaidBossDefeatCount` are
/// `Map<Name, Int>`. `PredatorDefeatCount` is a single aggregate `Int`, not
/// per-boss.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BossFlags {
    pub normal_defeated: HashSet<String>,
    pub tower_defeated: HashSet<String>,
    pub specific_defeated: HashSet<String>,
    pub tower_defeat_counts: HashMap<String, u32>,
    pub raid_defeat_counts: HashMap<String, u32>,
    pub predator_defeat_count: u32,
}

/// Verified shapes: `RelicObtainForInstanceFlag`/`NoteObtainForInstanceFlag`
/// are `Map<Name, Bool>`; `RelicPossessNumMap` is `Map<Name, Int>`;
/// `RelicPossessNum`/`FoundTreasureCount` are aggregate `Int`s.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Collectibles {
    pub relics_obtained: HashSet<String>,
    pub relic_possess_counts: HashMap<String, u32>,
    pub relic_possess_total: u32,
    pub notes_obtained: HashSet<String>,
    pub treasures_found: u32,
}

/// `OrderedQuestArray_FullRelease`/`CompletedQuestArray_FullRelease` — both
/// `Array<Struct>` at the `SaveData` level (not `RecordData`); each element
/// carries a quest id. Kept as raw id strings — quest metadata (names,
/// objectives) is Phase 3 reference-data territory.
#[derive(Debug, Clone, Default, Serialize)]
pub struct QuestState {
    pub ordered_quest_ids: Vec<String>,
    pub completed_quest_ids: Vec<String>,
}

/// The `RecordData` counters that don't fit the boss/collectible/quest
/// groupings — verified present as plain `Int`s on a real save.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MiscCounters {
    pub pal_butcher_count: u32,
    pub pal_rankup_count: u32,
    pub mutation_count: u32,
    pub awakening_count: u32,
    pub camp_conquered_count: u32,
    pub oilrig_clear_count: u32,
    pub normal_dungeon_clear_count: u32,
    pub fixed_dungeon_clear_count: u32,
    pub tribe_capture_count: u32,
}

/// A player's meta-progression, from their own `Players/<uid>.sav` —
/// distinct from their in-world `Pal`-shaped character entry in
/// `CharacterSaveParameterMap` (see [`Pal`] and the `character` decoder).
#[derive(Debug, Clone, Serialize)]
pub struct PlayerProgress {
    pub player_uid: Uuid,
    /// `PaldeckUnlockFlag` — authoritative dex completion. Deliberately not
    /// inferred from currently-owned Pals: releasing a Pal must not un-catch it.
    pub paldeck_unlocked: HashSet<String>,
    /// `PalCaptureCount` — per-species capture count, toward the 10-capture bonus.
    pub capture_counts: HashMap<String, u32>,
    /// `PalCaptureBonusCount` — whether the 10-capture bonus was claimed per species.
    pub capture_bonus_claimed: HashSet<String>,
    /// `UnlockedRecipeTechnologyNames`.
    pub unlocked_tech: Vec<String>,
    pub tech_points: u32,
    pub boss_tech_points: u32,
    /// `FastTravelPointUnlockFlag`.
    pub fast_travel_unlocked: HashSet<String>,
    pub bosses: BossFlags,
    pub collectibles: Collectibles,
    pub quests: QuestState,
    pub stats: MiscCounters,
    /// `OtomoCharacterContainerId` — resolves a `Pal.location` to `Party`.
    pub party_container_id: Option<Uuid>,
    /// `PalStorageContainerId` — resolves a `Pal.location` to `Box`.
    pub box_container_id: Option<Uuid>,
}
