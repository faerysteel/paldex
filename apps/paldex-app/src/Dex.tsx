import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { DexEntryView, DexProgressView, SnapshotSummaryView } from "./types";
import { useSpeciesIcons } from "./icons";

/** Captures needed for a species' capture bonus. */
const BONUS_AT = 10;

type Show = "all" | "caught" | "missing";

interface Props {
  summary: SnapshotSummaryView;
}

export default function Dex({ summary }: Props) {
  const [dex, setDex] = useState<DexProgressView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [show, setShow] = useState<Show>("all");
  const [filter, setFilter] = useState("");

  const load = useCallback(async () => {
    setError(null);
    try {
      setDex(await invoke<DexProgressView>("dex_progress"));
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, summary.snapshotId]);

  const entries = useMemo(() => dex?.entries ?? [], [dex]);

  const visible = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    return entries.filter((e) => {
      if (show === "caught" && !e.caught) return false;
      if (show === "missing" && e.caught) return false;
      if (!needle) return true;
      return (
        e.displayName.toLowerCase().includes(needle) ||
        e.characterId.toLowerCase().includes(needle) ||
        (e.dexLabel?.toLowerCase().includes(needle) ?? false)
      );
    });
  }, [entries, show, filter]);

  // Only the species actually on screen are worth decoding artwork for.
  const speciesOnScreen = useMemo(() => visible.map((e) => e.characterId), [visible]);
  const icons = useSpeciesIcons(speciesOnScreen);

  const caught = dex?.unlockedSpeciesCount ?? 0;
  const total = dex?.totalSpeciesCount ?? 0;
  const bonusComplete = useMemo(
    () => entries.filter((e) => isBonusComplete(e)).length,
    [entries],
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
            hint={`species with ${BONUS_AT}+ captures`}
          />
          {total > 0 && (
            <Stat label="Completion" value={`${Math.round((caught / total) * 100)}%`} />
          )}
        </div>

        <div className="dex-controls">
          <input
            className="filter dex-filter"
            placeholder="Filter by name or number…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
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

      <p className="muted dex-count">
        Showing {visible.length} of {entries.length}
      </p>

      <ul className="dex-grid">
        {visible.map((entry) => (
          <Tile key={entry.characterId} entry={entry} icon={icons[entry.characterId]} />
        ))}
      </ul>

      {visible.length === 0 && <p className="muted">No species match that filter.</p>}
    </section>
  );
}

function isBonusComplete(entry: DexEntryView): boolean {
  return entry.bonusClaimed || entry.captureCount >= BONUS_AT;
}

function Tile({ entry, icon }: { entry: DexEntryView; icon?: string }) {
  const complete = isBonusComplete(entry);
  const className = [
    "dex-tile",
    entry.caught ? "dex-caught" : "dex-unseen",
    complete ? "dex-bonus" : "",
  ]
    .filter(Boolean)
    .join(" ");

  const label = entry.dexLabel ? `No.${entry.dexLabel} ${entry.displayName}` : entry.displayName;
  const title = entry.caught
    ? `${label} — ${entry.captureCount} capture${entry.captureCount === 1 ? "" : "s"}${
        complete ? ", bonus complete" : ""
      }`
    : `${label} — not caught`;

  return (
    <li className={className} title={title}>
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
      <span className="dex-meta">
        {entry.caught ? (
          <>
            {entry.captureCount > 0 && (
              <span className="dex-captures">×{entry.captureCount}</span>
            )}
            {complete && (
              <span className="dex-star" title="Capture bonus complete">
                ★
              </span>
            )}
          </>
        ) : (
          <span className="dex-dash">—</span>
        )}
      </span>
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
