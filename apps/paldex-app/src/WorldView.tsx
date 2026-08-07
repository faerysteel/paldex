import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import type { SnapshotSummaryView } from "./types";
import { relativeTime } from "./time";
import Roster from "./Roster";
import Dex from "./Dex";

/**
 * Emitted by the Rust watcher after every automatic re-sync that ingested a
 * new snapshot. Must match `commands::SNAPSHOT_EVENT`.
 */
const SNAPSHOT_EVENT = "paldex://snapshot";
/** How long the "updated" flash stays up after an automatic sync. */
const FLASH_MS = 4000;

type Tab = "dex" | "roster";

const TABS: { id: Tab; label: string }[] = [
  { id: "dex", label: "Paldex" },
  { id: "roster", label: "Roster" },
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

      {tab === "dex" && <Dex summary={summary} />}
      {tab === "roster" && <Roster summary={summary} />}
    </div>
  );
}
