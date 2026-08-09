/**
 * Mirrors the serde representation of the types in `crates/paldex-locate`.
 * Keep these in sync with `SaveSource`, `WorldKind`, `World`, and `SaveRootView`.
 */

export type SaveSource =
  | { kind: "windowsSteam" }
  | { kind: "crossOver"; bottle: string }
  | { kind: "whisky"; bottle: string }
  | { kind: "manual" };

export type WorldKind = "localWorld" | "coopGuestStub" | "unknown";

export interface World {
  id: string;
  path: string;
  kind: WorldKind;
  /** Epoch milliseconds, or null when the timestamp could not be read. */
  lastPlayed: number | null;
  players: string[];
}

export interface SaveRootView {
  path: string;
  source: SaveSource;
  steamId: string;
  sourceLabel: string;
  worlds: World[];
}

/**
 * Mirrors `crates/apps/paldex-app/src-tauri/src/queries.rs`'s view structs —
 * the SQLite-store-backed data behind the roster/dex/player screens.
 */

export interface SnapshotSummaryView {
  snapshotId: number;
  palCount: number;
  playerCount: number;
  /** Epoch milliseconds. */
  takenAt: number;
}

export type LocationKind = "party" | "box" | "other" | null;

/**
 * One of a species' elements. `id` is the game's internal enum name (`Leaf`,
 * `Earth`); `name` is the localized label it shows (`Grass`, `Ground`). Colour
 * and any other styling must key off `id` — `name` changes with the language.
 */
export interface ElementView {
  id: string;
  name: string;
}

/** A job a species can do, already in the game's own display order. */
export interface WorkSuitabilityView {
  id: string;
  name: string;
  level: number;
}

/**
 * Authored per-species base stats — the inputs the game combines with level,
 * IVs and souls, not a finished stat line.
 */
export interface BaseStatsView {
  hp: number;
  meleeAttack: number;
  shotAttack: number;
  defense: number;
  support: number;
  craftSpeed: number;
}

export interface PalView {
  instanceId: string;
  characterId: string;
  /** Localized species name from the game pak; null when no pak was found. */
  displayName: string | null;
  /** Empty without a pak, and for the few genuinely elementless species. */
  elements: ElementView[];
  /** Species rarity tier; 0 without a pak. */
  rarity: number;
  owner: string | null;
  level: number;
  rank: number;
  soulHp: number;
  soulAttack: number;
  soulDefense: number;
  soulCraftSpeed: number;
  ivHp: number;
  ivShot: number;
  ivDefense: number;
  gender: "male" | "female" | "unknown";
  isLucky: boolean;
  isBoss: boolean;
  isPredator: boolean;
  nickname: string | null;
  locationKind: LocationKind;
  passives: string[];
  /** Localized passive names, parallel to `passives`. */
  passiveNames: string[];
  equippedMoves: string[];
  masteredMoves: string[];
}

export interface DexEntryView {
  characterId: string;
  displayName: string;
  /** Paldeck number as the game shows it, e.g. "005B". */
  dexLabel: string | null;
  caught: boolean;
  /** Best `PalCaptureCount` across players, toward the 10-capture bonus. */
  captureCount: number;
  bonusClaimed: boolean;
  /** All four are empty/null without a pak. */
  elements: ElementView[];
  rarity: number;
  stats: BaseStatsView | null;
  workSuitabilities: WorkSuitabilityView[];
}

export interface DexProgressView {
  unlockedSpeciesCount: number;
  unlockedSpecies: string[];
  /** Total Paldeck entries, or 0 without a pak. */
  totalSpeciesCount: number;
  entries: DexEntryView[];
}

export interface PlayerProgressView {
  playerUid: string;
  techPoints: number;
  bossTechPoints: number;
  palButcherCount: number;
  mutationCount: number;
  awakeningCount: number;
  campConqueredCount: number;
  normalDungeonClearCount: number;
  fixedDungeonClearCount: number;
  relicPossessTotal: number;
  treasuresFound: number;
}

export interface BaseCampView {
  id: string;
  guildId: string | null;
}

export interface PlayerFlagsView {
  playerUid: string;
  unlockedTech: string[];
  /** Localized technology names, parallel to `unlockedTech`. */
  unlockedTechNames: string[];
  normalBossDefeated: string[];
  towerBossDefeated: string[];
  specificBossDefeated: string[];
  completedQuests: string[];
  relicsObtained: string[];
  notesObtained: string[];
  fastTravelUnlocked: string[];
}
