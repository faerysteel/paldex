import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { SnapshotSummaryView, WorldPlayerView } from "./types";
import { relativeTime } from "./time";
import Roster from "./Roster";
import Dex from "./Dex";
import Analysis from "./Analysis";
import Breeding from "./Breeding";
import Bases from "./Bases";

/**
 * Emitted by the Rust watcher after every automatic re-sync that ingested a
 * new snapshot. Must match `commands::SNAPSHOT_EVENT`.
 */
const SNAPSHOT_EVENT = "paldex://snapshot";
/** How long the "updated" flash stays up after an automatic sync. */
const FLASH_MS = 4000;

type Tab = "dex" | "roster" | "analysis" | "breeding" | "bases";

const TABS: { id: Tab; label: string }[] = [
  { id: "dex", label: "Paldex" },
  { id: "roster", label: "Roster" },
  { id: "analysis", label: "Analysis" },
  { id: "breeding", label: "Breeding" },
  { id: "bases", label: "Bases" },
];

interface Props {
  summary: SnapshotSummaryView;
  onBack: () => void;
  onSummaryChange: (summary: SnapshotSummaryView) => void;
}

/**
 * The selected world's screens, and everything they share: the current
 * snapshot, the manual resync, and the live-sync subscription.
 *
 * The subscription lives here rather than in a tab because it must survive tab
 * switches — a listener mounted inside `Roster` would be torn down the moment
 * the user looked at the dex, silently reverting the app to manual-only sync.
 */
export default function WorldView({ summary, onBack, onSummaryChange }: Props) {
  const [tab, setTab] = useState<Tab>("dex");
  const [error, setError] = useState<string | null>(null);
  const [resyncing, setResyncing] = useState(false);
  const [lastAutoSync, setLastAutoSync] = useState<number | null>(null);
  const [players, setPlayers] = useState<WorldPlayerView[]>([]);
  // `null` is "all players". Lives here rather than in a tab for the same
  // reason the sync listener does: switching tabs must not reset it, and the
  // whole point of the selector is that it applies across screens.
  const [playerUid, setPlayerUid] = useState<string | null>(null);

  // Re-fetched on every snapshot: a player who joins the world mid-session
  // should appear without a restart.
  useEffect(() => {
    invoke<WorldPlayerView[]>("world_players")
      .then(setPlayers)
      .catch((e: unknown) => setError(String(e)));
  }, [summary.snapshotId]);

  // A selected player who is no longer in the world would silently filter
  // everything down to nothing, so fall back to all players.
  useEffect(() => {
    if (playerUid !== null && !players.some((p) => p.playerUid === playerUid)) {
      setPlayerUid(null);
    }
  }, [players, playerUid]);

  useEffect(() => {
    const pending = listen<SnapshotSummaryView>(SNAPSHOT_EVENT, (event) => {
      console.info(`[paldex] auto-sync -> snapshot ${event.payload.snapshotId}`);
      onSummaryChange(event.payload);
      setLastAutoSync(Date.now());
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, [onSummaryChange]);

  // A timestamp rather than a boolean, so a second auto-sync arriving while
  // the first is still showing restarts the flash instead of being swallowed.
  useEffect(() => {
    if (lastAutoSync === null) return;
    const timer = setTimeout(() => setLastAutoSync(null), FLASH_MS);
    return () => clearTimeout(timer);
  }, [lastAutoSync]);

  const resync = useCallback(async () => {
    setResyncing(true);
    setError(null);
    try {
      onSummaryChange(await invoke<SnapshotSummaryView>("force_resync"));
    } catch (e) {
      setError(String(e));
    } finally {
      setResyncing(false);
    }
  }, [onSummaryChange]);

  return (
    <div className="roster">
      <header className="header">
        <div>
          <h1>{TABS.find((t) => t.id === tab)?.label}</h1>
          <p className="tagline">
            {summary.palCount} pals · {summary.playerCount} player
            {summary.playerCount === 1 ? "" : "s"} · synced{" "}
            {relativeTime(summary.takenAt)}
            {lastAutoSync !== null && (
              <span className="live-flash"> · updated from a new save</span>
            )}
          </p>
          <p className="muted watching">
            Watching for in-game saves — this updates on its own.
          </p>
        </div>
        <div className="actions">
          {players.length > 1 && (
            <label className="player-picker">
              <span className="muted">Player</span>
              <select
                value={playerUid ?? ""}
                onChange={(e) => setPlayerUid(e.target.value || null)}
              >
                <option value="">All players</option>
                {players.map((p) => (
                  <option key={p.playerUid} value={p.playerUid}>
                    {playerLabel(p)}
                  </option>
                ))}
              </select>
            </label>
          )}
          <button className="btn" onClick={() => void resync()} disabled={resyncing}>
            {resyncing ? "Syncing…" : "Resync"}
          </button>
          <button className="btn btn-ghost" onClick={onBack}>
            ← Back
          </button>
        </div>
      </header>

      <nav className="tabs">
        {TABS.map((t) => (
          <button
            key={t.id}
            className={tab === t.id ? "tab tab-on" : "tab"}
            onClick={() => setTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </nav>

      {error && <p className="notice">{error}</p>}

      {tab === "dex" && <Dex summary={summary} playerUid={playerUid} />}
      {tab === "roster" && <Roster summary={summary} playerUid={playerUid} />}
      {tab === "analysis" && <Analysis summary={summary} playerUid={playerUid} />}
      {tab === "breeding" && <Breeding summary={summary} playerUid={playerUid} />}
      {/* No `playerUid`: bases belong to the guild, not a player. Omitting it
          here is what makes the tab's world scope visible at the call site. */}
      {tab === "bases" && <Bases summary={summary} />}
    </div>
  );
}

/** A player's name, falling back to a short uid when the save didn't name them. */
function playerLabel(p: WorldPlayerView): string {
  const name = p.name ?? `${p.playerUid.slice(0, 8)}…`;
  return p.level === null ? name : `${name} (Lv ${p.level})`;
}
