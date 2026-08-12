//! Read-only reporting queries against the store's latest snapshot for a
//! world — the data the roster table, dex grid, and player panel render.

use paldex_store::Store;
use serde::Serialize;

/// Whose progress a query reports.
///
/// An explicit type rather than an `Option<&str>` because the two cases are
/// not "filter or don't": most of what the game tracks — capture counts,
/// capture bonuses, tech, quests — is *per player*, and silently aggregating
/// it across a shared world invents a player who has everyone's progress at
/// once. That is exactly the bug this type exists to make hard to write: the
/// dex screen used to report `MAX(capture_count)` across players, which
/// claimed too many species were at their capture bonus in a world where the two
/// players were individually at different totals.
///
/// [`All`](PlayerScope::All) is still right for genuinely world-level
/// questions — "has anyone here caught this species" — so it stays available,
/// but naming it forces the choice at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerScope<'a> {
    /// Every player in the world, aggregated.
    All,
    /// One player, by `player_uid`.
    Only(&'a str),
}

impl<'a> PlayerScope<'a> {
    /// The uid to bind, when this scope narrows to one player.
    #[must_use]
    pub fn uid(self) -> Option<&'a str> {
        match self {
            Self::All => None,
            Self::Only(uid) => Some(uid),
        }
    }

    /// The extra `AND player_uid = ?n` clause this scope needs, bound to the
    /// caller's next free parameter index.
    fn clause(self, index: usize) -> String {
        match self {
            Self::All => String::new(),
            Self::Only(_) => format!(" AND player_uid = ?{index}"),
        }
    }
}

/// One of a species' elements, carrying both the internal enum name and the
/// label the game shows. The id travels alongside the name because the UI
/// colours a chip by element and must not key that off localized text.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementView {
    pub id: String,
    pub name: String,
}

/// A job a species can do, and how well. Ordered by the game's own display
/// order, not alphabetically — see `ReferenceIndex::work_suitability_order`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkSuitabilityView {
    pub id: String,
    pub name: String,
    pub level: u32,
}

/// A species' authored base stats. These are the per-species inputs the game
/// combines with level, IVs and souls — not a finished stat line, so the UI
/// presents them as relative rather than as numbers a player would see in a
/// Pal's status screen.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseStatsView {
    pub hp: u32,
    pub melee_attack: u32,
    pub shot_attack: u32,
    pub defense: u32,
    pub support: u32,
    pub craft_speed: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PalView {
    pub instance_id: String,
    pub character_id: String,
    /// Localized species name from the game pak, e.g. `Kitsun` for
    /// `AmaterasuWolf`. `None` when the pak isn't available, in which case the
    /// UI falls back to `character_id`.
    pub display_name: Option<String>,
    /// This pal's species' elements. Empty when no pak is available, and also
    /// for the handful of species the shipped data leaves elementless.
    pub elements: Vec<ElementView>,
    /// Species rarity tier, which drives the in-game rarity stars. 0 without a
    /// pak.
    pub rarity: u32,
    pub owner: Option<String>,
    pub level: i64,
    pub rank: i64,
    pub soul_hp: i64,
    pub soul_attack: i64,
    pub soul_defense: i64,
    pub soul_craft_speed: i64,
    pub iv_hp: i64,
    pub iv_shot: i64,
    pub iv_defense: i64,
    pub gender: String,
    pub is_lucky: bool,
    pub is_boss: bool,
    pub is_predator: bool,
    pub nickname: Option<String>,
    pub location_kind: Option<String>,
    pub passives: Vec<String>,
    /// Localized passive names, parallel to `passives`, falling back to the
    /// raw id when unresolved. Empty when no game pak is available.
    pub passive_names: Vec<String>,
    pub equipped_moves: Vec<String>,
    pub mastered_moves: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DexProgressView {
    /// Species unlocked across every player in this world (a species any
    /// player has caught counts once), straight from `PaldeckUnlockFlag`.
    pub unlocked_species_count: i64,
    pub unlocked_species: Vec<String>,
    /// Total Paldeck entries — species that carry a Paldeck number. 0 without
    /// a pak.
    ///
    /// This is the real denominator, not the count of named species: tower
    /// bosses, raid/collab content, and unused entries have no Paldeck number
    /// and are excluded, exactly as the game excludes them.
    pub total_species_count: i64,
    /// One row per Paldeck entry when a pak is available; otherwise one row
    /// per unlocked species, so the grid still renders without the game
    /// installed — just without the unseen ones.
    pub entries: Vec<DexEntryView>,
}

/// A single species' dex state. Built in the command layer, which is where
/// store facts meet pak reference data.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DexEntryView {
    pub character_id: String,
    pub display_name: String,
    /// Paldeck number as the game shows it, e.g. `005B`. `None` only for a
    /// caught species with no Paldeck entry, which the grid still lists rather
    /// than dropping.
    pub dex_label: Option<String>,
    /// From `PaldeckUnlockFlag`, so releasing or butchering a Pal never
    /// un-catches it — `dex_events` is append-only.
    pub caught: bool,
    /// `PalCaptureCount` for the selected player, or the best across players
    /// under [`PlayerScope::All`].
    pub capture_count: i64,
    /// This species' capture-bonus tier, 0..=[`CAPTURE_BONUS_AT`], where 0 is
    /// "never caught" and `CAPTURE_BONUS_AT` is "bonus complete".
    ///
    /// [`CAPTURE_BONUS_AT`]: paldex_model::CAPTURE_BONUS_AT
    pub bonus_tier: i64,
    /// Elements, rarity, base stats and work suitabilities as
    /// `DT_PalMonsterParameter` authored them. All empty or `None` without a
    /// pak, which is the same fallback the rest of this view already uses.
    pub elements: Vec<ElementView>,
    pub rarity: u32,
    pub stats: Option<BaseStatsView>,
    pub work_suitabilities: Vec<WorkSuitabilityView>,
}

/// Dex facts as the store has them, keyed by the save's own species ids —
/// before reference data maps them onto canonical species and names them.
#[derive(Debug, Default)]
pub struct DexFacts {
    pub unlocked: Vec<String>,
    /// Save species id -> capture count, for whichever [`PlayerScope`] was
    /// asked for.
    pub capture_counts: std::collections::HashMap<String, i64>,
    /// Save species id -> capture-bonus tier, same scope.
    pub bonus_tiers: std::collections::HashMap<String, i64>,
}

/// One player, as the player selector lists them.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPlayerView {
    pub player_uid: String,
    /// `None` for a save whose world data didn't name this player; the UI
    /// falls back to a shortened uid rather than showing nothing.
    pub name: Option<String>,
    /// `None` on a snapshot ingested before names were recorded.
    pub level: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerProgressView {
    pub player_uid: String,
    pub tech_points: i64,
    pub boss_tech_points: i64,
    pub pal_butcher_count: i64,
    pub mutation_count: i64,
    pub awakening_count: i64,
    pub camp_conquered_count: i64,
    pub normal_dungeon_clear_count: i64,
    pub fixed_dungeon_clear_count: i64,
    pub relic_possess_total: i64,
    pub treasures_found: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseCampView {
    pub id: String,
    pub guild_id: Option<String>,
}

/// One Pal as the analysis screen shows it: enough to identify and judge it,
/// and nothing else.
///
/// Deliberately not a [`PalView`]. The analysis screen lists the same Pals
/// several times over (graded, best-of-species, condense fodder), and shipping
/// the full roster shape for each would be several copies of a payload the
/// roster tab already fetches.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GradedPalView {
    pub instance_id: String,
    pub character_id: String,
    /// Localized species name, or `None` without a pak — same fallback as
    /// [`PalView::display_name`].
    pub display_name: Option<String>,
    pub nickname: Option<String>,
    pub level: i64,
    pub rank: i64,
    pub gender: String,
    pub iv_hp: i64,
    pub iv_shot: i64,
    pub iv_defense: i64,
    /// Mean of the three talents, as `analysis::grade_ivs` computes it.
    pub composite: f32,
    /// The tier that composite falls in — `D`..`S`, or `Perfect`.
    pub tier: String,
    /// Localized passive names, falling back to raw ids.
    pub passive_names: Vec<String>,
}

/// The quality screen's four lists. The three derived lists carry instance ids
/// into `graded` rather than repeating the rows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PalQualityView {
    /// Every owned Pal, graded, best composite first.
    pub graded: Vec<GradedPalView>,
    /// The best specimen of each species the player owns.
    pub best_of_species: Vec<String>,
    /// Duplicates worth condensing — never the best-of-species specimen.
    pub condense_candidates: Vec<String>,
    /// Owned Pals ranked by passive count, best first.
    pub passive_ranking: Vec<String>,
}

/// One parent in a suggested pairing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BreedingParentView {
    pub instance_id: String,
    pub character_id: String,
    pub display_name: Option<String>,
    pub nickname: Option<String>,
    pub level: i64,
    pub gender: String,
    pub iv_hp: i64,
    pub iv_shot: i64,
    pub iv_defense: i64,
}

/// A species the breeding picker may offer as a target.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BreedingTargetView {
    pub character_id: String,
    pub display_name: String,
    /// Paldeck number as the game shows it, or `None` for a producible species
    /// with no Paldeck entry.
    pub dex_label: Option<String>,
}

/// One side of a pairing that involves a species the player does not own.
///
/// `owned` is the best specimen when this species is in the roster, and `None`
/// when it is the side that would have to be obtained — the distinction the
/// whole "unowned parents" list exists to draw.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingSideView {
    pub character_id: String,
    pub display_name: Option<String>,
    /// Paldeck number as the game shows it, so an unowned species is still
    /// identifiable by the number the player would look up.
    pub dex_label: Option<String>,
    pub owned: Option<BreedingParentView>,
}

/// A pairing needing a Pal the player doesn't have — either a species missing
/// from the roster, or a second specimen of one already in it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnownedPairingView {
    pub parent_a: PairingSideView,
    pub parent_b: PairingSideView,
    /// `secondSpecimen`, `oppositeGender`, `oneSpecies` or `twoSpecies` —
    /// what the combination is waiting on, smallest ask first.
    pub need: String,
    /// The gender every owned candidate shares, for `oppositeGender`; `None`
    /// for every other need.
    pub blocking_gender: Option<String>,
    /// Species missing from the roster entirely — empty when both sides are
    /// owned and only another Pal of one of them is needed.
    pub missing_species: Vec<String>,
}

/// A suggested pairing, with the numbers behind its ranking — the plan asks
/// for recommendations whose inputs are visible, not a bare ordering.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BreedingPairView {
    pub parent_a: BreedingParentView,
    pub parent_b: BreedingParentView,
    /// Mean of the two parents' composite IV scores.
    pub parent_iv_average: f32,
    /// Localized names of every distinct passive across both parents — the
    /// pool the child draws from.
    pub inherited_passives: Vec<String>,
    pub score: f32,
}

/// `Clone` because it doubles as the [`crate::commands::SNAPSHOT_EVENT`]
/// payload, and Tauri's `emit` requires an owned, cloneable value.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotSummaryView {
    pub snapshot_id: i64,
    pub pal_count: i64,
    pub player_count: i64,
    pub taken_at: i64,
}

pub fn snapshot_summary(store: &Store, snapshot_id: i64) -> Result<SnapshotSummaryView, String> {
    let conn = store.conn();
    let pal_count = conn
        .query_row("SELECT COUNT(*) FROM pals WHERE snapshot_id = ?1", [snapshot_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let player_count = conn
        .query_row("SELECT COUNT(*) FROM players WHERE snapshot_id = ?1", [snapshot_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let taken_at = conn
        .query_row("SELECT taken_at FROM snapshots WHERE id = ?1", [snapshot_id], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    Ok(SnapshotSummaryView { snapshot_id, pal_count, player_count, taken_at })
}

/// Whether Pals with no owner — base-camp workers, 71 of them in the save
/// this was built against — count as part of the roster being asked for.
///
/// Orthogonal to [`PlayerScope`], not a special case of it. A base Pal
/// belongs to the guild rather than to any player, so "who owns it" and
/// "should it be here" are genuinely two questions: the breeding screen wants
/// them under *every* scope, because a Pal sitting in a base is still a Pal
/// you can put in a breeding farm, while the roster and analysis screens are
/// answering "how are *my* Pals doing" and reasonably leave them out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasePals {
    Include,
    Exclude,
}

/// The snapshot's Pals, filtered by who owns them.
pub fn pal_roster(
    store: &Store,
    snapshot_id: i64,
    scope: PlayerScope,
    base_pals: BasePals,
) -> Result<Vec<PalView>, String> {
    let conn = store.conn();
    let owner_clause = match (scope, base_pals) {
        (PlayerScope::All, BasePals::Include) => "",
        (PlayerScope::All, BasePals::Exclude) => " AND owner IS NOT NULL",
        (PlayerScope::Only(_), BasePals::Include) => " AND (owner = ?2 OR owner IS NULL)",
        (PlayerScope::Only(_), BasePals::Exclude) => " AND owner = ?2",
    };
    let sql = format!(
        "SELECT instance_id, character_id, owner, level, rank,
                soul_hp, soul_attack, soul_defense, soul_craft_speed,
                iv_hp, iv_shot, iv_defense, gender, is_lucky, is_boss,
                is_predator, nickname, location_kind
         FROM pals WHERE snapshot_id = ?1{owner_clause} ORDER BY character_id, level DESC"
    );
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

    let bind: Vec<Box<dyn rusqlite::ToSql>> = match scope.uid() {
        None => vec![Box::new(snapshot_id)],
        Some(uid) => vec![Box::new(snapshot_id), Box::new(uid.to_owned())],
    };
    let rows = stmt
        .query_map(rusqlite::params_from_iter(bind.iter()), |row| {
            Ok(PalView {
                instance_id: row.get(0)?,
                character_id: row.get(1)?,
                // Filled in by the command layer, which owns the pak-derived
                // reference data; this layer stays pure SQL.
                display_name: None,
                elements: Vec::new(),
                rarity: 0,
                owner: row.get(2)?,
                level: row.get(3)?,
                rank: row.get(4)?,
                soul_hp: row.get(5)?,
                soul_attack: row.get(6)?,
                soul_defense: row.get(7)?,
                soul_craft_speed: row.get(8)?,
                iv_hp: row.get(9)?,
                iv_shot: row.get(10)?,
                iv_defense: row.get(11)?,
                gender: row.get(12)?,
                is_lucky: row.get(13)?,
                is_boss: row.get(14)?,
                is_predator: row.get(15)?,
                nickname: row.get(16)?,
                location_kind: row.get(17)?,
                passives: Vec::new(),
                passive_names: Vec::new(),
                equipped_moves: Vec::new(),
                mastered_moves: Vec::new(),
            })
        })
        .map_err(|e| e.to_string())?;

    let mut pals: Vec<PalView> = rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?;

    let mut passive_stmt = conn
        .prepare("SELECT instance_id, passive_id FROM pal_passives WHERE snapshot_id = ?1")
        .map_err(|e| e.to_string())?;
    let passive_rows: Vec<(String, String)> = passive_stmt
        .query_map([snapshot_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    let mut move_stmt = conn
        .prepare("SELECT instance_id, move_id, kind FROM pal_moves WHERE snapshot_id = ?1")
        .map_err(|e| e.to_string())?;
    let move_rows: Vec<(String, String, String)> = move_stmt
        .query_map([snapshot_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    for pal in &mut pals {
        pal.passives = passive_rows
            .iter()
            .filter(|(id, _)| *id == pal.instance_id)
            .map(|(_, p)| p.clone())
            .collect();
        pal.equipped_moves = move_rows
            .iter()
            .filter(|(id, _, kind)| *id == pal.instance_id && kind == "equipped")
            .map(|(_, m, _)| m.clone())
            .collect();
        pal.mastered_moves = move_rows
            .iter()
            .filter(|(id, _, kind)| *id == pal.instance_id && kind == "mastered")
            .map(|(_, m, _)| m.clone())
            .collect();
    }

    Ok(pals)
}

/// Every dex fact the store holds for a world: which species are unlocked
/// (append-only, across every snapshot) plus this snapshot's capture progress,
/// all as [`scope`](PlayerScope) asks for them.
///
/// Capture counts and capture bonuses are earned *per player* — see
/// [`PlayerScope`] for why aggregating them is a bug rather than a summary.
/// Unlocks are read the same way: `dex_events` records who first saw each
/// species, so scoping to a player answers "what has *this* player caught"
/// while [`PlayerScope::All`] keeps the world-level roll-up.
pub fn dex_facts(
    store: &Store,
    world_id: &str,
    snapshot_id: i64,
    scope: PlayerScope,
) -> Result<DexFacts, String> {
    let conn = store.conn();

    let unlocked_sql = format!(
        "SELECT DISTINCT character_id FROM dex_events WHERE world_id = ?1{} \
         ORDER BY character_id",
        scope.clause(2)
    );
    let mut stmt = conn.prepare(&unlocked_sql).map_err(|e| e.to_string())?;
    let mut unlocked_binds: Vec<&str> = vec![world_id];
    unlocked_binds.extend(scope.uid());
    let unlocked: Vec<String> = stmt
        .query_map(rusqlite::params_from_iter(unlocked_binds), |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    // `MAX` is a no-op under `Only` (one row per player per key) and the
    // world-level best under `All`, so one statement serves both.
    let int_flags = |kind: &str| -> Result<std::collections::HashMap<String, i64>, String> {
        let sql = format!(
            "SELECT flag_key, MAX(COALESCE(value, 0)) FROM player_flags
             WHERE snapshot_id = ?1 AND flag_kind = '{kind}'{}
             GROUP BY flag_key",
            scope.clause(2)
        );
        let mut s = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut binds: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(snapshot_id)];
        if let Some(uid) = scope.uid() {
            binds.push(Box::new(uid.to_owned()));
        }
        let rows = s
            .query_map(rusqlite::params_from_iter(binds.iter()), |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .map_err(|e| e.to_string())?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (key, value) = row.map_err(|e| e.to_string())?;
            out.insert(key, value);
        }
        Ok(out)
    };

    let capture_counts = int_flags("capture_count")?;
    let bonus_tiers = int_flags("capture_bonus_tier")?;

    Ok(DexFacts { unlocked, capture_counts, bonus_tiers })
}

/// Who is in this world, for the player selector.
///
/// Ordered by name so the selector is stable between syncs — `players` has no
/// inherent order, and a list that reshuffles under the user on every autosave
/// would be worse than an arbitrary but fixed one.
pub fn world_players(store: &Store, snapshot_id: i64) -> Result<Vec<WorldPlayerView>, String> {
    let conn = store.conn();
    let mut stmt = conn
        .prepare(
            "SELECT player_uid, name, level FROM players WHERE snapshot_id = ?1
             ORDER BY name IS NULL, name, player_uid",
        )
        .map_err(|e| e.to_string())?;
    let players = stmt
        .query_map([snapshot_id], |row| {
            Ok(WorldPlayerView {
                player_uid: row.get(0)?,
                name: row.get(1)?,
                level: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    Ok(players)
}

pub fn player_progress(store: &Store, snapshot_id: i64) -> Result<Vec<PlayerProgressView>, String> {
    let conn = store.conn();
    let mut stmt = conn
        .prepare(
            "SELECT player_uid, tech_points, boss_tech_points, pal_butcher_count,
                    mutation_count, awakening_count, camp_conquered_count,
                    normal_dungeon_clear_count, fixed_dungeon_clear_count,
                    relic_possess_total, treasures_found
             FROM players WHERE snapshot_id = ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([snapshot_id], |row| {
            Ok(PlayerProgressView {
                player_uid: row.get(0)?,
                tech_points: row.get(1)?,
                boss_tech_points: row.get(2)?,
                pal_butcher_count: row.get(3)?,
                mutation_count: row.get(4)?,
                awakening_count: row.get(5)?,
                camp_conquered_count: row.get(6)?,
                normal_dungeon_clear_count: row.get(7)?,
                fixed_dungeon_clear_count: row.get(8)?,
                relic_possess_total: row.get(9)?,
                treasures_found: row.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

pub fn base_summary(store: &Store, snapshot_id: i64) -> Result<Vec<BaseCampView>, String> {
    let conn = store.conn();
    let mut stmt = conn
        .prepare("SELECT id, guild_id FROM base_camps WHERE snapshot_id = ?1")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([snapshot_id], |row| Ok(BaseCampView { id: row.get(0)?, guild_id: row.get(1)? }))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// World & player progress beyond the scalar counters `player_progress`
/// already covers — tech tree, boss defeats, quest completion, collectibles
/// — all of it already sitting in `player_flags` since Phase 4's ingest, just
/// not queried back out until now. Base camp worker/storage detail isn't
/// included: `BaseCampSaveData`'s bespoke binary format (see
/// `paldex-model::rawdata::base_camp`'s docs) isn't decoded past id and
/// guild ownership yet.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerFlagsView {
    pub player_uid: String,
    pub unlocked_tech: Vec<String>,
    /// Localized technology names, parallel to `unlocked_tech`, falling back
    /// to the raw id when unresolved. Empty when no game pak is available.
    pub unlocked_tech_names: Vec<String>,
    pub normal_boss_defeated: Vec<String>,
    pub tower_boss_defeated: Vec<String>,
    pub specific_boss_defeated: Vec<String>,
    pub completed_quests: Vec<String>,
    pub relics_obtained: Vec<String>,
    pub notes_obtained: Vec<String>,
    pub fast_travel_unlocked: Vec<String>,
}

pub fn player_flags_detail(store: &Store, snapshot_id: i64) -> Result<Vec<PlayerFlagsView>, String> {
    let conn = store.conn();
    let mut player_stmt = conn
        .prepare("SELECT player_uid FROM players WHERE snapshot_id = ?1")
        .map_err(|e| e.to_string())?;
    let player_uids: Vec<String> = player_stmt
        .query_map([snapshot_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;

    let mut flag_stmt = conn
        .prepare(
            "SELECT flag_key FROM player_flags
             WHERE snapshot_id = ?1 AND player_uid = ?2 AND flag_kind = ?3
             ORDER BY flag_key",
        )
        .map_err(|e| e.to_string())?;
    let mut keys_for = |player_uid: &str, kind: &str| -> Result<Vec<String>, String> {
        flag_stmt
            .query_map(rusqlite::params![snapshot_id, player_uid, kind], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())
    };

    player_uids
        .into_iter()
        .map(|player_uid| {
            Ok(PlayerFlagsView {
                unlocked_tech: keys_for(&player_uid, "unlocked_tech")?,
                unlocked_tech_names: Vec::new(),
                normal_boss_defeated: keys_for(&player_uid, "normal_boss_defeated")?,
                tower_boss_defeated: keys_for(&player_uid, "tower_boss_defeated")?,
                specific_boss_defeated: keys_for(&player_uid, "specific_boss_defeated")?,
                completed_quests: keys_for(&player_uid, "completed_quest")?,
                relics_obtained: keys_for(&player_uid, "relic_obtained")?,
                notes_obtained: keys_for(&player_uid, "note_obtained")?,
                fast_travel_unlocked: keys_for(&player_uid, "fast_travel_unlocked")?,
                player_uid,
            })
        })
        .collect()
}
