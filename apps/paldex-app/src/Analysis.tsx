import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type {
  BlockedPairingView,
  BreedingPairView,
  BreedingParentView,
  DexProgressView,
  GradedPalView,
  IvTier,
  PairingSideView,
  PalQualityView,
  SnapshotSummaryView,
  UnownedPairingView,
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

/** Which parent pool the breeding results are drawn from. */
type Pool = "owned" | "blocked" | "unowned";

const POOLS: { id: Pool; label: string; hint: string }[] = [
  {
    id: "owned",
    label: "Owned pairs",
    hint: "Both parents are Pals in this world, and you can breed them now",
  },
  {
    id: "blocked",
    label: "Blocked",
    hint: "You own both species but can't fill a farm with them",
  },
  {
    id: "unowned",
    label: "Unowned parents",
    hint: "Combinations needing at least one species you don't own",
  },
];

/**
 * The inverse breeding search: pick a species, get the combinations that
 * produce it.
 *
 * Two pools, because they answer different questions and cannot share a
 * ranking. "Owned pairs" names two specific Pals and ranks them by their IVs
 * and passives. "Unowned parents" is a *species* search — there is no
 * individual to rank for a species nobody owns, so it ranks by how many new
 * species the combination would cost.
 *
 * Split from the grading above because it re-queries on every target change
 * while the grading is fetched once per snapshot.
 */
function Breeding({ summary }: { summary: SnapshotSummaryView }) {
  const [targets, setTargets] = useState<{ id: string; label: string }[]>([]);
  const [target, setTarget] = useState("");
  const [pool, setPool] = useState<Pool>("owned");
  const [pairs, setPairs] = useState<BreedingPairView[] | null>(null);
  const [blocked, setBlocked] = useState<BlockedPairingView[] | null>(null);
  const [unowned, setUnowned] = useState<UnownedPairingView[] | null>(null);
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

  // Both pools are fetched for a target rather than one per tab, so the counts
  // on the tabs are honest before you click and switching costs no round trip.
  useEffect(() => {
    if (!target) {
      setPairs(null);
      setBlocked(null);
      setUnowned(null);
      return;
    }
    let cancelled = false;
    setBusy(true);
    setError(null);
    setLimit(PAIR_PAGE);
    void (async () => {
      try {
        const [owned, stuck, lacking] = await Promise.all([
          invoke<BreedingPairView[]>("breeding_options", { target }),
          invoke<BlockedPairingView[]>("blocked_breeding_options", { target }),
          invoke<UnownedPairingView[]>("unowned_breeding_options", { target }),
        ]);
        if (cancelled) return;
        setPairs(owned);
        setBlocked(stuck);
        setUnowned(lacking);
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

  // Paging is per pool, so switching tabs starts at the top of the new list.
  useEffect(() => setLimit(PAIR_PAGE), [pool]);

  const counts: Record<Pool, number | null> = {
    owned: pairs?.length ?? null,
    blocked: blocked?.length ?? null,
    unowned: unowned?.length ?? null,
  };

  const shownPairs = useMemo(() => pairs?.slice(0, limit) ?? [], [pairs, limit]);
  const shownBlocked = useMemo(() => blocked?.slice(0, limit) ?? [], [blocked, limit]);
  const shownUnowned = useMemo(() => unowned?.slice(0, limit) ?? [], [unowned, limit]);

  // Distinct species across all three pools, for the same reason as the graded
  // table above: paging must not change what artwork is requested.
  const speciesOnScreen = useMemo(
    () => [
      ...new Set([
        ...(pairs ?? []).flatMap((p) => [p.parentA.characterId, p.parentB.characterId]),
        ...(blocked ?? []).flatMap((p) => [p.parentA.characterId, p.parentB.characterId]),
        ...(unowned ?? []).flatMap((p) => [p.parentA.characterId, p.parentB.characterId]),
      ]),
    ],
    [pairs, blocked, unowned],
  );
  const icons = useSpeciesIcons(speciesOnScreen);

  const active = { owned: pairs, blocked, unowned }[pool];
  const shownCount = { owned: shownPairs, blocked: shownBlocked, unowned: shownUnowned }[pool]
    .length;

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

      {target && (
        <div className="seg breeding-pools">
          {POOLS.map((option) => (
            <button
              key={option.id}
              className={pool === option.id ? "seg-btn seg-on" : "seg-btn"}
              onClick={() => setPool(option.id)}
              title={option.hint}
            >
              {option.label}
              {counts[option.id] !== null && (
                <span className="seg-count"> {counts[option.id]}</span>
              )}
            </button>
          ))}
        </div>
      )}

      {error && <p className="notice">{error}</p>}
      {busy && <p className="muted">Searching combinations…</p>}

      {!busy && active !== null && active.length === 0 && (
        <p className="muted">
          {pool === "owned" &&
            "Nothing you own breeds into that species right now — check the other two tabs for what is standing in the way."}
          {pool === "blocked" &&
            "No combination is stuck: every pairing you own both species of can be bred today."}
          {pool === "unowned" &&
            "No combination reaches that species, even counting parents you don’t own. It is not obtainable by breeding."}
        </p>
      )}

      {!busy && pool === "owned" && pairs !== null && pairs.length > 0 && (
        <>
          <p className="muted dex-count">
            {pairs.length} owned pair{pairs.length === 1 ? "" : "s"} produce this species —
            showing the best {shownCount}
          </p>
          <ul className="pairs">
            {shownPairs.map((pair) => (
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
          <ShowMore total={pairs.length} shown={shownCount} onMore={() => setLimit((n) => n + PAIR_PAGE)} />
          <p className="muted dex-detail-note">
            One pair per species combination, using the best specimens you own
            of each. Ranked by the parents’ average IVs plus the passives the
            child could inherit — a child takes at most four, so a larger pool
            stops helping past that.
          </p>
        </>
      )}

      {!busy && pool === "blocked" && blocked !== null && blocked.length > 0 && (
        <>
          <p className="muted dex-count">
            {blocked.length === 1
              ? "1 combination you own is stuck"
              : `${blocked.length} combinations you own are stuck`}{" "}
            — showing {shownCount}
          </p>
          <ul className="pairs">
            {shownBlocked.map((pairing) => (
              <li
                key={`${pairing.parentA.instanceId}-${pairing.parentB.instanceId}`}
                className="pair"
              >
                <Parent parent={pairing.parentA} icon={icons[pairing.parentA.characterId]} />
                <span className="pair-plus" aria-hidden="true">
                  +
                </span>
                <Parent parent={pairing.parentB} icon={icons[pairing.parentB.characterId]} />
                <div className="pair-why">
                  <span className="pair-blocked">{blockedLabel(pairing)}</span>
                  <span className="muted">{blockedFix(pairing)}</span>
                </div>
              </li>
            ))}
          </ul>
          <ShowMore
            total={blocked.length}
            shown={shownCount}
            onMore={() => setLimit((n) => n + PAIR_PAGE)}
          />
          <p className="muted dex-detail-note">
            You have both species, but not a pair that can share a farm. Gender
            is read across every Pal you own of each species, not just the best
            few, so a mate buried at the bottom of the box still counts. The
            specimens shown are your best of each.
          </p>
        </>
      )}

      {!busy && pool === "unowned" && unowned !== null && unowned.length > 0 && (
        <>
          <p className="muted dex-count">
            {unowned.length === 1
              ? "1 combination needs"
              : `${unowned.length} combinations need`}{" "}
            a species you don’t own — showing {shownCount}
          </p>
          <ul className="pairs">
            {shownUnowned.map((pairing) => (
              <li
                key={`${pairing.parentA.characterId}-${pairing.parentB.characterId}`}
                className="pair"
              >
                <Side side={pairing.parentA} icon={icons[pairing.parentA.characterId]} />
                <span className="pair-plus" aria-hidden="true">
                  +
                </span>
                <Side side={pairing.parentB} icon={icons[pairing.parentB.characterId]} />
                <div className="pair-why">
                  <span className="pair-need">
                    Need {pairing.missingSpecies.length === 1 ? "1 species" : "2 species"}
                  </span>
                  <span className="muted">{pairing.missingSpecies.join(", ")}</span>
                </div>
              </li>
            ))}
          </ul>
          <ShowMore
            total={unowned.length}
            shown={shownCount}
            onMore={() => setLimit((n) => n + PAIR_PAGE)}
          />
          <p className="muted dex-detail-note">
            Species combinations, not specific Pals: a species nobody owns has
            no IVs, gender or passives to rank on. Ordered by how much you’d
            have to catch first — one new species before two — then by the
            quality of the parent you already have. Combinations where you own
            both parents are under Owned pairs.
          </p>
        </>
      )}
    </div>
  );
}

function ShowMore({
  total,
  shown,
  onMore,
}: {
  total: number;
  shown: number;
  onMore: () => void;
}) {
  if (total <= shown) return null;
  return (
    <button className="btn btn-ghost show-more" onClick={onMore}>
      Show {Math.min(PAIR_PAGE, total - shown)} more
    </button>
  );
}

/** What is standing in the way, in three words. */
function blockedLabel(pairing: BlockedPairingView): string {
  if (pairing.reason === "onlySpecimen") return "Only one owned";
  return pairing.blockingGender === "female" ? "All female" : "All male";
}

/** What to do about it. */
function blockedFix(pairing: BlockedPairingView): string {
  const species = pairing.parentA.displayName ?? pairing.parentA.characterId;
  if (pairing.reason === "onlySpecimen") {
    return `Catch or breed a second ${species} — a Pal can't breed with itself`;
  }
  const wanted = pairing.blockingGender === "female" ? "male" : "female";
  const other = pairing.parentB.displayName ?? pairing.parentB.characterId;
  return species === other
    ? `Every ${species} you own is ${pairing.blockingGender} — you need a ${wanted}`
    : `Need a ${wanted} ${species} or ${other}`;
}

/**
 * One side of an unowned pairing. An owned side shows the best specimen the
 * way the owned list does; an unowned one shows the species alone, marked so
 * the two are never mistaken for each other.
 */
function Side({ side, icon }: { side: PairingSideView; icon?: string }) {
  if (side.owned) return <Parent parent={side.owned} icon={icon} />;
  return (
    <div className="pair-parent pair-parent-missing">
      {icon ? (
        <img className="pal-icon" src={icon} alt="" loading="lazy" width={32} height={32} />
      ) : (
        <span className="pal-icon" aria-hidden="true" />
      )}
      <div className="pair-parent-text">
        <span className="species" title={side.characterId}>
          {side.displayName ?? side.characterId}
        </span>
        <span className="muted">
          {side.dexLabel ? `No.${side.dexLabel} · ` : ""}not owned
        </span>
      </div>
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
