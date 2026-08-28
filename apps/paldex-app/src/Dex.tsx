import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type {
  BaseStatsView,
  DexEntryView,
  DexProgressView,
  ElementView,
  SnapshotSummaryView,
} from "./types";
import { CAPTURE_BONUS_AT } from "./types";
import { useSpeciesIcons } from "./icons";
import { elementColor } from "./elements";

/** Sentinel for "don't filter by element" in the element picker. */
const ANY_ELEMENT = "";

type Show = "all" | "caught" | "missing";

interface Props {
  summary: SnapshotSummaryView;
  /** Whose dex to show; `null` is every player in the world. */
  playerUid: string | null;
}

export default function Dex({ summary, playerUid }: Props) {
  const [dex, setDex] = useState<DexProgressView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [show, setShow] = useState<Show>("all");
  const [filter, setFilter] = useState("");
  const [element, setElement] = useState(ANY_ELEMENT);
  const [selected, setSelected] = useState<string | null>(null);

  const load = useCallback(async () => {
    setError(null);
    try {
      setDex(await invoke<DexProgressView>("dex_progress", { playerUid }));
    } catch (e) {
      setError(String(e));
    }
  }, [playerUid]);

  useEffect(() => {
    void load();
  }, [load, summary.snapshotId]);

  const entries = useMemo(() => dex?.entries ?? [], [dex]);

  // The element list comes from the data rather than a hard-coded set, so a
  // species with a new element in a future patch is still filterable.
  const elementOptions = useMemo(() => {
    const seen = new Map<string, string>();
    for (const entry of entries) {
      for (const el of entry.elements) if (!seen.has(el.id)) seen.set(el.id, el.name);
    }
    return [...seen].map(([id, name]) => ({ id, name }));
  }, [entries]);

  const visible = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    return entries.filter((e) => {
      if (show === "caught" && !e.caught) return false;
      if (show === "missing" && e.caught) return false;
      if (element !== ANY_ELEMENT && !e.elements.some((el) => el.id === element)) return false;
      if (!needle) return true;
      return (
        e.displayName.toLowerCase().includes(needle) ||
        e.elements.some((el) => el.name.toLowerCase().includes(needle)) ||
        e.workSuitabilities.some((w) => w.name.toLowerCase().includes(needle)) ||
        (e.dexLabel?.toLowerCase().includes(needle) ?? false)
      );
    });
  }, [entries, show, filter, element]);

  // Only the species actually on screen are worth decoding artwork for.
  const speciesOnScreen = useMemo(() => visible.map((e) => e.characterId), [visible]);
  const icons = useSpeciesIcons(speciesOnScreen);

  // Base stats are authored multipliers with no absolute meaning, so the detail
  // panel shows each one against the highest in the Paldeck rather than as a
  // bare number the player has nothing to compare to.
  const statCeilings = useMemo(() => peakStats(entries), [entries]);

  const caught = dex?.unlockedSpeciesCount ?? 0;
  const total = dex?.totalSpeciesCount ?? 0;
  const bonusComplete = useMemo(
    () => entries.filter((e) => isBonusComplete(e)).length,
    [entries],
  );

  const detail = useMemo(
    () => entries.find((e) => e.characterId === selected) ?? null,
    [entries, selected],
  );

  if (error) return <p className="notice">{error}</p>;
  if (!dex) return <p className="muted">Loading dex…</p>;

  return (
    <section className="dex">
      <div className="dex-head">
        <div className="dex-stats">
          <Stat label="Caught" value={caught} />
          <Stat label="Paldeck entries" value={total > 0 ? total : "—"} />
          <Stat
            label="Capture bonus"
            value={bonusComplete}
            hint={`species with ${CAPTURE_BONUS_AT}+ captures`}
          />
          {total > 0 && (
            <Stat label="Completion" value={`${Math.round((caught / total) * 100)}%`} />
          )}
        </div>

        <div className="dex-controls">
          <input
            className="filter dex-filter"
            placeholder="Filter by name, number, element or job…"
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
          <div className="seg">
            {(["all", "caught", "missing"] as const).map((option) => (
              <button
                key={option}
                className={show === option ? "seg-btn seg-on" : "seg-btn"}
                onClick={() => setShow(option)}
              >
                {option === "all" ? "All" : option === "caught" ? "Caught" : "Missing"}
              </button>
            ))}
          </div>
        </div>
      </div>

      {total === 0 && (
        <p className="muted dex-caveat">
          Palworld isn’t installed where Paldex can see it, so only species you
          have caught can be listed — there is no species list to compare
          against, and no artwork.
        </p>
      )}

      {detail && (
        <Detail
          entry={detail}
          icon={icons[detail.characterId]}
          ceilings={statCeilings}
          onClose={() => setSelected(null)}
        />
      )}

      <p className="muted dex-count">
        Showing {visible.length} of {entries.length}
      </p>

      <ul className="dex-grid">
        {visible.map((entry) => (
          <Tile
            key={entry.characterId}
            entry={entry}
            icon={icons[entry.characterId]}
            selected={entry.characterId === selected}
            onSelect={() =>
              setSelected((prev) => (prev === entry.characterId ? null : entry.characterId))
            }
          />
        ))}
      </ul>

      {visible.length === 0 && <p className="muted">No species match that filter.</p>}
    </section>
  );
}

/**
 * How far this species is through its capture bonus, from 0 to
 * `CAPTURE_BONUS_AT`.
 *
 * The save records the tier directly, so trust it and fall back to the count
 * only for a snapshot ingested before tiers were stored. Capping the fallback
 * is what keeps the two answers agreeing: the stored tier is exactly
 * `min(captureCount, CAPTURE_BONUS_AT)` in every observed save, so the
 * fallback has to be capped the same way or a species caught many times would
 * read as an impossible tier.
 */
function bonusTier(entry: DexEntryView): number {
  return Math.min(Math.max(entry.bonusTier, entry.captureCount), CAPTURE_BONUS_AT);
}

function isBonusComplete(entry: DexEntryView): boolean {
  return bonusTier(entry) >= CAPTURE_BONUS_AT;
}

/** The highest value seen for each base stat, used to scale the detail bars. */
function peakStats(entries: DexEntryView[]): BaseStatsView {
  const peak: BaseStatsView = {
    hp: 1,
    meleeAttack: 1,
    shotAttack: 1,
    defense: 1,
    support: 1,
    craftSpeed: 1,
  };
  for (const entry of entries) {
    if (!entry.stats) continue;
    for (const key of Object.keys(peak) as (keyof BaseStatsView)[]) {
      peak[key] = Math.max(peak[key], entry.stats[key]);
    }
  }
  return peak;
}

const STAT_LABELS: [keyof BaseStatsView, string][] = [
  ["hp", "HP"],
  ["meleeAttack", "Melee"],
  ["shotAttack", "Ranged"],
  ["defense", "Defense"],
  ["support", "Support"],
  ["craftSpeed", "Work speed"],
];

function Detail({
  entry,
  icon,
  ceilings,
  onClose,
}: {
  entry: DexEntryView;
  icon?: string;
  ceilings: BaseStatsView;
  onClose: () => void;
}) {
  const stats = entry.stats;
  return (
    <aside className="dex-detail">
      <div className="dex-detail-head">
        <div className="dex-detail-art">
          {icon ? (
            <img src={icon} alt="" width={72} height={72} />
          ) : (
            <span className="dex-noart" aria-hidden="true">
              ?
            </span>
          )}
        </div>
        <div className="dex-detail-title">
          <h2>
            {entry.dexLabel && <span className="dex-detail-num">No.{entry.dexLabel}</span>}
            {entry.displayName}
          </h2>
          <Elements elements={entry.elements} />
          <p className="muted dex-detail-sub">
            {entry.rarity > 0 && <>Rarity {entry.rarity} · </>}
            {entry.caught
              ? `${entry.captureCount} capture${entry.captureCount === 1 ? "" : "s"}`
              : "Not caught"}
            {isBonusComplete(entry)
              ? " · capture bonus complete"
              : bonusTier(entry) > 0 &&
                ` · capture bonus ${bonusTier(entry)}/${CAPTURE_BONUS_AT}`}
          </p>
        </div>
        <button className="btn btn-ghost" onClick={onClose}>
          Close
        </button>
      </div>

      <div className="dex-detail-body">
        <div className="dex-detail-block">
          <h3>Base stats</h3>
          {stats ? (
            <ul className="statbars">
              {STAT_LABELS.map(([key, label]) => (
                <li key={key}>
                  <span className="statbar-label">{label}</span>
                  <span className="statbar-track">
                    <span
                      className="statbar-fill"
                      style={{ width: `${Math.round((stats[key] / ceilings[key]) * 100)}%` }}
                    />
                  </span>
                  <span className="statbar-value">{stats[key]}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted">No base stats without the game installed.</p>
          )}
          <p className="muted dex-detail-note">
            Authored per-species values, shown against the highest in the
            Paldeck. The game combines these with level, IVs and souls, so they
            are not the numbers on a Pal’s status screen.
          </p>
        </div>

        <div className="dex-detail-block">
          <h3>Work suitability</h3>
          {entry.workSuitabilities.length > 0 ? (
            <ul className="jobs">
              {entry.workSuitabilities.map((job) => (
                <li key={job.id} className="job">
                  <span className="job-name">{job.name}</span>
                  <span className="job-level">{job.level}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted">This species cannot be assigned to base work.</p>
          )}
        </div>
      </div>
    </aside>
  );
}

function Elements({ elements }: { elements: ElementView[] }) {
  if (elements.length === 0) return null;
  return (
    <span className="elements">
      {elements.map((el) => (
        <span
          key={el.id}
          className="element"
          style={{ color: elementColor(el.id), borderColor: elementColor(el.id) }}
        >
          {el.name}
        </span>
      ))}
    </span>
  );
}

function Tile({
  entry,
  icon,
  selected,
  onSelect,
}: {
  entry: DexEntryView;
  icon?: string;
  selected: boolean;
  onSelect: () => void;
}) {
  const complete = isBonusComplete(entry);
  const tier = bonusTier(entry);
  const className = [
    "dex-tile",
    entry.caught ? "dex-caught" : "dex-unseen",
    complete ? "dex-bonus" : "",
    selected ? "dex-selected" : "",
  ]
    .filter(Boolean)
    .join(" ");

  const label = entry.dexLabel ? `No.${entry.dexLabel} ${entry.displayName}` : entry.displayName;
  const elementSuffix =
    entry.elements.length > 0 ? ` — ${entry.elements.map((e) => e.name).join("/")}` : "";
  const bonusSuffix = complete
    ? ", bonus complete"
    : tier > 0
      ? `, bonus ${tier}/${CAPTURE_BONUS_AT}`
      : "";
  const title = entry.caught
    ? `${label}${elementSuffix} — ${entry.captureCount} capture${
        entry.captureCount === 1 ? "" : "s"
      }${bonusSuffix}`
    : `${label}${elementSuffix} — not caught`;

  return (
    <li className={className}>
      <button className="dex-tile-btn" onClick={onSelect} title={title} aria-pressed={selected}>
        {entry.dexLabel && <span className="dex-num">{entry.dexLabel}</span>}
        <div className="dex-art">
          {icon ? (
            <img src={icon} alt="" loading="lazy" width={56} height={56} />
          ) : (
            <span className="dex-noart" aria-hidden="true">
              ?
            </span>
          )}
        </div>
        <span className="dex-name">{entry.displayName}</span>
        <span className="dex-dots" aria-hidden="true">
          {entry.elements.map((el) => (
            <span
              key={el.id}
              className="dex-dot"
              style={{ background: elementColor(el.id) }}
            />
          ))}
        </span>
        {/* Below the top tier the capture count and the tier are the same
            number, so showing both would read "×3 3/5". Only a complete
            species has a count worth stating on its own — a high capture count is
            interesting, three captures is just the tier again. */}
        <span className="dex-meta">
          {!entry.caught ? (
            <span className="dex-dash">—</span>
          ) : complete ? (
            <>
              {entry.captureCount > 0 && (
                <span className="dex-captures">×{entry.captureCount}</span>
              )}
              <span className="dex-star" title="Capture bonus complete">
                ★
              </span>
            </>
          ) : (
            tier > 0 && (
              <span className="dex-tier" title={`Capture bonus ${tier}/${CAPTURE_BONUS_AT}`}>
                {tier}/{CAPTURE_BONUS_AT}
              </span>
            )
          )}
        </span>
      </button>
    </li>
  );
}

function Stat({
  label,
  value,
  hint,
}: {
  label: string;
  value: number | string;
  hint?: string;
}) {
  return (
    <div className="stat" title={hint}>
      <span className="stat-value">{value}</span>
      <span className="stat-label">{label}</span>
    </div>
  );
}
