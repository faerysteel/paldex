//! The stable interface the rest of the app depends on for species names,
//! passive-skill definitions, breeding results, and icon artwork — deliberately
//! decoupled from *how* that data was obtained, per the plan's Phase 3 design:
//! "the UI depends only on this trait, so swapping the fallback in is a
//! one-line change."
//!
//! ## Status: names and classification are real; numeric stats are not
//!
//! [`crate::ReferenceIndex`] is the real implementation, built from the user's
//! own installed pak. What it does and doesn't cover follows from one fact
//! about how the game ships its data.
//!
//! ### The constraint
//!
//! Palworld's `DataTable` packages are cooked with
//! `PKG_UnversionedProperties` set — confirmed by reading the flag directly
//! off the real `DT_PalMonsterParameter.uasset`
//! (`PackageFlags = 0x80002200`), and corroborated by its name table
//! containing no field names at all (no `HP`, no `ZukanIndex`). Unversioned
//! rows identify their properties positionally against the compiled class
//! schema, so deserializing them needs a `.usmap` mapping file. None ships in
//! the pak (checked: zero `.usmap` entries in all 185,003), none exists for
//! this build, and generating one means running UE4SS against the live game —
//! a manual, per-user, per-patch step. Writing an unversioned-property
//! deserializer without a schema is not tractable; it is comparable in scope
//! to a meaningful chunk of CUE4Parse.
//!
//! ### What is obtainable anyway
//!
//! Two kinds of data escape that constraint, because neither depends on the
//! row schema:
//!
//! - **Name tables** are plain `FString`s in every cooked package. This gives
//!   the full Pal species key set from `DT_PalMonsterParameter` and the human
//!   key set from `DT_PalHumanParameter` — which is what finally lets human
//!   NPCs be filtered out of the Pal roster, the classification Phase 2
//!   deferred to here.
//! - **Localized text** is stored as `FText`, three plain `FString`s
//!   (namespace, key, source), so [`crate::text_table`] reads it directly.
//!   Palworld's `Game.locres` files are empty; all text lives in per-language
//!   `DataTable`s. This yields real display names for species, skills,
//!   technologies, items, and map objects in all 17 shipped languages.
//!
//! Everything comes from the user's own installation at runtime, so no game
//! assets are redistributed and — notably — the plan's pre-agreed fallback of
//! vendoring a third-party dataset was **not needed** for any of it, avoiding
//! that provenance/licensing decision entirely.
//!
//! ### Still missing
//!
//! Numeric and relational data genuinely needs the schema: base stats,
//! elements, work suitabilities, dex numbers ([`Species::dex_number`] is
//! always `None`), rarity, and breeding combos. Icon *textures* are present in
//! the pak at `Pal/Content/Pal/Texture/PalIcon/` (with human NPCs in their own
//! `NPC/` subfolder) and don't need a schema to locate, but do need BC/DXT
//! decoding plus `.ubulk` handling, which isn't implemented yet.
//!
//! [`PassthroughReferenceData`] is retained for tests and for the case where
//! no pak is available (the app must still run without the game installed).

/// A Pal species' static reference data.
#[derive(Debug, Clone, PartialEq)]
pub struct Species {
    pub character_id: String,
    pub display_name: String,
    pub dex_number: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PassiveSkill {
    pub id: String,
    pub display_name: String,
}

pub trait ReferenceData {
    fn species(&self, character_id: &str) -> Option<&Species>;
    fn passive(&self, id: &str) -> Option<&PassiveSkill>;
    fn breeding_result(&self, a: &str, b: &str) -> Option<&str>;
    fn icon(&self, character_id: &str) -> Option<&[u8]>;
}

/// A stub [`ReferenceData`] with no real data behind it — `species`/`passive`
/// echo the raw id back as the display name (so the UI has *something*
/// readable rather than a blank field) and never resolve dex numbers, icons,
/// or breeding results. Exists so the rest of the app can be built and
/// tested against the real trait shape before a real data source lands.
#[derive(Debug, Default)]
pub struct PassthroughReferenceData;

impl ReferenceData for PassthroughReferenceData {
    fn species(&self, _character_id: &str) -> Option<&Species> {
        None
    }

    fn passive(&self, _id: &str) -> Option<&PassiveSkill> {
        None
    }

    fn breeding_result(&self, _a: &str, _b: &str) -> Option<&str> {
        None
    }

    fn icon(&self, _character_id: &str) -> Option<&[u8]> {
        None
    }
}
