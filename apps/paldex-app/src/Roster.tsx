import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type {
  DexProgressView,
  PalView,
  PlayerFlagsView,
  PlayerProgressView,
  SnapshotSummaryView,
} from "./types";
import { relativeTime } from "./time";

/// Fixed row height, in px, matching `.roster-table tbody tr` in styles.css.
/// Windowed rendering needs to know it without measuring.
const ROW_HEIGHT = 48;
/// Rows rendered beyond the viewport on each side, so a fast scroll doesn't
/// expose blank space before React catches up.
const OVERSCAN = 10;

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
  const [icons, setIcons] = useState<Record<string, string>>({});
  const iconUrlsRef = useRef<string[]>([]);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [resyncing, setResyncing] = useState(false);
  const [filter, setFilter] = useState("");
  const [sort, setSort] = useState<Sort>({ column: "level", direction: "desc" });

  const load = useCallback(async () => {
    setError(null);
    try {
      console.info("[paldex] roster: requesting data");
      const [palsResult, dexResult, playersResult, flagsResult] = await Promise.all([
        invoke<PalView[]>("pal_roster"),
        invoke<DexProgressView>("dex_progress"),
        invoke<PlayerProgressView[]>("player_progress"),
        invoke<PlayerFlagsView[]>("player_flags_detail"),
      ]);
      console.info(`[paldex] roster: got ${palsResult.length} pals`);
      setPals(palsResult);
      setDex(dexResult);
      setPlayers(playersResult);
      setPlayerFlags(flagsResult);

      // One batched call for the few hundred distinct species on screen —
      // per-row requests would be hundreds of IPC round trips. Artwork is
      // optional, so a failure here must not blank the roster.
      const species = [...new Set(palsResult.map((p) => p.characterId))];
      try {
        console.info(`[paldex] roster: requesting ${species.length} icons`);
        const dataUrls = await invoke<Record<string, string>>("pal_icons", {
          characterIds: species,
        });
        console.info(`[paldex] roster: received ${Object.keys(dataUrls).length} icons`);
        // Convert to blob URLs before they reach the DOM. A data URL is ~22 KB
        // of base64, and the same species repeats across many rows, so putting
        // them in `src` directly costs tens of MB of attribute text and defeats
        // the webview's per-URL image cache.
        const blobUrls: Record<string, string> = {};
        await Promise.all(
          Object.entries(dataUrls).map(async ([id, dataUrl]) => {
            blobUrls[id] = URL.createObjectURL(await (await fetch(dataUrl)).blob());
          }),
        );
        iconUrlsRef.current.forEach((url) => URL.revokeObjectURL(url));
        iconUrlsRef.current = Object.values(blobUrls);
        console.info("[paldex] roster: icons ready");
        setIcons(blobUrls);
      } catch (e) {
        console.warn("icons unavailable:", e);
      }
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, summary.snapshotId]);

  useEffect(
    () => () => {
      iconUrlsRef.current.forEach((url) => URL.revokeObjectURL(url));
      iconUrlsRef.current = [];
    },
    [],
  );

  // Track the scroll container so only visible rows are rendered.
  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const sync = () => {
      setScrollTop(el.scrollTop);
      setViewportHeight(el.clientHeight);
    };
    sync();
    el.addEventListener("scroll", sync, { passive: true });
    window.addEventListener("resize", sync);
    return () => {
      el.removeEventListener("scroll", sync);
      window.removeEventListener("resize", sync);
    };
  }, [pals]);

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
            speciesLabel(p).toLowerCase().includes(needle) ||
            p.characterId.toLowerCase().includes(needle) ||
            passiveLabels(p).some((n) => n.toLowerCase().includes(needle)) ||
            (p.nickname?.toLowerCase().includes(needle) ?? false),
        )
      : pals;
    const sorted = [...matches].sort((a, b) => {
      const dir = sort.direction === "asc" ? 1 : -1;
      switch (sort.column) {
        case "characterId":
          return dir * speciesLabel(a).localeCompare(speciesLabel(b));
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

  // Only the rows on screen are rendered. A full roster is ~2000 rows, each
  // now carrying an icon; rendering all of them is what blanked the window.
  const visible = useMemo(() => {
    const total = filtered.length;
    const height = viewportHeight || 600;
    const start = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
    const end = Math.min(total, Math.ceil((scrollTop + height) / ROW_HEIGHT) + OVERSCAN);
    return {
      rows: filtered.slice(start, end),
      padTop: start * ROW_HEIGHT,
      padBottom: (total - end) * ROW_HEIGHT,
    };
  }, [filtered, scrollTop, viewportHeight]);

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
                  <span title={techLabels(f).join(", ")}>
                    {f.unlockedTech.length} technologies unlocked
                  </span>
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
        <div className="roster-table-wrap" ref={wrapRef}>
          <table className="roster-table">
            <thead>
              <tr>
                <th className="icon-col" aria-label="Icon"></th>
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
              {visible.padTop > 0 && <tr aria-hidden="true" style={{ height: visible.padTop }} />}
              {visible.rows.map((pal) => (
                <tr key={pal.instanceId}>
                  <td className="icon-col">
                    {icons[pal.characterId] && (
                      <img
                        className="pal-icon"
                        src={icons[pal.characterId]}
                        alt=""
                        loading="lazy"
                        width={32}
                        height={32}
                      />
                    )}
                  </td>
                  <td className="species" title={pal.characterId}>
                    {speciesLabel(pal)}
                  </td>
                  <td className="muted">{pal.nickname ?? ""}</td>
                  <td>{pal.level}</td>
                  <td>{pal.rank}</td>
                  <td className="muted">{pal.gender}</td>
                  <td className="ivs">
                    {pal.ivHp}/{pal.ivShot}/{pal.ivDefense}
                  </td>
                  <td className="muted passives" title={pal.passives.join(", ")}>
                    {passiveLabels(pal).join(", ")}
                  </td>
                  <td className="muted">{locationLabel(pal.locationKind)}</td>
                  <td>
                    {pal.isLucky && <span className="badge badge-localWorld">Lucky</span>}
                    {pal.isBoss && <span className="badge badge-unknown">Boss</span>}
                  </td>
                </tr>
              ))}
              {visible.padBottom > 0 && (
                <tr aria-hidden="true" style={{ height: visible.padBottom }} />
              )}
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

/** Display name when the game pak supplied one, else the raw internal id. */
function speciesLabel(pal: PalView): string {
  return pal.displayName ?? pal.characterId;
}

/**
 * Localized passive names, falling back to raw ids when no pak is available
 * (in which case `passiveNames` is empty rather than parallel).
 */
function passiveLabels(pal: PalView): string[] {
  return pal.passiveNames.length === pal.passives.length ? pal.passiveNames : pal.passives;
}

/**
 * Localized technology names, falling back to raw ids when no pak is
 * available (in which case `unlockedTechNames` is empty rather than parallel).
 */
function techLabels(flags: PlayerFlagsView): string[] {
  return flags.unlockedTechNames.length === flags.unlockedTech.length
    ? flags.unlockedTechNames
    : flags.unlockedTech;
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
