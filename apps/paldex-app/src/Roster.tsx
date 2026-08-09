import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type {
  PalView,
  PlayerFlagsView,
  PlayerProgressView,
  SnapshotSummaryView,
} from "./types";
import { useSpeciesIcons } from "./icons";
import { elementColor } from "./elements";

/// Fixed row height, in px, matching `.roster-table tbody tr` in styles.css.
/// Windowed rendering needs to know it without measuring.
const ROW_HEIGHT = 48;
/// Rows rendered beyond the viewport on each side, so a fast scroll doesn't
/// expose blank space before React catches up.
const OVERSCAN = 10;

/// Sentinel for "don't filter by element" in the element picker.
const ANY_ELEMENT = "";

type Sort = { column: SortColumn; direction: "asc" | "desc" };
type SortColumn = "characterId" | "level" | "ivAvg" | "rank" | "rarity";

interface Props {
  summary: SnapshotSummaryView;
}

export default function Roster({ summary }: Props) {
  const [pals, setPals] = useState<PalView[] | null>(null);
  const [players, setPlayers] = useState<PlayerProgressView[] | null>(null);
  const [playerFlags, setPlayerFlags] = useState<PlayerFlagsView[] | null>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const [scrollTop, setScrollTop] = useState(0);
  const [viewportHeight, setViewportHeight] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [element, setElement] = useState(ANY_ELEMENT);
  const [sort, setSort] = useState<Sort>({ column: "level", direction: "desc" });

  const load = useCallback(async () => {
    setError(null);
    try {
      console.info("[paldex] roster: requesting data");
      const [palsResult, playersResult, flagsResult] = await Promise.all([
        invoke<PalView[]>("pal_roster"),
        invoke<PlayerProgressView[]>("player_progress"),
        invoke<PlayerFlagsView[]>("player_flags_detail"),
      ]);
      console.info(`[paldex] roster: got ${palsResult.length} pals`);
      setPals(palsResult);
      setPlayers(playersResult);
      setPlayerFlags(flagsResult);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, summary.snapshotId]);

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

  // Elements come from the data rather than a fixed list, so a species with a
  // new element in a future patch stays filterable.
  const elementOptions = useMemo(() => {
    const seen = new Map<string, string>();
    for (const pal of pals ?? []) {
      for (const el of pal.elements) if (!seen.has(el.id)) seen.set(el.id, el.name);
    }
    return [...seen].map(([id, name]) => ({ id, name })).sort((a, b) => a.name.localeCompare(b.name));
  }, [pals]);

  const filtered = useMemo(() => {
    if (!pals) return [];
    const needle = filter.trim().toLowerCase();
    const matches = pals.filter((p) => {
      if (element !== ANY_ELEMENT && !p.elements.some((el) => el.id === element)) return false;
      if (!needle) return true;
      return (
        speciesLabel(p).toLowerCase().includes(needle) ||
        p.characterId.toLowerCase().includes(needle) ||
        p.elements.some((el) => el.name.toLowerCase().includes(needle)) ||
        passiveLabels(p).some((n) => n.toLowerCase().includes(needle)) ||
        (p.nickname?.toLowerCase().includes(needle) ?? false)
      );
    });
    const sorted = [...matches].sort((a, b) => {
      const dir = sort.direction === "asc" ? 1 : -1;
      switch (sort.column) {
        case "characterId":
          return dir * speciesLabel(a).localeCompare(speciesLabel(b));
        case "level":
          return dir * (a.level - b.level);
        case "rank":
          return dir * (a.rank - b.rank);
        case "rarity":
          return dir * (a.rarity - b.rarity);
        case "ivAvg":
          return dir * (ivAverage(a) - ivAverage(b));
      }
    });
    return sorted;
  }, [pals, filter, element, sort]);

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

  // Artwork for the species actually rendered. The windowed row list changes
  // as the user scrolls, so this keys off the whole filtered set rather than
  // the visible slice — otherwise every scroll would refetch.
  const speciesOnScreen = useMemo(
    () => [...new Set(filtered.map((p) => p.characterId))],
    [filtered],
  );
  const icons = useSpeciesIcons(speciesOnScreen);

  const toggleSort = (column: SortColumn) => {
    setSort((prev) =>
      prev.column === column
        ? { column, direction: prev.direction === "asc" ? "desc" : "asc" }
        : { column, direction: "desc" },
    );
  };

  return (
    <>
      {error && <p className="notice">{error}</p>}

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

      <div className="roster-controls">
        <input
          className="filter"
          type="text"
          placeholder="Filter by species, element, passive or nickname…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        {elementOptions.length > 0 && (
          <select
            className="picker"
            value={element}
            onChange={(e) => setElement(e.target.value)}
            aria-label="Filter by element"
          >
            <option value={ANY_ELEMENT}>Any element</option>
            {elementOptions.map((el) => (
              <option key={el.id} value={el.id}>
                {el.name}
              </option>
            ))}
          </select>
        )}
      </div>

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
                <th>Element</th>
                <th>Nickname</th>
                <SortableHeader column="level" sort={sort} onToggle={toggleSort}>
                  Lv
                </SortableHeader>
                <SortableHeader column="rank" sort={sort} onToggle={toggleSort}>
                  Rank
                </SortableHeader>
                <SortableHeader column="rarity" sort={sort} onToggle={toggleSort}>
                  Rarity
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
                  <td className="element-col">
                    {pal.elements.map((el) => (
                      <span
                        key={el.id}
                        className="element"
                        style={{ color: elementColor(el.id), borderColor: elementColor(el.id) }}
                      >
                        {el.name}
                      </span>
                    ))}
                  </td>
                  <td className="muted">{pal.nickname ?? ""}</td>
                  <td>{pal.level}</td>
                  <td>{pal.rank}</td>
                  <td className="muted">{pal.rarity > 0 ? pal.rarity : ""}</td>
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
    </>
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
