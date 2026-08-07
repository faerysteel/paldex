import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

import type { SaveRootView, SnapshotSummaryView, World, WorldKind } from "./types";
import { relativeTime } from "./time";
import WorldView from "./WorldView";

type LoadState =
  | { status: "loading" }
  | { status: "ready"; roots: SaveRootView[] }
  | { status: "error"; message: string };

export default function App() {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [notice, setNotice] = useState<string | null>(null);
  const [summary, setSummary] = useState<SnapshotSummaryView | null>(null);
  const [selecting, setSelecting] = useState<string | null>(null);

  const selectWorld = useCallback(async (world: World) => {
    setSelecting(world.path);
    setNotice(null);
    try {
      const result = await invoke<SnapshotSummaryView>("select_world", {
        worldPath: world.path,
      });
      setSummary(result);
    } catch (e) {
      setNotice(String(e));
    } finally {
      setSelecting(null);
    }
  }, []);

  const scan = useCallback(async () => {
    setState({ status: "loading" });
    setNotice(null);
    try {
      const roots = await invoke<SaveRootView[]>("list_saves");
      setState({ status: "ready", roots });
    } catch (e) {
      setState({ status: "error", message: String(e) });
    }
  }, []);

  useEffect(() => {
    void scan();
  }, [scan]);

  const chooseFolder = useCallback(async () => {
    const picked = await open({
      directory: true,
      title: "Select your Palworld SaveGames folder",
    });
    if (typeof picked !== "string") return;

    try {
      const roots = await invoke<SaveRootView[]>("resolve_folder", {
        path: picked,
      });
      setNotice(null);
      setState({ status: "ready", roots });
    } catch (e) {
      setNotice(String(e));
    }
  }, []);

  // Every hook above runs on every render. Selecting a world used to return
  // here *before* `scan`/`chooseFolder` were declared, so the render after
  // selection ran fewer hooks than the one before it — React aborts the whole
  // tree on that, which showed up as a black window with no error.
  if (summary) {
    return (
      <WorldView
        summary={summary}
        onBack={() => setSummary(null)}
        onSummaryChange={setSummary}
      />
    );
  }

  return (
    <div className="app">
      <header className="header">
        <div>
          <h1>Paldex</h1>
          <p className="tagline">
            Local Palworld save tracker — nothing leaves this machine.
          </p>
        </div>
        <div className="actions">
          <button className="btn" onClick={() => void scan()}>
            Rescan
          </button>
          <button className="btn btn-ghost" onClick={() => void chooseFolder()}>
            Choose folder…
          </button>
        </div>
      </header>

      {notice && <p className="notice">{notice}</p>}

      <main>
        {state.status === "loading" && <p className="muted">Looking for saves…</p>}

        {state.status === "error" && (
          <div className="panel panel-error">
            <h2>Could not scan for saves</h2>
            <p className="muted">{state.message}</p>
          </div>
        )}

        {state.status === "ready" &&
          (state.roots.length === 0 ? (
            <NoSavesFound onChoose={() => void chooseFolder()} />
          ) : (
            state.roots.map((root) => (
              <RootCard
                key={root.path}
                root={root}
                onSelectWorld={(w) => void selectWorld(w)}
                selecting={selecting}
              />
            ))
          ))}
      </main>
    </div>
  );
}

function RootCard({
  root,
  onSelectWorld,
  selecting,
}: {
  root: SaveRootView;
  onSelectWorld: (world: World) => void;
  selecting: string | null;
}) {
  const trackable = root.worlds.filter((w) => w.kind === "localWorld");
  const others = root.worlds.filter((w) => w.kind !== "localWorld");

  return (
    <section className="panel">
      <div className="root-head">
        <span className="source">{root.sourceLabel}</span>
        <span className="steam-id">Steam ID {root.steamId}</span>
      </div>
      <p className="path" title={root.path}>
        {root.path}
      </p>

      {trackable.length > 0 && (
        <ul className="worlds">
          {trackable.map((w) => (
            <WorldRow
              key={w.path}
              world={w}
              onSelect={onSelectWorld}
              selecting={selecting === w.path}
            />
          ))}
        </ul>
      )}

      {others.length > 0 && (
        <details className="others">
          <summary>
            {others.length} world{others.length === 1 ? "" : "s"} that can’t be
            tracked
          </summary>
          <p className="muted explain">
            Worlds you joined as a guest keep only a small local file — the world
            state itself lives on the host’s machine, so there is nothing here to
            read.
          </p>
          <ul className="worlds">
            {others.map((w) => (
              <WorldRow key={w.path} world={w} onSelect={onSelectWorld} selecting={false} />
            ))}
          </ul>
        </details>
      )}

      {root.worlds.length === 0 && (
        <p className="muted">No world folders in this save.</p>
      )}
    </section>
  );
}

function WorldRow({
  world,
  onSelect,
  selecting,
}: {
  world: World;
  onSelect: (world: World) => void;
  selecting: boolean;
}) {
  const trackable = world.kind === "localWorld";
  return (
    <li
      className={trackable ? "world world-clickable" : "world world-dim"}
      onClick={trackable ? () => onSelect(world) : undefined}
    >
      <div className="world-main">
        <span className="world-id" title={world.id}>
          {world.id.slice(0, 8)}
        </span>
        <KindBadge kind={world.kind} />
      </div>
      <div className="world-meta">
        {trackable && (
          <span>
            {world.players.length} player
            {world.players.length === 1 ? "" : "s"}
          </span>
        )}
        <span title={formatAbsolute(world.lastPlayed)}>
          {relativeTime(world.lastPlayed)}
        </span>
        {selecting && <span className="muted">Loading…</span>}
      </div>
    </li>
  );
}

const KIND_LABEL: Record<WorldKind, string> = {
  localWorld: "Hosted here",
  coopGuestStub: "Co-op guest",
  unknown: "Unrecognised",
};

function KindBadge({ kind }: { kind: WorldKind }) {
  return <span className={`badge badge-${kind}`}>{KIND_LABEL[kind]}</span>;
}

function NoSavesFound({ onChoose }: { onChoose: () => void }) {
  return (
    <div className="panel">
      <h2>No Palworld saves found</h2>
      <p className="muted">Paldex looked in the usual places:</p>
      <ul className="muted checklist">
        <li>CrossOver bottles — every bottle, every Windows user</li>
        <li>Whisky bottles</li>
        <li>
          <code>%LOCALAPPDATA%\Pal\Saved\SaveGames</code> on Windows
        </li>
      </ul>
      <p className="muted">
        If Palworld is installed somewhere unusual, point Paldex at the folder
        yourself. Your <code>SaveGames</code> folder, a Steam-ID folder, or a
        single world folder will all work.
      </p>
      <button className="btn" onClick={onChoose}>
        Choose folder…
      </button>
    </div>
  );
}

function formatAbsolute(ms: number | null): string {
  return ms === null ? "unknown" : new Date(ms).toLocaleString();
}
