//! Reference-data types and lookup trait.
//!
//! [`crate::ReferenceIndex`] extracts names, species parameters, breeding data,
//! and icon paths from an installed pak. Schema-bound rows use
//! [`crate::BUNDLED_MAPPINGS`]; see the crate README for sources and failure modes.
//!
//! Element and work-suitability values use internal enum names.
//! [`crate::ReferenceIndex::element_name`] and
//! [`crate::ReferenceIndex::work_suitability_name`] resolve localized labels.
//!
//! [`PassthroughReferenceData`] returns `None` for all lookups. Callers must
//! supply raw-id display fallbacks.

/// A Pal species' base stats, as `DT_PalMonsterParameter` stores them.
///
/// These are the per-species multipliers the game combines with level, IVs and
/// souls — not a finished stat line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BaseStats {
    pub hp: u32,
    pub melee_attack: u32,
    pub shot_attack: u32,
    pub defense: u32,
    pub support: u32,
    pub craft_speed: u32,
}

/// A Pal species' static reference data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Species {
    pub character_id: String,
    pub display_name: String,
    /// Paldeck number. `None` for tower bosses, raid/collab content, and
    /// unused entries, which the game data itself leaves unnumbered — that
    /// absence is what separates a real Paldeck species from the rest.
    pub dex_number: Option<u32>,
    /// Variant marker shown after the number, e.g. the `B` in `005B`. Empty
    /// for a base species.
    pub dex_suffix: String,
    /// Element types, in the game's order, with the `None` slot omitted — so
    /// one entry for a single-element species and two for a dual.
    pub elements: Vec<String>,
    /// Rarity tier as authored, which drives the in-game rarity stars.
    pub rarity: u32,
    pub stats: BaseStats,
    /// Work suitability levels, keyed by the game's own suitability name
    /// (`Handcraft`, `Mining`, …). Only levels above zero are present, so an
    /// absent key means the species cannot do that job at all.
    pub work_suitabilities: std::collections::BTreeMap<String, u32>,
}

impl Species {
    /// The Paldeck label as the game shows it, e.g. `005B`, or `None` for a
    /// species with no Paldeck entry.
    #[must_use]
    pub fn dex_label(&self) -> Option<String> {
        self.dex_number
            .map(|n| format!("{n:03}{}", self.dex_suffix))
    }
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

    /// The pak entry holding this species' icon, without its file extension.
    ///
    /// Pair with [`crate::load_icon_png`] for on-demand decoding. Caching is
    /// the caller's responsibility.
    fn icon_path(&self, character_id: &str) -> Option<&str>;
}

/// Empty [`ReferenceData`] implementation: every lookup returns `None`.
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

    fn icon_path(&self, _character_id: &str) -> Option<&str> {
        None
    }
}
