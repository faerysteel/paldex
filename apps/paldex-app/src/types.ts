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
