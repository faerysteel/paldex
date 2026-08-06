import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type {
  DexProgressView,
  PalView,
  PlayerFlagsView,
  PlayerProgressView,
  SnapshotSummaryView,
} from "./types";
import { relativeTime } from "./time";

type Sort = { column: SortColumn; direction: "asc" | "desc" };
type SortColumn = "characterId" | "level" | "ivAvg" | "rank";

interface Props {
  summary: SnapshotSummaryView;
  onBack: () => void;
  onSummaryChange: (summary: SnapshotSummaryView) => void;
}

export default function Roster({ summary, onBack, onSummaryChange }: Props) {
  const [pals, setPals] = useState<PalView[] | null>(null);
  const [dex, setDex] = useState<DexProgressView | null>(null);
  const [players, setPlayers] = useState<PlayerProgressView[] | null>(null);
  const [playerFlags, setPlayerFlags] = useState<PlayerFlagsView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [resyncing, setResyncing] = useState(false);
  const [filter, setFilter] = useState("");
  const [sort, setSort] = useState<Sort>({ column: "level", direction: "desc" });

  const load = useCallback(async () => {
    setError(null);
    try {
      const [palsResult, dexResult, playersResult, flagsResult] = await Promise.all([
        invoke<PalView[]>("pal_roster"),
        invoke<DexProgressView>("dex_progress"),
        invoke<PlayerProgressView[]>("player_progress"),
        invoke<PlayerFlagsView[]>("player_flags_detail"),
      ]);
      setPals(palsResult);
      setDex(dexResult);
      setPlayers(playersResult);
      setPlayerFlags(flagsResult);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, summary.snapshotId]);

  const resync = useCallback(async () => {
    setResyncing(true);
    setError(null);
    try {
      const next = await invoke<SnapshotSummaryView>("force_resync");
      onSummaryChange(next);
    } catch (e) {
      setError(String(e));
    } finally {
      setResyncing(false);
    }
  }, [onSummaryChange]);

  const filtered = useMemo(() => {
    if (!pals) return [];
    const needle = filter.trim().toLowerCase();
    const matches = needle
      ? pals.filter(
          (p) =>
            p.characterId.toLowerCase().includes(needle) ||
            (p.nickname?.toLowerCase().includes(needle) ?? false),
        )
      : pals;
    const sorted = [...matches].sort((a, b) => {
      const dir = sort.direction === "asc" ? 1 : -1;
      switch (sort.column) {
        case "characterId":
          return dir * a.characterId.localeCompare(b.characterId);
        case "level":
          return dir * (a.level - b.level);
        case "rank":
          return dir * (a.rank - b.rank);
        case "ivAvg":
          return dir * (ivAverage(a) - ivAverage(b));
      }
    });
    return sorted;
  }, [pals, filter, sort]);

  const toggleSort = (column: SortColumn) => {
    setSort((prev) =>
      prev.column === column
        ? { column, direction: prev.direction === "asc" ? "desc" : "asc" }
        : { column, direction: "desc" },
    );
  };

  return (
    <div className="roster">
      <header className="header">
        <div>
          <h1>Roster</h1>
          <p className="tagline">
            {summary.palCount} pals · {summary.playerCount} player
            {summary.playerCount === 1 ? "" : "s"} · synced{" "}
            {relativeTime(summary.takenAt)}
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

      {error && <p className="notice">{error}</p>}

      {dex && (
        <p className="muted dex-summary">
          Dex: {dex.unlockedSpeciesCount} species unlocked (across every player
          — a total isn't shown yet, since species reference data isn't wired
          up)
        </p>
      )}

      {players && players.length > 0 && (
        <details className="others">
          <summary>Player progress</summary>
          <ul className="worlds">
            {players.map((p) => (
              <li key={p.playerUid} className="world">
                <div className="world-main">
                  <span className="world-id" title={p.playerUid}>
                    {p.playerUid.slice(0, 8)}
                  </span>
                </div>
                <div className="world-meta">
                  <span>{p.techPoints} tech pts</span>
                  <span>{p.bossTechPoints} boss tech pts</span>
                  <span>{p.mutationCount} mutations</span>
                  <span>{p.awakeningCount} awakenings</span>
                </div>
              </li>
            ))}
          </ul>
        </details>
      )}

      {playerFlags && playerFlags.length > 0 && (
        <details className="others">
          <summary>Tech, bosses & quests</summary>
          <ul className="worlds">
            {playerFlags.map((f) => (
              <li key={f.playerUid} className="world" style={{ alignItems: "flex-start" }}>
                <div className="world-main">
                  <span className="world-id" title={f.playerUid}>
                    {f.playerUid.slice(0, 8)}
                  </span>
                </div>
                <div className="world-meta" style={{ flexDirection: "column", alignItems: "flex-start", gap: "0.25rem" }}>
                  <span>{f.unlockedTech.length} technologies unlocked</span>
                  <span>
                    {f.normalBossDefeated.length} normal · {f.towerBossDefeated.length} tower ·{" "}
                    {f.specificBossDefeated.length} named bosses defeated
                  </span>
                  <span>{f.completedQuests.length} quests completed</span>
                  <span>
                    {f.relicsObtained.length} relics · {f.notesObtained.length} notes ·{" "}
                    {f.fastTravelUnlocked.length} fast travel points
                  </span>
                </div>
              </li>
            ))}
          </ul>
        </details>
      )}

      <input
        className="filter"
        type="text"
        placeholder="Filter by species or nickname…"
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
      />

      {pals === null && !error && <p className="muted">Loading roster…</p>}

      {pals !== null && (
        <div className="roster-table-wrap">
          <table className="roster-table">
            <thead>
              <tr>
                <SortableHeader column="characterId" sort={sort} onToggle={toggleSort}>
                  Species
                </SortableHeader>
                <th>Nickname</th>
                <SortableHeader column="level" sort={sort} onToggle={toggleSort}>
                  Lv
                </SortableHeader>
                <SortableHeader column="rank" sort={sort} onToggle={toggleSort}>
                  Rank
                </SortableHeader>
                <th>Gender</th>
                <SortableHeader column="ivAvg" sort={sort} onToggle={toggleSort}>
                  IVs (HP/Shot/Def)
                </SortableHeader>
                <th>Passives</th>
                <th>Location</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((pal) => (
                <tr key={pal.instanceId}>
                  <td className="species">{pal.characterId}</td>
                  <td className="muted">{pal.nickname ?? ""}</td>
                  <td>{pal.level}</td>
                  <td>{pal.rank}</td>
                  <td className="muted">{pal.gender}</td>
                  <td className="ivs">
                    {pal.ivHp}/{pal.ivShot}/{pal.ivDefense}
                  </td>
                  <td className="muted passives">{pal.passives.join(", ")}</td>
                  <td className="muted">{locationLabel(pal.locationKind)}</td>
                  <td>
                    {pal.isLucky && <span className="badge badge-localWorld">Lucky</span>}
                    {pal.isBoss && <span className="badge badge-unknown">Boss</span>}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {filtered.length === 0 && (
            <p className="muted" style={{ padding: "1rem" }}>
              No pals match “{filter}”.
            </p>
          )}
        </div>
      )}
    </div>
  );
}

function SortableHeader({
  column,
  sort,
  onToggle,
  children,
}: {
  column: SortColumn;
  sort: Sort;
  onToggle: (c: SortColumn) => void;
  children: React.ReactNode;
}) {
  const active = sort.column === column;
  return (
    <th
      className={active ? "sortable sortable-active" : "sortable"}
      onClick={() => onToggle(column)}
    >
      {children}
      {active && (sort.direction === "asc" ? " ▲" : " ▼")}
    </th>
  );
}

function ivAverage(pal: PalView): number {
  return (pal.ivHp + pal.ivShot + pal.ivDefense) / 3;
}

function locationLabel(kind: PalView["locationKind"]): string {
  switch (kind) {
    case "party":
      return "Party";
    case "box":
      return "Box";
    case "other":
      return "Base / other";
    default:
      return "Unknown";
  }
}
