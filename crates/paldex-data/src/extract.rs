//! Builds the reference index from the user's own installed pak.
//!
//! ## Where each field comes from
//!
//! Palworld's `DataTable` packages set `PKG_UnversionedProperties`, so row
//! values carry no names or types. Three mechanisms cover everything the
//! tracker needs, and all three read the user's own installation — nothing is
//! redistributed and no third-party dataset is vendored:
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
//! 3. **Row values** — Paldeck numbers, base stats, elements, rarity and work
//!    suitabilities — are decoded against the bundled `Mappings.usmap` by
//!    [`crate::datatable`]. This is what retired the vendored dex-number
//!    table that previously stood in for `ZukanIndex`.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::breeding::{BreedingIndex, SpeciesRow};
use crate::reference::{BaseStats, PassiveSkill, Species};
use crate::unversioned::{Properties, Value};
use crate::usmap::Usmap;
use crate::{datatable, text_table, uasset, Pak, PakError};

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
/// Parent pairings that override the generic breeding rule.
const COMBI_UNIQUE: &str = "Pal/Content/Pal/DataTable/Character/DT_PalCombiUnique.uasset";

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

/// The UI string table, which is where the *labels* for the enum ids in
/// [`Species::elements`] and [`Species::work_suitabilities`] live.
///
/// Those fields hold the game's internal enum names — `Leaf`, `Earth`,
/// `Handcraft` — which are not what the game shows the player (`Grass`,
/// `Ground`, `Handiwork`). Rather than hard-code that translation, and lose
/// every language but English with it, both prefixes are read straight out of
/// this table, keyed by the same enum name.
const UI_NAMES: &str = "DT_UI_Common_Text_Common";
const ELEMENT_NAME_PREFIX: &str = "COMMON_ELEMENT_NAME_";
const WORK_SUITABILITY_NAME_PREFIX: &str = "COMMON_WORK_SUITABILITY_";

/// Pal icon textures. The sibling `NPC/` and `SKin/` folders hold human-NPC
/// portraits and cosmetic skins, neither of which is a species icon.
const ICON_DIR: &str = "Pal/Content/Pal/Texture/PalIcon/Normal/";
const ICON_PREFIX: &str = "T_";
const ICON_SUFFIX: &str = "_icon_normal";

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
    /// Element enum name -> localized label, e.g. `Leaf` -> `Grass`.
    element_names: HashMap<String, String>,
    /// Work suitability enum name -> localized label, e.g. `Handcraft` ->
    /// `Handiwork`.
    work_names: HashMap<String, String>,
    /// Work suitability enum names in the order the UI table lists them, which
    /// is the order the in-game Paldeck shows the icons in. `Species` stores
    /// suitabilities in a `BTreeMap`, so without this they would render
    /// alphabetically — correct, but not what the player is used to reading.
    work_order: Vec<String>,
    human_npc_ids: HashSet<String>,
    /// Species key -> pak entry base path (no extension) for its icon.
    icon_paths: HashMap<String, String>,
    language: String,
    /// Breeding rules, keyed on tribe — see [`crate::breeding`].
    breeding: BreedingIndex,
    /// Non-fatal problems hit during extraction, for diagnostics.
    warnings: Vec<String>,
}

/// The enum entry the game uses for "no element", which is stored in the
/// second slot of every single-element species.
const NO_ELEMENT: &str = "None";

/// Property-name prefix for the per-job suitability levels.
const WORK_SUITABILITY_PREFIX: &str = "WorkSuitability_";

/// How well a row represents the species it normalizes onto.
///
/// A row naming the species outright beats one that only reaches it after a
/// `BOSS_`/`PREDATOR_` prefix is stripped, and among equals the one carrying a
/// Paldeck number wins. Without this, the alpha form of a species — stored
/// with `ZukanIndex = -1` — silently erases the base form's number, which cost
/// 36 species their Paldeck entry when rows were applied in file order.
fn row_rank(row_name: &str, normalized_key: &str, row: &Properties) -> u8 {
    let is_exact = row_name.eq_ignore_ascii_case(normalized_key);
    let has_dex = row.get("ZukanIndex").and_then(Value::as_i32).is_some_and(|n| n > 0);
    u8::from(is_exact) * 2 + u8::from(has_dex)
}

/// Copy one `DT_PalMonsterParameter` row onto the species it describes.
///
/// A `ZukanIndex` of zero or below means "not in the Paldeck" — raid bosses
/// and quest-only variants store `-1` — so it maps to `None` rather than to a
/// number, preserving the distinction [`Species::dex_number`] documents.
fn apply_row(species: &mut Species, row: &Properties) {
    let int = |key: &str| row.get(key).and_then(Value::as_i32);

    species.dex_number = int("ZukanIndex").filter(|n| *n > 0).map(|n| n as u32);
    species.dex_suffix =
        row.get("ZukanIndexSuffix").and_then(Value::as_str).unwrap_or_default().to_owned();
    species.rarity = int("Rarity").unwrap_or(0).max(0) as u32;

    species.elements = ["ElementType1", "ElementType2"]
        .iter()
        .filter_map(|key| row.get(*key).and_then(Value::as_str))
        .filter(|e| !e.is_empty() && *e != NO_ELEMENT)
        .map(str::to_owned)
        .collect();

    species.stats = BaseStats {
        hp: int("Hp").unwrap_or(0).max(0) as u32,
        melee_attack: int("MeleeAttack").unwrap_or(0).max(0) as u32,
        shot_attack: int("ShotAttack").unwrap_or(0).max(0) as u32,
        defense: int("Defense").unwrap_or(0).max(0) as u32,
        support: int("Support").unwrap_or(0).max(0) as u32,
        craft_speed: int("CraftSpeed").unwrap_or(0).max(0) as u32,
    };

    species.work_suitabilities = row
        .iter()
        .filter_map(|(key, value)| {
            let job = key.strip_prefix(WORK_SUITABILITY_PREFIX)?;
            let level = value.as_i32()?;
            (level > 0).then(|| (job.to_owned(), level as u32))
        })
        .collect::<BTreeMap<_, _>>();
}

impl ReferenceIndex {
    /// Extract reference data from an opened pak, using `language` for display
    /// names (see [`TEXT_LANGUAGES`]).
    pub fn extract(pak: &mut Pak, language: &str) -> Result<Self, ExtractError> {
        let mut index = Self { language: language.to_owned(), ..Default::default() };

        index.icon_paths = index_icons(pak);
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
                    ..Default::default()
                },
            );
        }

        index.apply_parameters(pak);

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

        index.read_ui_labels(pak, &text_root);

        Ok(index)
    }

    /// Load the element and work-suitability labels from the UI string table.
    ///
    /// A warning rather than an error if the table moves, for the same reason
    /// [`Self::apply_parameters`] is: these are labels on data that is already
    /// correct, so losing them should cost the user pretty names, not the
    /// entire reference index.
    fn read_ui_labels(&mut self, pak: &mut Pak, text_root: &str) {
        let entries = match read_text_table(pak, text_root, UI_NAMES) {
            Ok(entries) => entries,
            Err(e) => {
                self.warnings.push(format!("reading {UI_NAMES}: {e}"));
                return;
            }
        };

        for entry in entries {
            // Some keys under the work prefix are sub-categories with no text
            // of their own (`Mining_Stone`), so an absent display is a skip
            // rather than a fallback to the raw id.
            let Some(text) = entry.display() else { continue };
            if let Some(id) = entry.key.strip_prefix(ELEMENT_NAME_PREFIX) {
                self.element_names.insert(id.to_ascii_lowercase(), text);
            } else if let Some(id) = entry.key.strip_prefix(WORK_SUITABILITY_NAME_PREFIX) {
                if id.is_empty() {
                    continue;
                }
                self.work_order.push(id.to_owned());
                self.work_names.insert(id.to_ascii_lowercase(), text);
            }
        }
    }

    /// The property schema for Palworld's cooked packages.
    ///
    /// Bundled because the game ships none: its `DataTable`s set
    /// `PKG_UnversionedProperties`, so row values carry no names or types and
    /// are matched positionally against this. Regenerate with
    /// `tools/usmap/regen-usmap.sh` after a game update.
    const MAPPINGS: &'static [u8] = crate::BUNDLED_MAPPINGS;

    /// Attach Paldeck numbers, elements, stats, rarity and work suitabilities
    /// from `DT_PalMonsterParameter`.
    ///
    /// Failure here is reported as a warning rather than an error: names,
    /// classification and artwork are all schema-free and still correct
    /// without it, so a game patch that moves this table should degrade the
    /// tracker rather than break it. The `real_parameters` tests assert the
    /// data really is present, so a silent regression still fails the suite.
    fn apply_parameters(&mut self, pak: &mut Pak) {
        let usmap = match Usmap::parse(Self::MAPPINGS) {
            Ok(m) => m,
            Err(e) => {
                self.warnings.push(format!("bundled Mappings.usmap is unusable: {e}"));
                return;
            }
        };

        // Several rows can normalize onto one species: `AmaterasuWolf`, its
        // alpha `BOSS_AmaterasuWolf`, and quest-only duplicates all collapse
        // to the same key. They are not interchangeable — variant rows store
        // `ZukanIndex = -1` and boosted stats — so rows are ranked and only a
        // better one is allowed to overwrite.
        let mut best: HashMap<String, u8> = HashMap::new();

        for entry in MONSTER_PARAMS {
            let base = entry.trim_end_matches(".uasset");
            let (Ok(uasset), Ok(uexp)) =
                (pak.read(entry), pak.read(&format!("{base}.uexp")))
            else {
                continue;
            };
            let table = match datatable::read(&uasset, &uexp, &usmap) {
                Ok(t) => t,
                Err(e) => {
                    self.warnings.push(format!("reading {base}: {e}"));
                    continue;
                }
            };
            for row in &table.rows {
                let key = normalize_key(&row.name);
                let props = &row.properties;

                // Breeding keys on tribe and needs every row, including the
                // variants that lose the ranking below.
                self.breeding.add_species(&SpeciesRow {
                    key: &key,
                    character_id: &row.name,
                    tribe: props.get("Tribe").and_then(Value::as_str).unwrap_or_default(),
                    rank: props.get("CombiRank").and_then(Value::as_i32).unwrap_or(0).max(0) as u32,
                    is_pal: props.get("IsPal").and_then(Value::as_bool).unwrap_or(false),
                    ignore_combi: props.get("IgnoreCombi").and_then(Value::as_bool).unwrap_or(false),
                    zukan: props
                        .get("ZukanIndex")
                        .and_then(Value::as_i32)
                        .unwrap_or(0)
                        .max(0) as u32,
                });

                let Some(species) = self.species.get_mut(&key) else {
                    continue;
                };
                let rank = row_rank(&row.name, &key, props);
                // `>=` rather than `>` so a later row of equal rank still
                // wins, matching the engine's own last-insert-wins `RowMap`.
                if rank >= *best.get(&key).unwrap_or(&0) {
                    best.insert(key, rank);
                    apply_row(species, props);
                }
            }
        }

        self.read_unique_combos(pak, &usmap);
        self.breeding.finish();
    }

    /// Load the unique parent-pair overrides from `DT_PalCombiUnique`.
    ///
    /// Reported as a warning rather than an error: without it the generic
    /// `CombiRank` rule still answers every pair, just without the 258
    /// hand-authored exceptions.
    fn read_unique_combos(&mut self, pak: &mut Pak, usmap: &Usmap) {
        let base = COMBI_UNIQUE.trim_end_matches(".uasset");
        let (Ok(uasset), Ok(uexp)) = (pak.read(COMBI_UNIQUE), pak.read(&format!("{base}.uexp")))
        else {
            self.warnings.push(format!("{base} is missing from the pak"));
            return;
        };
        let table = match datatable::read(&uasset, &uexp, usmap) {
            Ok(t) => t,
            Err(e) => {
                self.warnings.push(format!("reading {base}: {e}"));
                return;
            }
        };
        for row in &table.rows {
            let get = |k: &str| row.properties.get(k).and_then(Value::as_str).unwrap_or_default();
            self.breeding.add_unique(
                get("ParentTribeA"),
                get("ParentTribeB"),
                get("ChildCharacterID"),
            );
        }
    }

    /// How many species carry a Paldeck number — the real Paldeck
    /// denominator, unlike [`Self::species_count`].
    #[must_use]
    pub fn dex_entry_count(&self) -> usize {
        self.species.values().filter(|s| s.dex_number.is_some()).count()
    }

    /// Non-fatal problems hit while extracting, e.g. a parameter table that
    /// moved in a game update. Empty on a healthy extraction.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    #[must_use]
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Number of named species — one per row of `DT_PalNameText_Common`.
    ///
    /// An upper bound on the Paldeck rather than the Paldeck itself: it counts
    /// quest-only and cut entries too. Use [`Self::dex_entry_count`] for the
    /// real denominator, which the game's own `ZukanIndex` now supplies.
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

    /// Resolve an element enum name from [`Species::elements`] to the label
    /// the game shows, e.g. `Leaf` -> `Grass`.
    #[must_use]
    pub fn element_name(&self, id: &str) -> Option<&str> {
        self.element_names.get(&id.to_ascii_lowercase()).map(String::as_str)
    }

    /// Resolve a work suitability enum name from
    /// [`Species::work_suitabilities`] to the label the game shows, e.g.
    /// `Handcraft` -> `Handiwork`.
    #[must_use]
    pub fn work_suitability_name(&self, id: &str) -> Option<&str> {
        self.work_names.get(&id.to_ascii_lowercase()).map(String::as_str)
    }

    /// Work suitability enum names in the game's own display order. Callers
    /// ordering a species' suitabilities should follow this rather than the
    /// `BTreeMap`'s alphabetical order.
    #[must_use]
    pub fn work_suitability_order(&self) -> &[String] {
        &self.work_order
    }

    #[must_use]
    pub fn icon_count(&self) -> usize {
        self.icon_paths.len()
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
pub(crate) fn normalize_key(character_id: &str) -> String {
    let lower = character_id.to_ascii_lowercase();
    for prefix in VARIANT_PREFIXES {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    lower
}

/// Map species keys to their icon texture's pak entry.
///
/// Entries are named `T_<CharacterID>_icon_normal`; matching is
/// case-insensitive because the pak is inconsistent about it elsewhere.
fn index_icons(pak: &Pak) -> HashMap<String, String> {
    let mut icons = HashMap::new();
    for path in pak.files() {
        let Some(stem) = path.strip_prefix(ICON_DIR).and_then(|p| p.strip_suffix(".uasset")) else {
            continue;
        };
        let Some(rest) = stem.strip_prefix(ICON_PREFIX) else {
            continue;
        };
        if rest.len() <= ICON_SUFFIX.len() {
            continue;
        }
        let (id, suffix) = rest.split_at(rest.len() - ICON_SUFFIX.len());
        if !suffix.eq_ignore_ascii_case(ICON_SUFFIX) {
            continue;
        }
        icons.insert(normalize_key(id), format!("{ICON_DIR}{stem}"));
    }
    icons
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

    /// Resolved from `DT_PalCombiUnique` first, then the generic `CombiRank`
    /// rule — see [`crate::breeding`].
    fn breeding_result(&self, a: &str, b: &str) -> Option<&str> {
        self.breeding.child_of(&normalize_key(a), &normalize_key(b))
    }

    fn icon_path(&self, character_id: &str) -> Option<&str> {
        self.icon_paths.get(&normalize_key(character_id)).map(String::as_str)
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
            Species {
                character_id: "Anubis".into(),
                display_name: "Anubis".into(),
                ..Default::default()
            },
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
