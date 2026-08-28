import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { BaseCampView, GradedPalView, SnapshotSummaryView } from "./types";
import { condenseStars } from "./types";
import { useSpeciesIcons } from "./icons";

interface Props {
  summary: SnapshotSummaryView;
}

/**
 * Which Pals work at which base camp.
 *
 * World-scoped on purpose, and that is why this takes no `playerUid`: a base
 * belongs to the guild, not to a player, so the player selector does not apply
 * — the same reasoning that makes base Pals orthogonal to `PlayerScope` in the
 * query layer. Omitting the prop is what makes that visible at the call site.
 *
 * Bases are identified by number rather than name because the save has no
 * usable name to show: every base decodes to the same untranslated placeholder
 * (see `paldex-model`'s `rawdata::base_camp`), and Palworld gives no way to
 * name one. Level, HP and rank are absent for the same reason — nothing in the
 * decoded bytes distinguishes one base from another.
 */
export default function Bases({ summary }: Props) {
  const [bases, setBases] = useState<BaseCampView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());

  const load = useCallback(async () => {
    setError(null);
    try {
      setBases(await invoke<BaseCampView[]>("base_summary"));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, summary.snapshotId]);

  const toggle = useCallback((id: string) => {
    setExpanded((open) => {
      const next = new Set(open);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  }, []);

  // Artwork for every base's workers at once, not just the expanded ones:
  // requesting per expansion re-fetched on every click, and the whole set is
  // at most a few dozen species.
  const speciesOnScreen = useMemo(
    () => [...new Set((bases ?? []).flatMap((b) => b.workers.map((w) => w.characterId)))],
    [bases],
  );
  const icons = useSpeciesIcons(speciesOnScreen);

  const workerTotal = useMemo(
    () => (bases ?? []).reduce((sum, b) => sum + b.workerCount, 0),
    [bases],
  );

  if (error) return <p className="notice">{error}</p>;
  if (!bases) return <p className="muted">Reading base camps…</p>;
  if (bases.length === 0) {
    return <p className="muted">This world has no base camps.</p>;
  }

  return (
    <section className="bases">
      <div className="dex-stats">
        <Stat label="Bases" value={bases.length} />
        <Stat
          label="Pals working"
          value={workerTotal}
          hint="Assigned to a base camp rather than carried or boxed"
        />
      </div>

      <ul className="base-list">
        {bases.map((base) => {
          const open = expanded.has(base.id);
          return (
            <li key={base.id} className="base-item">
              <button
                className="base-head"
                onClick={() => toggle(base.id)}
                aria-expanded={open}
                title={base.id}
              >
                <span className="base-caret" aria-hidden="true">
                  {open ? "▾" : "▸"}
                </span>
                <span className="base-name">Base {base.number}</span>
                <span className="muted base-count">
                  {base.workerCount} {base.workerCount === 1 ? "worker" : "workers"}
                </span>
              </button>

              {open &&
                (base.workers.length === 0 ? (
                  <p className="muted base-empty">No workers assigned.</p>
                ) : (
                  <WorkerTable workers={base.workers} icons={icons} />
                ))}
            </li>
          );
        })}
      </ul>

      <p className="muted dex-detail-note">
        Bases are numbered rather than named because the save stores no name for
        them — every base carries the same untranslated placeholder, and the
        game gives no way to change it. The numbering is by internal id, so it
        stays put between syncs. This tab is world-wide: bases belong to the
        guild, so the player selector does not apply to it.
      </p>
    </section>
  );
}

function WorkerTable({
  workers,
  icons,
}: {
  workers: GradedPalView[];
  icons: Record<string, string>;
}) {
  return (
    <div className="roster-table-wrap base-workers">
      <table className="roster-table">
        <thead>
          <tr>
            <th className="icon-col" aria-label="Icon"></th>
            <th>Species</th>
            <th>Nickname</th>
            <th>Lv</th>
            <th>Stars</th>
            <th>Gender</th>
            <th>IVs (HP/Shot/Def)</th>
            <th>Score</th>
            <th>Passives</th>
          </tr>
        </thead>
        <tbody>
          {workers.map((pal) => (
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
                {pal.displayName ?? pal.characterId}
              </td>
              <td className="muted">{pal.nickname ?? ""}</td>
              <td>{pal.level}</td>
              <td>{condenseStars(pal.rank)}</td>
              <td className="muted">{pal.gender}</td>
              <td className="ivs">
                {pal.ivHp}/{pal.ivShot}/{pal.ivDefense}
              </td>
              <td>{pal.composite.toFixed(1)}</td>
              <td className="muted passives">{pal.passiveNames.join(", ")}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Stat({ label, value, hint }: { label: string; value: number | string; hint?: string }) {
  return (
    <div className="stat" title={hint}>
      <span className="stat-value">{value}</span>
      <span className="stat-label">{label}</span>
    </div>
  );
}
