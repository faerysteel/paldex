//! Read-only reporting queries against the store's latest snapshot for a
//! world — the data the roster table, dex grid, and player panel render.

use paldex_store::Store;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PalView {
    pub instance_id: String,
    pub character_id: String,
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
    pub equipped_moves: Vec<String>,
    pub mastered_moves: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DexProgressView {
    /// Species unlocked across every player in this world (a species any
    /// player has caught counts once). No denominator/percentage yet — that
    /// needs Phase 3's species reference table, currently a stub (see
    /// `paldex-data`'s `ReferenceData` docs).
    pub unlocked_species_count: i64,
    pub unlocked_species: Vec<String>,
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

#[derive(Debug, Serialize)]
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

pub fn pal_roster(store: &Store, snapshot_id: i64) -> Result<Vec<PalView>, String> {
    let conn = store.conn();
    let mut stmt = conn
        .prepare(
            "SELECT instance_id, character_id, owner, level, rank,
                    soul_hp, soul_attack, soul_defense, soul_craft_speed,
                    iv_hp, iv_shot, iv_defense, gender, is_lucky, is_boss,
                    is_predator, nickname, location_kind
             FROM pals WHERE snapshot_id = ?1 ORDER BY character_id, level DESC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([snapshot_id], |row| {
            Ok(PalView {
                instance_id: row.get(0)?,
                character_id: row.get(1)?,
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

pub fn dex_progress(store: &Store, world_id: &str) -> Result<DexProgressView, String> {
    let conn = store.conn();
    let mut stmt = conn
        .prepare("SELECT DISTINCT character_id FROM dex_events WHERE world_id = ?1 ORDER BY character_id")
        .map_err(|e| e.to_string())?;
    let unlocked_species: Vec<String> = stmt
        .query_map([world_id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    Ok(DexProgressView {
        unlocked_species_count: unlocked_species.len() as i64,
        unlocked_species,
    })
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
