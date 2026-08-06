use paldex_model::{BaseCamp, Gender, Guild, GroupKind, Pal, PalLocationKind, PlayerProgress};
use rusqlite::{params, Transaction};

use crate::{now_millis, StoreError};

/// One fully-decoded world snapshot, ready to ingest.
#[derive(Debug, Default)]
pub struct SnapshotInput {
    pub taken_at: i64,
    pub level_mtime: Option<i64>,
    pub level_hash: String,
    pub pals: Vec<Pal>,
    pub players: Vec<PlayerProgress>,
    pub guilds: Vec<Guild>,
    pub base_camps: Vec<BaseCamp>,
}

impl SnapshotInput {
    /// Convenience constructor stamping `taken_at` with the current time.
    pub fn now(level_mtime: Option<i64>, level_hash: String) -> Result<Self, StoreError> {
        Ok(Self {
            taken_at: now_millis()?,
            level_mtime,
            level_hash,
            ..Self::default()
        })
    }
}

pub(crate) fn insert_snapshot(
    tx: &Transaction,
    world_id: &str,
    input: &SnapshotInput,
) -> Result<i64, StoreError> {
    tx.execute(
        "INSERT INTO snapshots (world_id, taken_at, level_mtime, level_hash)
         VALUES (?1, ?2, ?3, ?4)",
        params![world_id, input.taken_at, input.level_mtime, input.level_hash],
    )?;
    Ok(tx.last_insert_rowid())
}

pub(crate) fn insert_pals(tx: &Transaction, snapshot_id: i64, pals: &[Pal]) -> Result<(), StoreError> {
    let mut pal_stmt = tx.prepare(
        "INSERT INTO pals (
             snapshot_id, instance_id, character_id, owner, level, rank,
             soul_hp, soul_attack, soul_defense, soul_craft_speed,
             iv_hp, iv_shot, iv_defense, gender, is_lucky, is_boss, is_predator,
             nickname, location_container_id, location_slot_index, location_kind
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)",
    )?;
    let mut passive_stmt = tx.prepare(
        "INSERT INTO pal_passives (snapshot_id, instance_id, passive_id) VALUES (?1,?2,?3)",
    )?;
    let mut move_stmt = tx.prepare(
        "INSERT INTO pal_moves (snapshot_id, instance_id, move_id, kind) VALUES (?1,?2,?3,?4)",
    )?;

    for pal in pals {
        let instance_id = pal.instance_id.to_string();
        let (location_container_id, location_slot_index, location_kind) = match pal.location {
            Some(loc) => (
                Some(loc.container_id.to_string()),
                Some(i64::from(loc.slot_index)),
                Some(location_kind_str(loc.kind)),
            ),
            None => (None, None, None),
        };

        pal_stmt.execute(params![
            snapshot_id,
            instance_id,
            pal.character_id,
            pal.owner.map(|u| u.to_string()),
            i64::from(pal.level),
            i64::from(pal.rank),
            i64::from(pal.souls.hp),
            i64::from(pal.souls.attack),
            i64::from(pal.souls.defense),
            i64::from(pal.souls.craft_speed),
            i64::from(pal.ivs.hp),
            i64::from(pal.ivs.shot),
            i64::from(pal.ivs.defense),
            gender_str(pal.gender),
            pal.is_lucky,
            pal.is_boss,
            pal.is_predator,
            pal.nickname,
            location_container_id,
            location_slot_index,
            location_kind,
        ])?;

        for passive in &pal.passives {
            passive_stmt.execute(params![snapshot_id, instance_id, passive])?;
        }
        for m in &pal.equipped_moves {
            move_stmt.execute(params![snapshot_id, instance_id, m, "equipped"])?;
        }
        for m in &pal.mastered_moves {
            move_stmt.execute(params![snapshot_id, instance_id, m, "mastered"])?;
        }
    }
    Ok(())
}

pub(crate) fn insert_players(
    tx: &Transaction,
    snapshot_id: i64,
    players: &[PlayerProgress],
) -> Result<(), StoreError> {
    let mut player_stmt = tx.prepare(
        "INSERT INTO players (
             snapshot_id, player_uid, tech_points, boss_tech_points,
             party_container_id, box_container_id, pal_butcher_count,
             pal_rankup_count, mutation_count, awakening_count,
             camp_conquered_count, oilrig_clear_count, normal_dungeon_clear_count,
             fixed_dungeon_clear_count, tribe_capture_count, predator_defeat_count,
             relic_possess_total, treasures_found
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
    )?;
    let mut flag_stmt = tx.prepare(
        "INSERT INTO player_flags (snapshot_id, player_uid, flag_kind, flag_key, value)
         VALUES (?1,?2,?3,?4,?5)",
    )?;

    for p in players {
        let player_uid = p.player_uid.to_string();
        player_stmt.execute(params![
            snapshot_id,
            player_uid,
            i64::from(p.tech_points),
            i64::from(p.boss_tech_points),
            p.party_container_id.map(|u| u.to_string()),
            p.box_container_id.map(|u| u.to_string()),
            i64::from(p.stats.pal_butcher_count),
            i64::from(p.stats.pal_rankup_count),
            i64::from(p.stats.mutation_count),
            i64::from(p.stats.awakening_count),
            i64::from(p.stats.camp_conquered_count),
            i64::from(p.stats.oilrig_clear_count),
            i64::from(p.stats.normal_dungeon_clear_count),
            i64::from(p.stats.fixed_dungeon_clear_count),
            i64::from(p.stats.tribe_capture_count),
            i64::from(p.bosses.predator_defeat_count),
            i64::from(p.collectibles.relic_possess_total),
            i64::from(p.collectibles.treasures_found),
        ])?;

        let mut set_flags = |kind: &str, keys: &std::collections::HashSet<String>| -> Result<(), StoreError> {
            for k in keys {
                flag_stmt.execute(params![snapshot_id, player_uid, kind, k, Option::<i64>::None])?;
            }
            Ok(())
        };
        set_flags("paldeck_unlocked", &p.paldeck_unlocked)?;
        set_flags("capture_bonus_claimed", &p.capture_bonus_claimed)?;
        set_flags("fast_travel_unlocked", &p.fast_travel_unlocked)?;
        set_flags("normal_boss_defeated", &p.bosses.normal_defeated)?;
        set_flags("tower_boss_defeated", &p.bosses.tower_defeated)?;
        set_flags("specific_boss_defeated", &p.bosses.specific_defeated)?;
        set_flags("relic_obtained", &p.collectibles.relics_obtained)?;
        set_flags("note_obtained", &p.collectibles.notes_obtained)?;

        let mut count_flags = |kind: &str, entries: &std::collections::HashMap<String, u32>| -> Result<(), StoreError> {
            for (k, v) in entries {
                flag_stmt.execute(params![snapshot_id, player_uid, kind, k, i64::from(*v)])?;
            }
            Ok(())
        };
        count_flags("capture_count", &p.capture_counts)?;
        count_flags("tower_boss_defeat_count", &p.bosses.tower_defeat_counts)?;
        count_flags("raid_boss_defeat_count", &p.bosses.raid_defeat_counts)?;
        count_flags("relic_possess_count", &p.collectibles.relic_possess_counts)?;

        for tech in &p.unlocked_tech {
            flag_stmt.execute(params![snapshot_id, player_uid, "unlocked_tech", tech, Option::<i64>::None])?;
        }
        for quest in &p.quests.completed_quest_ids {
            flag_stmt.execute(params![snapshot_id, player_uid, "completed_quest", quest, Option::<i64>::None])?;
        }
        for quest in &p.quests.ordered_quest_ids {
            flag_stmt.execute(params![snapshot_id, player_uid, "ordered_quest", quest, Option::<i64>::None])?;
        }
    }
    Ok(())
}

pub(crate) fn insert_guilds(tx: &Transaction, snapshot_id: i64, guilds: &[Guild]) -> Result<(), StoreError> {
    let mut stmt = tx.prepare("INSERT INTO guilds (snapshot_id, id, kind) VALUES (?1,?2,?3)")?;
    for g in guilds {
        let kind = match &g.kind {
            GroupKind::Organization => "organization".to_owned(),
            GroupKind::Guild => "guild".to_owned(),
            GroupKind::Other(s) => format!("other:{s}"),
        };
        stmt.execute(params![snapshot_id, g.id.to_string(), kind])?;
    }
    Ok(())
}

pub(crate) fn insert_base_camps(
    tx: &Transaction,
    snapshot_id: i64,
    base_camps: &[BaseCamp],
) -> Result<(), StoreError> {
    let mut stmt =
        tx.prepare("INSERT INTO base_camps (snapshot_id, id, guild_id) VALUES (?1,?2,?3)")?;
    for b in base_camps {
        stmt.execute(params![
            snapshot_id,
            b.id.to_string(),
            b.guild_id.map(|u| u.to_string())
        ])?;
    }
    Ok(())
}

/// Append newly-unlocked dex entries. `INSERT OR IGNORE` means an entry
/// already recorded (from an earlier snapshot) keeps its original
/// `first_seen_at` — this is the whole mechanism behind dex completion being
/// monotonic across snapshots.
pub(crate) fn append_dex_events(
    tx: &Transaction,
    world_id: &str,
    taken_at: i64,
    players: &[PlayerProgress],
) -> Result<(), StoreError> {
    let mut stmt = tx.prepare(
        "INSERT OR IGNORE INTO dex_events (world_id, player_uid, character_id, first_seen_at)
         VALUES (?1,?2,?3,?4)",
    )?;
    for p in players {
        let player_uid = p.player_uid.to_string();
        for species in &p.paldeck_unlocked {
            stmt.execute(params![world_id, player_uid, species, taken_at])?;
        }
    }
    Ok(())
}

fn gender_str(g: Gender) -> &'static str {
    match g {
        Gender::Male => "male",
        Gender::Female => "female",
        Gender::Unknown => "unknown",
    }
}

fn location_kind_str(k: PalLocationKind) -> &'static str {
    match k {
        PalLocationKind::Party => "party",
        PalLocationKind::Box => "box",
        PalLocationKind::Other => "other",
    }
}
