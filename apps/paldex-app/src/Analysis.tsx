import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type {
  GradedPalView,
  IvTier,
  PalQualityView,
  SnapshotSummaryView,
} from "./types";
import { useSpeciesIcons } from "./icons";

/**
 * Rows shown before the "show more" button. A full roster is ~2,000 Pals and
 * every list here is ranked, so the tail is the part nobody scrolls to — a
 * page size avoids the windowed-rendering machinery the roster table needs.
 */
const PAGE = 60;

/** Which of the four ranked lists is on screen. */
type List = "graded" | "best" | "condense" | "passives";

const LISTS: { id: List; label: string; hint: string }[] = [
  { id: "graded", label: "All", hint: "Every owned Pal, best IVs first" },
  { id: "best", label: "Best of species", hint: "The one to keep, per species" },
  { id: "condense", label: "Condense fodder", hint: "Duplicates worth feeding for souls" },
  { id: "passives", label: "By passives", hint: "Ranked by how many passives they carry" },
];

interface Props {
  summary: SnapshotSummaryView;
  /** Whose Pals to grade; `null` is every player. */
  playerUid: string | null;
}

/**
 * The derived-insight screen: IV grades, which specimen of each species to
 * keep, which duplicates to condense, and how owned Pals rank by passives.
 * The breeding search is its own tab — see `Breeding.tsx`.
 *
 * Every number shown is an input to a recommendation rather than a verdict —
 * the plan asks for suggestions the player can check, so the composite score,
 * the tier it falls in, and the passives a pairing would draw from are all on
 * screen next to the suggestion they produced.
 */
export default function Analysis({ summary, playerUid }: Props) {
  const [quality, setQuality] = useState<PalQualityView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [list, setList] = useState<List>("graded");
  const [filter, setFilter] = useState("");
  const [limit, setLimit] = useState(PAGE);
  const [includeBasePals, setIncludeBasePals] = useState(true);

  const load = useCallback(async () => {
    setError(null);
    try {
      setQuality(await invoke<PalQualityView>("pal_quality", { playerUid, includeBasePals }));
    } catch (e) {
      setError(String(e));
    }
  }, [playerUid, includeBasePals]);

  useEffect(() => {
    void load();
  }, [load, summary.snapshotId]);

  // Reset paging whenever the visible set changes out from under it, so
  // switching lists doesn't land the user 600 rows deep in a shorter one.
  useEffect(() => setLimit(PAGE), [list, filter]);

  const byId = useMemo(() => {
    const map = new Map<string, GradedPalView>();
    for (const pal of quality?.graded ?? []) map.set(pal.instanceId, pal);
    return map;
  }, [quality]);

  // The three derived lists are instance ids; `graded` is already the rows.
  const selected = useMemo(() => {
    if (!quality) return [];
    switch (list) {
      case "graded":
        return quality.graded;
      case "best":
        return resolve(quality.bestOfSpecies, byId);
      case "condense":
        return resolve(quality.condenseCandidates, byId);
      case "passives":
        return resolve(quality.passiveRanking, byId);
    }
  }, [quality, list, byId]);

  const visible = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (!needle) return selected;
    return selected.filter(
      (p) =>
        speciesLabel(p).toLowerCase().includes(needle) ||
        p.characterId.toLowerCase().includes(needle) ||
        (p.nickname?.toLowerCase().includes(needle) ?? false) ||
        p.passiveNames.some((n) => n.toLowerCase().includes(needle)),
    );
  }, [selected, filter]);

  const shown = useMemo(() => visible.slice(0, limit), [visible, limit]);

  // Distinct species across the whole filtered list, not the current page.
  // Keying off the page instead re-requested artwork on every "show more"
  // with an ever-longer id list, which is what made repeated paging crawl.
  const speciesOnScreen = useMemo(
    () => [...new Set(visible.map((p) => p.characterId))],
    [visible],
  );
  const icons = useSpeciesIcons(speciesOnScreen);

  const perfect = useMemo(
    () => (quality?.graded ?? []).filter((p) => p.tier === "Perfect").length,
    [quality],
  );

  if (error) return <p className="notice">{error}</p>;
  if (!quality) return <p className="muted">Grading roster…</p>;

  return (
    <section className="analysis">
      <div className="dex-stats">
        <Stat label="Graded" value={quality.graded.length} />
        <Stat
          label="Species owned"
          value={quality.bestOfSpecies.length}
          hint="Distinct species with at least one Pal"
        />
        <Stat
          label="Condense fodder"
          value={quality.condenseCandidates.length}
          hint="Duplicates, never counting the best of a species"
        />
        <Stat label="Perfect IVs" value={perfect} hint="100 across all three talents" />
      </div>

      <div className="dex-controls">
        <input
          className="filter dex-filter"
          placeholder="Filter by species, nickname or passive…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <div className="seg">
          {LISTS.map((option) => (
            <button
              key={option.id}
              className={list === option.id ? "seg-btn seg-on" : "seg-btn"}
              onClick={() => setList(option.id)}
              title={option.hint}
            >
              {option.label}
            </button>
          ))}
        </div>
        <label className="base-toggle">
          <input
            type="checkbox"
            checked={includeBasePals}
            onChange={(e) => setIncludeBasePals(e.target.checked)}
          />
          Include base pals
        </label>
      </div>

      <p className="muted dex-count">
        {LISTS.find((l) => l.id === list)?.hint} — showing {shown.length} of {visible.length}
      </p>

      <div className="roster-table-wrap">
        <table className="roster-table">
          <thead>
            <tr>
              <th className="icon-col" aria-label="Icon"></th>
              <th>Species</th>
              <th>Nickname</th>
              <th>Lv</th>
              <th>Rank</th>
              <th>Gender</th>
              <th>IVs (HP/Shot/Def)</th>
              <th>Score</th>
              <th>Tier</th>
              <th>Passives</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((pal) => (
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
                <td>{pal.composite.toFixed(1)}</td>
                <td>
                  <TierBadge tier={pal.tier} />
                </td>
                <td className="muted passives">{pal.passiveNames.join(", ")}</td>
              </tr>
            ))}
          </tbody>
        </table>

        {visible.length === 0 && (
          <p className="muted" style={{ padding: "1rem" }}>
            Nothing in this list{filter ? ` matches “${filter}”` : ""}.
          </p>
        )}
      </div>

      {visible.length > shown.length && (
        <button className="btn btn-ghost show-more" onClick={() => setLimit((n) => n + PAGE)}>
          Show {Math.min(PAGE, visible.length - shown.length)} more
        </button>
      )}

      <p className="muted dex-detail-note">
        Tiers are a judgment call on the composite (mean) IV score, not an
        in-game mechanic: 100 is Perfect, 90+ S, 80+ A, 70+ B, 60+ C. “By
        passives” ranks on passive <em>count</em> — the game states no ordering
        between one passive and another, so inventing a tier list would be a
        guess dressed as advice.
      </p>
    </section>
  );
}

function TierBadge({ tier }: { tier: IvTier }) {
  return <span className={`tier tier-${tier.toLowerCase()}`}>{tier}</span>;
}

/** Instance ids back to rows, skipping any the graded list somehow lacks. */
function resolve(ids: string[], byId: Map<string, GradedPalView>): GradedPalView[] {
  return ids.map((id) => byId.get(id)).filter((p): p is GradedPalView => p !== undefined);
}

/** Display name when the game pak supplied one, else the raw internal id. */
function speciesLabel(pal: GradedPalView): string {
  return pal.displayName ?? pal.characterId;
}

function Stat({ label, value, hint }: { label: string; value: number | string; hint?: string }) {
  return (
    <div className="stat" title={hint}>
      <span className="stat-value">{value}</span>
      <span className="stat-label">{label}</span>
    </div>
  );
}
