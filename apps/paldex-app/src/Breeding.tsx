import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type {
  BreedingPairView,
  BreedingParentView,
  BreedingTargetView,
  PairingNeed,
  PairingSideView,
  SnapshotSummaryView,
  UnownedPairingView,
} from "./types";
import { useSpeciesIcons } from "./icons";

/** Pairs shown at once. A common target has several hundred. */
const PAIR_PAGE = 25;

/** Which parent pool the breeding results are drawn from. */
type Pool = "owned" | "unowned";

const POOLS: { id: Pool; label: string; hint: string }[] = [
  {
    id: "owned",
    label: "Owned pairs",
    hint: "Both parents are Pals in this world, and you can breed them now",
  },
  {
    id: "unowned",
    label: "Unowned parents",
    hint: "Combinations waiting on a Pal you don't have — a new species, a second of one you own, or the other gender",
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
interface Props {
  summary: SnapshotSummaryView;
  /** Whose Pals count as owned parents; `null` is every player. */
  playerUid: string | null;
}

export default function Breeding({ summary, playerUid }: Props) {
  const [targets, setTargets] = useState<{ id: string; label: string }[]>([]);
  const [target, setTarget] = useState("");
  const [pool, setPool] = useState<Pool>("owned");
  const [pairs, setPairs] = useState<BreedingPairView[] | null>(null);
  const [unowned, setUnowned] = useState<UnownedPairingView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // Base-camp Pals have no owner, but they can still be put in a breeding
  // farm, so they count as parents under every player scope by default.
  const [includeBasePals, setIncludeBasePals] = useState(true);
  const [limit, setLimit] = useState(PAIR_PAGE);

  // Only species breeding can actually produce — not the whole Paldeck. The
  // backend filters out the ones the game will not breed and the variant forms
  // nothing yields, so the picker never promises a result that cannot exist.
  useEffect(() => {
    void (async () => {
      try {
        const producible = await invoke<BreedingTargetView[]>("breeding_targets");
        setTargets(
          producible.map((t) => ({
            id: t.characterId,
            label: t.dexLabel ? `No.${t.dexLabel} ${t.displayName}` : t.displayName,
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
      setUnowned(null);
      return;
    }
    let cancelled = false;
    setBusy(true);
    setError(null);
    setLimit(PAIR_PAGE);
    void (async () => {
      try {
        const [owned, waiting] = await Promise.all([
          invoke<BreedingPairView[]>("breeding_options", {
            target,
            playerUid,
            includeBasePals,
          }),
          invoke<UnownedPairingView[]>("unowned_breeding_options", {
            target,
            playerUid,
            includeBasePals,
          }),
        ]);
        if (cancelled) return;
        setPairs(owned);
        setUnowned(waiting);
      } catch (e) {
        if (!cancelled) setError(String(e));
      } finally {
        if (!cancelled) setBusy(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [target, playerUid, includeBasePals, summary.snapshotId]);

  // Paging is per pool, so switching tabs starts at the top of the new list.
  useEffect(() => setLimit(PAIR_PAGE), [pool]);

  const counts: Record<Pool, number | null> = {
    owned: pairs?.length ?? null,
    unowned: unowned?.length ?? null,
  };

  const shownPairs = useMemo(() => pairs?.slice(0, limit) ?? [], [pairs, limit]);
  const shownUnowned = useMemo(() => unowned?.slice(0, limit) ?? [], [unowned, limit]);

  // Distinct species across both pools, for the same reason as the graded
  // table above: paging must not change what artwork is requested.
  const speciesOnScreen = useMemo(
    () => [
      ...new Set([
        ...(pairs ?? []).flatMap((p) => [p.parentA.characterId, p.parentB.characterId]),
        ...(unowned ?? []).flatMap((p) => [p.parentA.characterId, p.parentB.characterId]),
      ]),
    ],
    [pairs, unowned],
  );
  const icons = useSpeciesIcons(speciesOnScreen);

  const active = pool === "owned" ? pairs : unowned;
  const shownCount = (pool === "owned" ? shownPairs : shownUnowned).length;

  return (
    <div className="breeding">
      <div className="breeding-head">
        <h2>Breed for a species</h2>
        <div className="breeding-controls">
          <label className="base-toggle">
            <input
              type="checkbox"
              checked={includeBasePals}
              onChange={(e) => setIncludeBasePals(e.target.checked)}
            />
            Include base pals
          </label>
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
          {pool === "owned"
            ? "Nothing you own breeds into that species right now — Unowned parents shows what is standing in the way."
            : "Nothing left to go and get — every combination for this species is one you can already breed."}
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

      {!busy && pool === "unowned" && unowned !== null && unowned.length > 0 && (
        <>
          <p className="muted dex-count">
            {unowned.length === 1
              ? "1 combination is"
              : `${unowned.length} combinations are`}{" "}
            waiting on a Pal you don’t have — showing {shownCount}
          </p>
          <ul className="pairs">
            {shownUnowned.map((pairing) => (
              <li
                key={`${pairing.parentA.characterId}-${pairing.parentB.characterId}`}
                className="pair"
              >
                <Side
                  side={pairing.parentA}
                  icon={icons[pairing.parentA.characterId]}
                  need={pairing.need}
                />
                <span className="pair-plus" aria-hidden="true">
                  +
                </span>
                <Side
                  side={pairing.parentB}
                  icon={icons[pairing.parentB.characterId]}
                  need={pairing.need}
                />
                <div className="pair-why">
                  <span className="pair-need">{needLabel(pairing)}</span>
                  <span className="muted">{needDetail(pairing)}</span>
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
            Everything you can’t breed today, and what each one is waiting on.
            Ordered by how much you’d have to go and get: another of something
            you own, then the other gender of one you own, then one new species,
            then two — and within that by the quality of the parent you have.
            Gender is read across every Pal you own of a species, not just the
            best few, so a mate at the bottom of the box still counts.
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

/** How big the ask is, as a chip. */
function needLabel(pairing: UnownedPairingView): string {
  switch (pairing.need) {
    case "secondSpecimen":
      return "Need a 2nd";
    case "oppositeGender":
      return pairing.blockingGender === "female" ? "Need a male" : "Need a female";
    case "oneSpecies":
      return "Need 1 species";
    case "twoSpecies":
      return "Need 2 species";
  }
}

/** The ask spelled out, next to the chip. */
function needDetail(pairing: UnownedPairingView): string {
  const a = pairing.parentA.displayName ?? pairing.parentA.characterId;
  const b = pairing.parentB.displayName ?? pairing.parentB.characterId;
  switch (pairing.need) {
    case "secondSpecimen":
      return `You own one ${a} — a Pal can’t breed with itself, so you need another`;
    case "oppositeGender": {
      const wanted = pairing.blockingGender === "female" ? "male" : "female";
      return a === b
        ? `Every ${a} you own is ${pairing.blockingGender} — you need a ${wanted}`
        : `Both are ${pairing.blockingGender} — you need a ${wanted} ${a} or ${b}`;
    }
    default:
      return pairing.missingSpecies.join(", ");
  }
}

/**
 * One side of an unowned pairing. An owned side shows the best specimen the
 * way the owned list does; an unowned one shows the species alone, marked so
 * the two are never mistaken for each other.
 */
function Side({
  side,
  icon,
  need,
}: {
  side: PairingSideView;
  icon?: string;
  need: PairingNeed;
}) {
  if (side.owned) return <Parent parent={side.owned} icon={icon} />;
  // For a second-specimen need the empty slot is a species you *do* own, so
  // "not owned" would be plainly wrong — what's missing is another one.
  const sub =
    need === "secondSpecimen"
      ? "need a second"
      : `${side.dexLabel ? `No.${side.dexLabel} · ` : ""}not owned`;
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
        <span className="muted">{sub}</span>
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
