import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type {
  BreedingPairView,
  BreedingParentView,
  DexProgressView,
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

/** Pairs shown at once. A common target has several hundred. */
const PAIR_PAGE = 25;

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
}

/**
 * The derived-insight screen: IV grades, which specimen of each species to
 * keep, which duplicates to condense, and which owned pairs breed a chosen
 * species.
 *
 * Every number shown is an input to a recommendation rather than a verdict —
 * the plan asks for suggestions the player can check, so the composite score,
 * the tier it falls in, and the passives a pairing would draw from are all on
 * screen next to the suggestion they produced.
 */
export default function Analysis({ summary }: Props) {
  const [quality, setQuality] = useState<PalQualityView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [list, setList] = useState<List>("graded");
  const [filter, setFilter] = useState("");
  const [limit, setLimit] = useState(PAGE);

  const load = useCallback(async () => {
    setError(null);
    try {
      setQuality(await invoke<PalQualityView>("pal_quality"));
    } catch (e) {
      setError(String(e));
    }
  }, []);

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

      <Breeding summary={summary} />

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

/**
 * The inverse breeding search: pick a species, get the owned pairs that
 * produce it, best first.
 *
 * Split from the grading above because it re-queries on every target change
 * while the grading is fetched once per snapshot.
 */
function Breeding({ summary }: { summary: SnapshotSummaryView }) {
  const [targets, setTargets] = useState<{ id: string; label: string }[]>([]);
  const [target, setTarget] = useState("");
  const [pairs, setPairs] = useState<BreedingPairView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [limit, setLimit] = useState(PAIR_PAGE);

  // The species list is the Paldeck, not the roster — the whole point is to
  // ask for something you do not have yet.
  useEffect(() => {
    void (async () => {
      try {
        const dex = await invoke<DexProgressView>("dex_progress");
        setTargets(
          dex.entries.map((e) => ({
            id: e.characterId,
            label: e.dexLabel ? `No.${e.dexLabel} ${e.displayName}` : e.displayName,
          })),
        );
      } catch (e) {
        console.warn("breeding targets unavailable:", e);
      }
    })();
  }, [summary.snapshotId]);

  useEffect(() => {
    if (!target) {
      setPairs(null);
      return;
    }
    let cancelled = false;
    setBusy(true);
    setError(null);
    setLimit(PAIR_PAGE);
    void (async () => {
      try {
        const result = await invoke<BreedingPairView[]>("breeding_options", { target });
        if (!cancelled) setPairs(result);
      } catch (e) {
        if (!cancelled) setError(String(e));
      } finally {
        if (!cancelled) setBusy(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [target, summary.snapshotId]);

  const shown = useMemo(() => pairs?.slice(0, limit) ?? [], [pairs, limit]);

  // Distinct parent species across every pair, for the same reason as the
  // graded table above: paging must not change what artwork is requested.
  const speciesOnScreen = useMemo(
    () => [
      ...new Set(
        (pairs ?? []).flatMap((p) => [p.parentA.characterId, p.parentB.characterId]),
      ),
    ],
    [pairs],
  );
  const icons = useSpeciesIcons(speciesOnScreen);

  return (
    <div className="breeding">
      <div className="breeding-head">
        <h2>Breed for a species</h2>
        <select
          className="picker"
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          aria-label="Target species"
        >
          <option value="">Pick a species…</option>
          {targets.map((t) => (
            <option key={t.id} value={t.id}>
              {t.label}
            </option>
          ))}
        </select>
      </div>

      {error && <p className="notice">{error}</p>}
      {busy && <p className="muted">Searching owned pairs…</p>}

      {!busy && pairs !== null && pairs.length === 0 && (
        <p className="muted">
          Nothing you own breeds into that species. Every pair here comes from
          Pals in this world, so a target with no result needs a parent you do
          not have yet.
        </p>
      )}

      {!busy && pairs !== null && pairs.length > 0 && (
        <>
          <p className="muted dex-count">
            {pairs.length} owned pair{pairs.length === 1 ? "" : "s"} produce this species —
            showing the best {shown.length}
          </p>
          <ul className="pairs">
            {shown.map((pair) => (
              <li key={`${pair.parentA.instanceId}-${pair.parentB.instanceId}`} className="pair">
                <Parent parent={pair.parentA} icon={icons[pair.parentA.characterId]} />
                <span className="pair-plus" aria-hidden="true">
                  +
                </span>
                <Parent parent={pair.parentB} icon={icons[pair.parentB.characterId]} />
                <div className="pair-why">
                  <span className="pair-score" title="Parent IV average plus 5 per inheritable passive">
                    {pair.score.toFixed(1)}
                  </span>
                  <span className="muted">
                    {pair.parentIvAverage.toFixed(1)} avg IVs
                    {pair.inheritedPassives.length > 0 && (
                      <> · {pair.inheritedPassives.join(", ")}</>
                    )}
                  </span>
                </div>
              </li>
            ))}
          </ul>
          {pairs.length > shown.length && (
            <button
              className="btn btn-ghost show-more"
              onClick={() => setLimit((n) => n + PAIR_PAGE)}
            >
              Show {Math.min(PAIR_PAGE, pairs.length - shown.length)} more
            </button>
          )}
          <p className="muted dex-detail-note">
            One pair per species combination, using the best specimens you own
            of each. Ranked by the parents’ average IVs plus the passives the
            child could inherit — a child takes at most four, so a larger pool
            stops helping past that.
          </p>
        </>
      )}
    </div>
  );
}

function Parent({ parent, icon }: { parent: BreedingParentView; icon?: string }) {
  return (
    <div className="pair-parent">
      {icon && <img className="pal-icon" src={icon} alt="" loading="lazy" width={32} height={32} />}
      <div className="pair-parent-text">
        <span className="species" title={parent.characterId}>
          {parent.displayName ?? parent.characterId}
          {parent.nickname && <span className="muted"> “{parent.nickname}”</span>}
        </span>
        <span className="muted">
          Lv {parent.level} · {parent.gender} · {parent.ivHp}/{parent.ivShot}/{parent.ivDefense}
        </span>
      </div>
    </div>
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
