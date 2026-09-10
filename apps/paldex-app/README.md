# paldex-app

Tauri 2 desktop app with a React frontend. Reads Palworld saves into SQLite and
joins them with reference data from the installed game pak.

## Modules

| Path | Responsibility |
| --- | --- |
| `src/main.tsx` | Mount `App` inside `ErrorBoundary` |
| `src/App.tsx` | Discover saves, select a world, hold its snapshot summary |
| `src/WorldView.tsx` | Tabs, player selection, manual resync, snapshot event subscription |
| `src/Dex.tsx`, `src/Roster.tsx` | Paldex progress and owned Pals |
| `src/Analysis.tsx` | IV scores, best specimens, condensation candidates, passive-count ranking |
| `src/Breeding.tsx`, `src/Bases.tsx` | Parent-pair searches and base workers |
| `src-tauri/src/commands.rs` | Tauri handlers, reference enrichment, app state, watcher lifecycle |
| `src-tauri/src/sync.rs` | Save decoding and snapshot ingest |
| `src-tauri/src/queries.rs` | SQLite queries and frontend response types |

Save decoding, roster queries, and icon decoding use
`#[tauri::command(async)]` to avoid blocking the main thread.

## App state

`AppState` stores:

- `store`: mutex-protected SQLite connection, opened lazily as `paldex.sqlite3`
  in Tauri's app-data directory.
- `reference`: `OnceLock<Option<LoadedReference>>`; loads English reference data
  once, retaining the pak handle and an icon cache. Failed discovery is cached
  for the session.
- `selected`: world id, path, and latest snapshot id.
- `watcher`: cooperative stop flag for the detached watcher thread.

## Snapshot lifecycle

1. `select_world` resolves a hosted world, forces an initial ingest, stores the
   selection, and starts a watcher. Replacing the watcher signals the previous
   thread to stop; an in-flight sync may finish.
2. The watcher debounces `Level.sav` and `Players/*.sav` events, excluding
   `*_dps.sav`, and checks the last event path for size stability.
3. `on_watched_change` calls `sync_world_if_changed`, which re-reads and
   decompresses `Level.sav`. A matching SHA-256 skips parsing and ingest.
4. A changed world is decoded into Pals, player identities, guilds, and bases.
   Player files supply progression and party/box locations; unreadable or
   undecodable player files are skipped. Base containers resolve base locations.
5. `Store::ingest_snapshot` writes a new snapshot in one transaction. If the
   world is still selected, the backend updates its snapshot id and emits
   `paldex://snapshot`. `WorldView` updates the summary; tabs reload on snapshot-id
   changes.

Limitations:

- Automatic change detection hashes only decompressed `Level.sav`. Player-only
  changes are skipped while that hash is unchanged. `force_resync` bypasses the
  hash check.
- Size stability is not content validation or an atomic read of the world.
  Automatic sync failures are logged; the next save event triggers another
  attempt. Manual resync remains available if watcher startup fails.
- The app does not call `Store::prune_snapshots`; snapshot history accumulates.

## Player scope

- `playerUid = null` selects all players; the selector appears for multiplayer
  worlds and persists across tab switches.
- Paldex, roster, analysis, and breeding accept player scope. Bases are
  world-scoped.
- Roster, analysis, and breeding each have an `includeBasePals` toggle, enabled
  by default. It controls ownerless rows independently of player scope.

## Without reference data

Save decoding and IV analysis remain available. The UI uses internal ids,
retains human NPC rows, omits icons and species metadata, and shows only caught
species in the dex. Breeding queries return no results.

## Development and verification

Run from the repository root; see the [root README](../../README.md#requirements)
for prerequisites.

```bash
pnpm install
pnpm dev
pnpm build
pnpm typecheck
pnpm -C apps/paldex-app lint
cargo test -p paldex-app
```

Backend tests are in `commands.rs` and `sync.rs`. Real-data cases skip when
required saves or pak data are unavailable. `PALDEX_TEST_SAVE_DIR` overrides
save discovery; app reference loading uses pak discovery, not `PALDEX_TEST_PAK`.

For browser-only rendering with captured fixtures, see the
[preview harness](preview/README.md).
