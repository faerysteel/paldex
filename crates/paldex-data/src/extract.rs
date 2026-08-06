//! Builds the reference index from the user's own installed pak.
//!
//! ## What is and isn't obtainable
//!
//! Palworld's `DataTable` packages set `PKG_UnversionedProperties`, so numeric
//! row *values* (stats, elements, work suitabilities, dex numbers, breeding
//! combos) need a `.usmap` schema that doesn't exist for this build — see
//! [`crate::reference`] for that writeup, which still stands.
//!
//! Two things survive that restriction, and between them they cover most of
//! what the tracker actually displays:
//!
//! 1. **Name tables** are plain `FString`s in every cooked package, so
//!    `DT_PalMonsterParameter` and `DT_PalHumanParameter` yield the full set of
//!    Pal species keys and human-NPC keys respectively. That is exactly what
//!    is needed to keep human NPCs out of the Pal roster — the classification
//!    problem Phase 2 explicitly deferred to Phase 3.
//! 2. **Localized text** is stored as `FText`, which serializes as plain
//!    `FString`s regardless of the surrounding schema (see
//!    [`crate::text_table`]), giving real display names for species, skills,
//!    technologies, items, and map objects in every shipped language.
//!
//! All of it comes from the user's own installation at runtime, so nothing is
//! redistributed and no third-party dataset is vendored.

use std::collections::{HashMap, HashSet};

use crate::reference::{PassiveSkill, Species};
use crate::{text_table, uasset, Pak, PakError};

/// Languages shipped in the pak.
///
/// Japanese is the game's *source* language: its tables live in the base
/// content path rather than under `L10N/`, which is why it needs the special
/// case in [`text_root`].
pub const TEXT_LANGUAGES: &[&str] = &[
    "en", "de", "es", "es-MX", "fr", "id", "it", "ja", "ko", "pl", "pt-BR", "ru", "th", "tr", "vi",
    "zh-Hans", "zh-Hant",
];

/// The game's source language, stored outside the `L10N/` tree.
const SOURCE_LANGUAGE: &str = "ja";

/// Character parameter tables. Each ships as a base table plus a `_Common`
/// companion that later content was added to; both must be read or newer
/// species (and NPCs) go missing.
const MONSTER_PARAMS: &[&str] = &[
    "Pal/Content/Pal/DataTable/Character/DT_PalMonsterParameter.uasset",
    "Pal/Content/Pal/DataTable/Character/DT_PalMonsterParameter_Common.uasset",
];
const HUMAN_PARAMS: &[&str] = &[
    "Pal/Content/Pal/DataTable/Character/DT_PalHumanParameter.uasset",
    "Pal/Content/Pal/DataTable/Character/DT_PalHumanParameter_Common.uasset",
];

/// Where a language's text `DataTable`s live inside the pak.
fn text_root(language: &str) -> String {
    if language == SOURCE_LANGUAGE {
        "Pal/Content/Pal/DataTable/Text".to_owned()
    } else {
        format!("Pal/Content/L10N/{language}/Pal/DataTable/Text")
    }
}

/// Text tables read for display names, as (table name, key prefix).
const PAL_NAMES: (&str, &str) = ("DT_PalNameText_Common", "PAL_NAME_");
const SKILL_NAMES: (&str, &str) = ("DT_SkillNameText_Common", "");
const TECH_NAMES: (&str, &str) = ("DT_TechnologyNameText_Common", "NAME_RECIPE_");
const ITEM_NAMES: (&str, &str) = ("DT_ItemNameText_Common", "ITEM_NAME_");
const MAP_OBJECT_NAMES: (&str, &str) = ("DT_MapObjectNameText_Common", "MAPOBJECT_NAME_");

/// Saves store passive ids bare; the skill text table prefixes them.
const PASSIVE_KEY_PREFIX: &str = "PASSIVE_";

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error(transparent)]
    Pak(#[from] PakError),
    #[error("parsing {entry}: {source}")]
    Uasset {
        entry: String,
        #[source]
        source: uasset::UassetError,
    },
    #[error("no text tables found for language {0:?} — is this a Palworld pak?")]
    NoTextTables(String),
}

/// Reference data extracted from a pak.
#[derive(Debug, Clone, Default)]
pub struct ReferenceIndex {
    /// Named species, keyed by base `CharacterID` — one entry per row of
    /// `DT_PalNameText_Common`. This is the dex-eligible set.
    species: HashMap<String, Species>,
    /// Every `FName` referenced by `DT_PalMonsterParameter`. A *superset* of
    /// the species keys (it also contains passive-skill ids, quest-only
    /// variants, and `BOSS_` forms), so it is only sound for membership tests
    /// — never as a species listing.
    pal_keys: HashSet<String>,
    passives: HashMap<String, PassiveSkill>,
    technologies: HashMap<String, String>,
    items: HashMap<String, String>,
    map_objects: HashMap<String, String>,
    human_npc_ids: HashSet<String>,
    language: String,
}

impl ReferenceIndex {
    /// Extract reference data from an opened pak, using `language` for display
    /// names (see [`TEXT_LANGUAGES`]).
    pub fn extract(pak: &mut Pak, language: &str) -> Result<Self, ExtractError> {
        let mut index = Self { language: language.to_owned(), ..Default::default() };

        index.pal_keys = row_name_candidates(pak, MONSTER_PARAMS)?;
        index.human_npc_ids = row_name_candidates(pak, HUMAN_PARAMS)?;

        let text_root = text_root(language);
        let pal_text = read_text_table(pak, &text_root, PAL_NAMES.0)?;
        if pal_text.is_empty() {
            return Err(ExtractError::NoTextTables(language.to_owned()));
        }

        for entry in &pal_text {
            let Some(character_id) = entry.key.strip_prefix(PAL_NAMES.1) else {
                continue;
            };
            let display = entry.display();
            index.species.insert(
                normalize_key(character_id),
                Species {
                    // Preserve the pak's own casing for display; only the
                    // lookup key is normalized.
                    character_id: character_id.to_owned(),
                    display_name: display.unwrap_or_else(|| character_id.to_owned()),
                    dex_number: None,
                },
            );
        }

        for entry in read_text_table(pak, &text_root, SKILL_NAMES.0)? {
            let display_name = entry.display().unwrap_or_else(|| entry.key.clone());
            index
                .passives
                .insert(entry.key.clone(), PassiveSkill { id: entry.key, display_name });
        }

        // Items and map objects must be read before technologies: most
        // technology names are a bare indirection into one of those tables
        // rather than literal text.
        for entry in read_text_table(pak, &text_root, ITEM_NAMES.0)? {
            if let Some(id) = entry.key.strip_prefix(ITEM_NAMES.1) {
                if let Some(text) = entry.display() {
                    index.items.insert(id.to_ascii_lowercase(), text);
                }
            }
        }
        for entry in read_text_table(pak, &text_root, MAP_OBJECT_NAMES.0)? {
            if let Some(id) = entry.key.strip_prefix(MAP_OBJECT_NAMES.1) {
                if let Some(text) = entry.display() {
                    index.map_objects.insert(id.to_ascii_lowercase(), text);
                }
            }
        }
        for entry in read_text_table(pak, &text_root, TECH_NAMES.0)? {
            let Some(id) = entry.key.strip_prefix(TECH_NAMES.1) else {
                continue;
            };
            let resolved = match text_table::text_reference(&entry.source) {
                Some(text_table::TextRef::Item(item_id)) => index.item(item_id).map(str::to_owned),
                Some(text_table::TextRef::MapObject(obj_id)) => {
                    index.map_object(obj_id).map(str::to_owned)
                }
                None => entry.display(),
            };
            if let Some(text) = resolved {
                index.technologies.insert(id.to_ascii_lowercase(), text);
            }
        }

        Ok(index)
    }

    #[must_use]
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Number of named species — one per row of `DT_PalNameText_Common`.
    ///
    /// This is *close to* but not exactly the in-game Paldeck denominator:
    /// without dex numbers (which need a `.usmap`) there is no way to tell a
    /// Paldeck-listed species from a quest-only or cut one. Treat it as an
    /// upper bound, and prefer the save's own `PaldeckUnlockFlag` key set when
    /// an authoritative denominator is required.
    #[must_use]
    pub fn species_count(&self) -> usize {
        self.species.len()
    }

    /// Every named species, unordered.
    pub fn species_iter(&self) -> impl Iterator<Item = &Species> {
        self.species.values()
    }

    #[must_use]
    pub fn passive_count(&self) -> usize {
        self.passives.len()
    }

    #[must_use]
    pub fn technology_count(&self) -> usize {
        self.technologies.len()
    }

    #[must_use]
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Resolve a technology id (as stored in `UnlockedRecipeTechnologyNames`)
    /// to its display name. Matched case-insensitively: saves store e.g.
    /// `AIcore` while the text table keys it `NAME_RECIPE_AICORE`.
    #[must_use]
    pub fn technology(&self, id: &str) -> Option<&str> {
        self.technologies.get(&id.to_ascii_lowercase()).map(String::as_str)
    }

    /// Resolve an item id to its display name, case-insensitively.
    #[must_use]
    pub fn item(&self, id: &str) -> Option<&str> {
        self.items.get(&id.to_ascii_lowercase()).map(String::as_str)
    }

    /// Resolve a buildable structure id to its display name,
    /// case-insensitively.
    #[must_use]
    pub fn map_object(&self, id: &str) -> Option<&str> {
        self.map_objects.get(&id.to_ascii_lowercase()).map(String::as_str)
    }

    #[must_use]
    pub fn map_object_count(&self) -> usize {
        self.map_objects.len()
    }

    /// Whether `character_id` is a human NPC rather than a Pal.
    ///
    /// Phase 2 classifies characters as Player-vs-Pal only, because the save
    /// alone can't tell a human NPC from a Pal (both carry full IV stats).
    /// The pak can: humans are rows of `DT_PalHumanParameter`.
    #[must_use]
    pub fn is_human_npc(&self, character_id: &str) -> bool {
        let key = normalize_key(character_id);
        self.human_npc_ids.contains(&key) && !self.is_pal_key(&key)
    }

    /// Whether `character_id` is a Pal — either a named species or a key the
    /// monster parameter table references (quest/variant forms that have no
    /// text row of their own).
    #[must_use]
    pub fn is_pal(&self, character_id: &str) -> bool {
        self.is_pal_key(&normalize_key(character_id))
    }

    fn is_pal_key(&self, key: &str) -> bool {
        self.species.contains_key(key) || self.pal_keys.contains(key)
    }

    /// Whether `character_id` resolves to anything the pak knows about.
    #[must_use]
    pub fn is_known(&self, character_id: &str) -> bool {
        let key = normalize_key(character_id);
        self.is_pal_key(&key) || self.human_npc_ids.contains(&key)
    }
}

/// Variant prefixes that saves put on `CharacterID`, mirroring
/// `paldex-model`'s normalization so ids from either side resolve identically.
const VARIANT_PREFIXES: &[&str] = &["boss_", "predator_"];

/// Normalize a `CharacterID` into the lookup key used by every map here:
/// variant prefix stripped, then lowercased.
///
/// Both steps are load-bearing against the real data. Unreal `FName`s are
/// case-insensitive and the game is inconsistent in practice — the save
/// contains `Sheepball` where the text table has `SheepBall`, and `Boss_`
/// alongside `BOSS_`. Comparing case-sensitively silently loses those.
fn normalize_key(character_id: &str) -> String {
    let lower = character_id.to_ascii_lowercase();
    for prefix in VARIANT_PREFIXES {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    lower
}

/// Read the name tables of a set of cooked packages, which for a `DataTable`
/// contain its row names (plus other referenced `FName`s — a superset, which
/// is fine for membership tests).
///
/// A missing entry is skipped rather than fatal: the `_Common` companions are
/// content-dependent and a future patch may drop or rename one.
fn row_name_candidates(pak: &mut Pak, entries: &[&str]) -> Result<HashSet<String>, ExtractError> {
    let mut names = HashSet::new();
    for entry in entries {
        let Ok(bytes) = pak.read(entry) else {
            continue;
        };
        let summary = uasset::parse_summary(&bytes)
            .map_err(|source| ExtractError::Uasset { entry: (*entry).to_owned(), source })?;
        names.extend(summary.names.iter().map(|n| normalize_key(n)));
    }
    Ok(names)
}

fn read_text_table(
    pak: &mut Pak,
    root: &str,
    table: &str,
) -> Result<Vec<text_table::TextEntry>, ExtractError> {
    let entry = format!("{root}/{table}.uexp");
    let bytes = pak.read(&entry)?;
    text_table::parse(&bytes, table)
        .map_err(|source| ExtractError::Uasset { entry, source })
}

impl crate::reference::ReferenceData for ReferenceIndex {
    fn species(&self, character_id: &str) -> Option<&Species> {
        self.species.get(&normalize_key(character_id))
    }

    /// Accepts either the text-table key (`PASSIVE_CraftSpeed_up2`) or the
    /// bare id as it appears in a save's `PassiveSkillList`
    /// (`CraftSpeed_up2`), since callers naturally have the latter.
    fn passive(&self, id: &str) -> Option<&PassiveSkill> {
        self.passives
            .get(id)
            .or_else(|| self.passives.get(&format!("{PASSIVE_KEY_PREFIX}{id}")))
    }

    /// Breeding combos live in an unversioned `DataTable` and remain
    /// unavailable without a `.usmap`.
    fn breeding_result(&self, _a: &str, _b: &str) -> Option<&str> {
        None
    }

    /// Icon textures are present in the pak but not yet decoded — see
    /// [`crate::reference`].
    fn icon(&self, _character_id: &str) -> Option<&[u8]> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_variant_prefixes() {
        assert_eq!(normalize_key("BOSS_AmaterasuWolf"), "amaterasuwolf");
        assert_eq!(normalize_key("PREDATOR_Anubis"), "anubis");
        assert_eq!(normalize_key("Anubis"), "anubis");
    }

    /// The real save and the real pak disagree on casing (`Sheepball` vs
    /// `SheepBall`, `Boss_` vs `BOSS_`); `FName`s are case-insensitive, so
    /// lookups must be too.
    #[test]
    fn normalization_is_case_insensitive() {
        assert_eq!(normalize_key("Sheepball"), normalize_key("SheepBall"));
        assert_eq!(normalize_key("Boss_LazyCatFish"), normalize_key("BOSS_LazyCatfish"));
        assert_eq!(normalize_key("Boss_Anubis"), "anubis");
    }

    /// A key present in both parameter tables must classify as a Pal, not an
    /// NPC — the species table is the more specific signal.
    #[test]
    fn species_membership_wins_over_npc_membership() {
        let mut index = ReferenceIndex::default();
        index.species.insert(
            "anubis".into(),
            Species { character_id: "Anubis".into(), display_name: "Anubis".into(), dex_number: None },
        );
        index.human_npc_ids.insert("anubis".into());
        index.human_npc_ids.insert("hunter_rifle".into());

        assert!(!index.is_human_npc("Anubis"));
        assert!(!index.is_human_npc("BOSS_Anubis"));
        assert!(index.is_human_npc("Hunter_Rifle"));
    }

    /// Saves carry bare passive ids while the text table keys them with a
    /// `PASSIVE_` prefix; both spellings must resolve.
    #[test]
    fn passives_resolve_with_or_without_the_table_prefix() {
        use crate::reference::ReferenceData;

        let mut index = ReferenceIndex::default();
        index.passives.insert(
            "PASSIVE_CraftSpeed_up2".into(),
            PassiveSkill { id: "PASSIVE_CraftSpeed_up2".into(), display_name: "Artisan".into() },
        );

        assert_eq!(index.passive("CraftSpeed_up2").map(|p| p.display_name.as_str()), Some("Artisan"));
        assert_eq!(
            index.passive("PASSIVE_CraftSpeed_up2").map(|p| p.display_name.as_str()),
            Some("Artisan")
        );
        assert!(index.passive("NotARealPassive").is_none());
    }

    /// A quest-only variant has no text row but is still a Pal, and must not
    /// be mistaken for an NPC.
    #[test]
    fn unnamed_pal_keys_still_classify_as_pals() {
        let mut index = ReferenceIndex::default();
        index.pal_keys.insert(normalize_key("AmaterasuWolf_Dark_Quest_Enemy"));

        assert!(index.is_pal("AmaterasuWolf_Dark_Quest_Enemy"));
        assert!(index.is_known("AmaterasuWolf_Dark_Quest_Enemy"));
        assert!(!index.is_human_npc("AmaterasuWolf_Dark_Quest_Enemy"));
        // …but it contributes no species entry, so it can't inflate the dex.
        assert_eq!(index.species_count(), 0);
    }
}
