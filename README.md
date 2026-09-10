# Paldex

Paldex is a local desktop application that reads Palworld save data and installed
game assets. It tracks Paldex completion, owned Pals, breeding options, and base
assignments, and refreshes its local state when the game writes a new save.

The application does not require an account or backend service. Save files and
game assets remain on the local machine; derived state is stored in SQLite.

## Platform support

| Platform | Save and pak discovery |
| --- | --- |
| Windows | Native Steam installation, including secondary Steam libraries |
| macOS on Apple Silicon | Palworld installations in CrossOver and Whisky bottles |

A save directory can also be selected manually. Hosted worlds are trackable.
Worlds joined as a guest are listed but cannot be decoded because their world
state is stored on the host.

## Features

| View | Data |
| --- | --- |
| Paldex | Per-player unlock state, capture counts, and progress toward the five-capture bonus |
| Roster | Species, level, elements, IVs, passives, condensation rank, and party/box/base location |
| Analysis | Composite IV scores, best specimens, condensation candidates, and passive rankings |
| Breeding | Available owned pairings, unavailable pairings, and missing parent species for a selected result |
| Bases | Base camps, guild ownership, and assigned workers |

Multiplayer saves expose a player selector. All views are scoped to the selected
player while base workers remain visible across player scopes.

Reference data—including display names, elements, artwork, base stats, and
breeding combinations—is read from the installed `Pal-Windows.pak`. If the pak
is unavailable, save decoding still works, but the UI uses internal identifiers
and disables features that require reference data.

## Architecture

```text
Palworld SaveGames
    │
    ├─ paldex-locate   discover Windows, CrossOver, and Whisky save roots
    ├─ paldex-sav      parse .sav containers and decompress zlib/Oodle payloads
    ├─ paldex-gvas     decode UE 5.1 GVAS property trees
    ├─ paldex-model    decode Palworld RawData and compute domain analysis
    └─ paldex-store    ingest snapshots and watch Level.sav for updates
                              │
Pal-Windows.pak              │
    └─ paldex-data     extract names, stats, breeding data, and textures
                              │
                       paldex-app
                       Tauri 2 backend + React frontend
```

The SQLite store separates current snapshots from append-only dex events. A
later save cannot remove a previously observed Paldex unlock. Current-state rows
are replaced when a new snapshot is ingested, and old snapshots are pruned.

### Save compression

Paldex supports both Palworld save container variants:

- `PlZ`: zlib-compressed payloads
- `PlM`: Oodle Kraken-compressed payloads

Oodle payloads are decoded with
[`oozextract`](https://crates.io/crates/oozextract), an MIT-licensed clean-room
Kraken decoder. No native `oo2core` library is required.

### Pak access

`crates/repak-oodle` is a vendored copy of
[`trumank/repak`](https://github.com/trumank/repak). Its Oodle decompression path
uses `oozextract` instead of loading a Windows x64 `oo2core` DLL. Pak writing and
Oodle compression are not used by Paldex. See
[`crates/repak-oodle/NOTICE.md`](crates/repak-oodle/NOTICE.md) for the patch scope
and upstream attribution.

### Unversioned property mappings

Palworld cooks numeric DataTable fields with `PKG_UnversionedProperties`.
`crates/paldex-data/data/Mappings.usmap` supplies the schema required to decode
those fields. See [Regenerating `Mappings.usmap`](#regenerating-mappingsusmap)
for the update procedure.

## Requirements

- Rust 1.85 or newer
- Node.js `^20.19` or `>=22.12`
- pnpm 10
- The [Tauri 2 system prerequisites](https://v2.tauri.app/start/prerequisites/)
  for the target platform
- A local Palworld installation for runtime reference data and real-data tests

The repository currently pins pnpm through the `packageManager` field in
`package.json`.

## Build and run

Install dependencies from the repository root:

```bash
pnpm install
```

Start the Tauri development application:

```bash
pnpm dev
```

Build an installable application bundle:

```bash
pnpm build
```

The root scripts delegate frontend work to `apps/paldex-app` and Rust work to
the Cargo workspace.

## Validation

Run the standard checks from the repository root:

```bash
pnpm typecheck
pnpm -C apps/paldex-app lint
pnpm lint:rust
pnpm test:rust
```

`pnpm test:rust` runs `cargo test --workspace`.

### Real-data tests

Integration tests that require Palworld data use automatic discovery by default.
They skip when the required local data is unavailable. The following variables
override discovery:

| Variable | Value |
| --- | --- |
| `PALDEX_TEST_SAVE_DIR` | A `SaveGames` directory, Steam-ID directory, or individual world directory |
| `PALDEX_TEST_PAK` | Absolute path to `Pal-Windows.pak` |

Example:

```bash
PALDEX_TEST_SAVE_DIR=/path/to/SaveGames \
PALDEX_TEST_PAK=/path/to/Pal-Windows.pak \
  cargo test --workspace
```

### Browser-only UI preview

The preview harness renders the production React components without starting
Tauri. It requires fixtures captured from a real save and pak:

```bash
PALDEX_FIXTURE_OUT="$PWD/apps/paldex-app/public/__fixture__" \
  cargo test -p paldex-app --lib dump_frontend_fixture -- --nocapture
pnpm -C apps/paldex-app preview:ui
```

Fixture data is derived from a personal save and is ignored by Git. Set
`PALDEX_FIXTURE_PLAYERS=1` during capture to generate per-player fixtures. See
[`apps/paldex-app/preview/README.md`](apps/paldex-app/preview/README.md) for the
fixture naming and capture behavior.

## Runtime data and privacy

- The desktop application opens Palworld saves and pak files read-only.
- The application does not write to Palworld directories or rotating backups.
- The application makes no network requests.
- The WebView uses a restrictive content security policy; image sources are
  limited to local application, data, blob, and Tauri asset URLs.
- Captured UI preview fixtures contain derived save data and must not be
  committed.

The application database is `paldex.sqlite3` under Tauri's platform app-data
directory:

| Platform | Default location |
| --- | --- |
| macOS | `~/Library/Application Support/gg.paldex.desktop/` |
| Windows | `%APPDATA%\gg.paldex.desktop\` |

Deleting the database resets local state; the next successful sync recreates it.

## Regenerating `Mappings.usmap`

Regeneration requires:

- Windows with Palworld installed
- Git
- Visual Studio Build Tools with the C++ workload
- An interactive desktop session with a working GPU

From PowerShell at the repository root:

```powershell
.\tools\usmap\regen-usmap.ps1 `
  -OutFile .\crates\paldex-data\data\Mappings.usmap
```

The script locates Palworld and MSBuild, clones the pinned mappings dumper,
applies `tools/usmap/palworld-ue51.patch`, builds the dumper, launches Palworld,
injects the dumper DLL, and copies the generated schema to `-OutFile`. Use
`Get-Help .\tools\usmap\regen-usmap.ps1 -Detailed` for all parameters.
See [`tools/usmap/README.md`](tools/usmap/README.md) for parameter details,
side effects, patch scope, and troubleshooting.

Validate the generated file before use:

```powershell
python3 .\tools\usmap\verify_usmap.py `
  .\crates\paldex-data\data\Mappings.usmap
```

A headless session or `-nullrhi` is insufficient because the dumper traverses the
live rendered object graph.

## Repository layout

```text
apps/paldex-app/
  src/                    React and TypeScript frontend
  src-tauri/              Tauri backend, commands, queries, and sync orchestration
  preview/                Browser-only fixture preview
crates/
  paldex-locate/          Save and pak discovery
  paldex-sav/             Save container parsing and decompression
  paldex-gvas/            UE 5.1 GVAS property-tree decoder
  paldex-model/           Domain types, RawData decoders, and analysis
  paldex-data/            Pak reader and runtime reference-data extraction
  paldex-store/           SQLite persistence and save watcher
  repak-oodle/            Vendored pak reader with cross-platform Oodle decoding
tools/usmap/              Mapping generation and validation tools
```

## License

Except where noted below, Paldex is available under either:

- [Apache License 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

Contributions intentionally submitted for inclusion are licensed under the same
terms unless stated otherwise.

`crates/repak-oodle` retains its upstream MIT/Apache-2.0 licensing and includes
its own license and notice files.

`crates/paldex-data/data/Mappings.usmap` is mechanically extracted from
Palworld's shipping executable and is not covered by this repository's
MIT/Apache-2.0 license grant. Redistributors must evaluate that file separately
or regenerate it with `tools/usmap/regen-usmap.ps1`.

No other game content is redistributed. Display names, artwork, stats, and
breeding data are read at runtime from the user's installed copy of Palworld.

Paldex is an unofficial fan project. Palworld is a trademark of Pocketpair, Inc.
This project is not affiliated with or endorsed by Pocketpair.
