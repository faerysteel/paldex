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

/**
 * One player in the selected world, for the player selector.
 *
 * Most of what Palworld tracks — capture counts, capture bonuses, the Paldeck
 * itself — is per player, so a shared world has no single answer to "how far
 * along is this save". Every screen takes a `playerUid`, and `null` means the
 * world as a whole.
 */
export interface WorldPlayerView {
  playerUid: string;
  /** Falls back to a shortened uid in the UI when the save didn't name them. */
  name: string | null;
  level: number | null;
}

/**
 * Captures of one species needed to complete its capture bonus. Mirrors
 * `paldex_model::CAPTURE_BONUS_AT`; see that constant for how it was derived.
 */
export const CAPTURE_BONUS_AT = 5;

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
  /**
   * `PalCaptureCount` for the selected player, or the best across players
   * when no player is selected.
   */
  captureCount: number;
  /** Capture-bonus tier, 0..=CAPTURE_BONUS_AT; the top value means complete. */
  bonusTier: number;
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

/** Composite-IV tier from `analysis::IvTier`, serialized as the variant name. */
export type IvTier = "D" | "C" | "B" | "A" | "S" | "Perfect";

/** One owned Pal as the analysis screen shows it — a trimmed `PalView`. */
export interface GradedPalView {
  instanceId: string;
  characterId: string;
  /** Localized species name; null without a pak. */
  displayName: string | null;
  nickname: string | null;
  level: number;
  rank: number;
  gender: "male" | "female" | "unknown";
  ivHp: number;
  ivShot: number;
  ivDefense: number;
  /** Mean of the three talents. */
  composite: number;
  tier: IvTier;
  /** Localized passive names, falling back to raw ids. */
  passiveNames: string[];
}

/**
 * The quality screen's lists. The three derived lists hold instance ids into
 * `graded` rather than repeating the rows.
 */
export interface PalQualityView {
  graded: GradedPalView[];
  bestOfSpecies: string[];
  condenseCandidates: string[];
  passiveRanking: string[];
}

export interface BreedingParentView {
  instanceId: string;
  characterId: string;
  displayName: string | null;
  nickname: string | null;
  level: number;
  gender: "male" | "female" | "unknown";
  ivHp: number;
  ivShot: number;
  ivDefense: number;
}

/**
 * One side of a pairing involving a species you don't own. `owned` is the best
 * specimen when the species is in your roster, and null when it is the side
 * you would have to obtain.
 */
export interface PairingSideView {
  characterId: string;
  displayName: string | null;
  /** Paldeck number as the game shows it, e.g. "005B". */
  dexLabel: string | null;
  owned: BreedingParentView | null;
}

/** A species the breeding picker may be asked for — one breeding can produce. */
export interface BreedingTargetView {
  characterId: string;
  displayName: string;
  /** Paldeck number as the game shows it, or null for a species with none. */
  dexLabel: string | null;
}

/**
 * What a pairing is waiting on, smallest ask first. Every one of these is a
 * Pal you don't currently have — a female Lamball you don't own is as much an
 * errand as a Lamball you don't own.
 */
export type PairingNeed =
  | "secondSpecimen"
  | "oppositeGender"
  | "oneSpecies"
  | "twoSpecies";

/**
 * A pairing you can't breed today, and what it's waiting on — a species
 * missing from your roster, a second specimen of one already in it, or one of
 * the opposite gender.
 */
export interface UnownedPairingView {
  parentA: PairingSideView;
  parentB: PairingSideView;
  need: PairingNeed;
  /** The gender every candidate shares, for `oppositeGender`; null otherwise. */
  blockingGender: "male" | "female" | null;
  /** Species missing from your roster entirely; empty when you own both sides. */
  missingSpecies: string[];
}

/** A suggested pairing, carrying the numbers behind its ranking. */
export interface BreedingPairView {
  parentA: BreedingParentView;
  parentB: BreedingParentView;
  /** Mean of the two parents' composite IV scores. */
  parentIvAverage: number;
  /** Localized names of every distinct passive across both parents. */
  inheritedPassives: string[];
  score: number;
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
