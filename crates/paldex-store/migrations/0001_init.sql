-- Local-only Paldex SQLite schema.
--
-- Current-state tables (pals, players, guilds, base_camps, pal_passives,
-- pal_moves, player_flags) are keyed by snapshot_id and replaced wholesale
-- each snapshot -- see paldex-store's ingest(). dex_events is the one
-- exception: append-only, never deleted, because dex completion must be
-- monotonic -- releasing or butchering a Pal must not un-catch it.

CREATE TABLE worlds (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    steam_id TEXT NOT NULL,
    path TEXT NOT NULL,
    last_seen INTEGER NOT NULL -- epoch millis
);

CREATE TABLE snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
    taken_at INTEGER NOT NULL, -- epoch millis
    level_mtime INTEGER,       -- epoch millis, nullable
    level_hash TEXT NOT NULL
);
CREATE INDEX idx_snapshots_world ON snapshots(world_id, taken_at DESC);

CREATE TABLE pals (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    instance_id TEXT NOT NULL,
    character_id TEXT NOT NULL,
    owner TEXT,
    level INTEGER NOT NULL,
    rank INTEGER NOT NULL,
    soul_hp INTEGER NOT NULL,
    soul_attack INTEGER NOT NULL,
    soul_defense INTEGER NOT NULL,
    soul_craft_speed INTEGER NOT NULL,
    iv_hp INTEGER NOT NULL,
    iv_shot INTEGER NOT NULL,
    iv_defense INTEGER NOT NULL,
    gender TEXT NOT NULL,
    is_lucky INTEGER NOT NULL, -- 0/1
    is_boss INTEGER NOT NULL,
    is_predator INTEGER NOT NULL,
    nickname TEXT,
    location_container_id TEXT,
    location_slot_index INTEGER,
    location_kind TEXT, -- 'party' | 'box' | 'other'
    PRIMARY KEY (snapshot_id, instance_id)
);
CREATE INDEX idx_pals_snapshot_species ON pals(snapshot_id, character_id);

CREATE TABLE pal_passives (
    snapshot_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    passive_id TEXT NOT NULL,
    FOREIGN KEY (snapshot_id, instance_id) REFERENCES pals(snapshot_id, instance_id) ON DELETE CASCADE
);
CREATE INDEX idx_pal_passives ON pal_passives(snapshot_id, passive_id);

CREATE TABLE pal_moves (
    snapshot_id INTEGER NOT NULL,
    instance_id TEXT NOT NULL,
    move_id TEXT NOT NULL,
    kind TEXT NOT NULL, -- 'equipped' | 'mastered'
    FOREIGN KEY (snapshot_id, instance_id) REFERENCES pals(snapshot_id, instance_id) ON DELETE CASCADE
);

CREATE TABLE players (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    player_uid TEXT NOT NULL,
    tech_points INTEGER NOT NULL,
    boss_tech_points INTEGER NOT NULL,
    party_container_id TEXT,
    box_container_id TEXT,
    pal_butcher_count INTEGER NOT NULL,
    pal_rankup_count INTEGER NOT NULL,
    mutation_count INTEGER NOT NULL,
    awakening_count INTEGER NOT NULL,
    camp_conquered_count INTEGER NOT NULL,
    oilrig_clear_count INTEGER NOT NULL,
    normal_dungeon_clear_count INTEGER NOT NULL,
    fixed_dungeon_clear_count INTEGER NOT NULL,
    tribe_capture_count INTEGER NOT NULL,
    predator_defeat_count INTEGER NOT NULL,
    relic_possess_total INTEGER NOT NULL,
    treasures_found INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, player_uid)
);

-- Generic per-player set/map data -- dex unlocks, capture counts, unlocked
-- tech, boss defeats, collectibles, quests -- one row per (kind, key[, value]).
-- Chosen over a dozen narrow tables because PlayerProgress's shape (see
-- paldex-model) is a set of ~15 semantically-similar Map<String,_>/HashSet<String>
-- fields; this stores all of them uniformly instead of one table each.
CREATE TABLE player_flags (
    snapshot_id INTEGER NOT NULL,
    player_uid TEXT NOT NULL,
    flag_kind TEXT NOT NULL, -- e.g. 'paldeck_unlocked', 'capture_count', 'unlocked_tech'
    flag_key TEXT NOT NULL,  -- species/tech/boss/quest id
    value INTEGER,           -- count for *_count kinds, NULL for presence-only kinds
    FOREIGN KEY (snapshot_id, player_uid) REFERENCES players(snapshot_id, player_uid) ON DELETE CASCADE
);
CREATE INDEX idx_player_flags ON player_flags(snapshot_id, player_uid, flag_kind);

CREATE TABLE guilds (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    kind TEXT NOT NULL, -- 'organization' | 'guild' | 'other:<raw>'
    PRIMARY KEY (snapshot_id, id)
);

CREATE TABLE base_camps (
    snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
    id TEXT NOT NULL,
    guild_id TEXT,
    PRIMARY KEY (snapshot_id, id)
);

-- Append-only. Never DELETE or UPDATE a row here outside of pruning a whole
-- world's data -- see the module docs above.
CREATE TABLE dex_events (
    world_id TEXT NOT NULL REFERENCES worlds(id) ON DELETE CASCADE,
    player_uid TEXT NOT NULL,
    character_id TEXT NOT NULL,
    first_seen_at INTEGER NOT NULL, -- epoch millis
    PRIMARY KEY (world_id, player_uid, character_id)
);
