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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Gender {
    Male,
    Female,
    Unknown,
}

/// A Pal's raw container reference, exactly as stored (`SlotId`).
///
/// Resolving `container_id` to a friendly Party/Box/Base label requires
/// cross-referencing the player's and base camps' own container ID fields —
/// that's `CharacterContainerSaveData`'s job, a separate decoder not yet
/// implemented. Until then this stays opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PalLocation {
    pub container_id: Uuid,
    pub slot_index: u32,
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
