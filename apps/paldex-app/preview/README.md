# UI preview harness

The preview harness runs the production React components in Vite without
starting the Tauri application. Tauri APIs are replaced with browser-side
adapters that load JSON fixtures captured from the real Rust commands.

Use the harness for visual frontend development when launching the desktop shell
is unnecessary or unavailable. It exercises the real component tree, including
the world picker, player scoping, roster filters, analysis, breeding, and bases.

## Prerequisites

- Repository dependencies installed with `pnpm install`
- A locally available hosted Palworld save
- A locally available `Pal-Windows.pak`
- Rust test dependencies built by Cargo

Save and pak paths are discovered automatically. Set `PALDEX_TEST_SAVE_DIR` and
`PALDEX_TEST_PAK` to override discovery; see the root [`README.md`](../../../README.md)
for accepted values.

## Capture fixtures

Run the fixture test from the repository root. `PALDEX_FIXTURE_OUT` must be an
absolute path because Cargo executes the test with the crate directory as its
working directory.

```bash
PALDEX_FIXTURE_OUT="$PWD/apps/paldex-app/public/__fixture__" \
  cargo test -p paldex-app --lib dump_frontend_fixture -- --nocapture
```

The test is a no-op when `PALDEX_FIXTURE_OUT` is unset. If save or pak discovery
fails, it reports that the required real data is unavailable and writes no
fixtures.

Fixture capture includes both values of the **include base Pals** filter and one
file per valid breeding target. Capture duration and output size therefore
depend on the save and installed game version.

### Capture player-scoped fixtures

Player-scoped capture is opt-in because it repeats roster, analysis, and
breeding capture for each player:

```bash
PALDEX_FIXTURE_PLAYERS=1 \
PALDEX_FIXTURE_OUT="$PWD/apps/paldex-app/public/__fixture__" \
  cargo test -p paldex-app --lib dump_frontend_fixture -- --nocapture
```

Without `PALDEX_FIXTURE_PLAYERS`, `world_players.json` contains an empty list.
The player selector is omitted and the harness renders the world-level view used
for single-player worlds.

## Run the preview

```bash
pnpm -C apps/paldex-app preview:ui
```

Vite serves the harness at <http://localhost:5199>. The port is fixed and Vite
fails instead of selecting another port when 5199 is unavailable.

## Implementation

`vite.preview.config.ts` sets `preview/` as the Vite root and aliases the Tauri
modules used by the application:

| Tauri module | Preview adapter |
| --- | --- |
| `@tauri-apps/api/core` | `preview/mock-core.ts` |
| `@tauri-apps/api/event` | `preview/mock-event.ts` |
| `@tauri-apps/plugin-dialog` | `preview/mock-dialog.ts` |

`mock-core.ts` maps `invoke(command, args)` calls to files under
`public/__fixture__/`. Fixture names encode command arguments in this order:

```text
<command>[__<target>][__nobase][__player_<uid>].json
```

Examples:

| Request | Fixture |
| --- | --- |
| `pal_roster` | `pal_roster.json` |
| `pal_roster` with base Pals excluded | `pal_roster__nobase.json` |
| `breeding_options` for Kitsun | `breeding_options__Kitsun.json` |
| Kitsun breeding without base Pals for one player | `breeding_options__Kitsun__nobase__player_<uid>.json` |

Commands with a target argument return an empty list when the corresponding
fixture is absent. This represents a target that the selected breeding pool
cannot produce. Missing fixtures for commands without a target are errors.

The loader verifies the response content type before parsing JSON. This check is
required because Vite returns the preview application's `index.html` with HTTP
200 for unknown paths.

## Fixture coverage

The capture test writes fixtures for:

- Save discovery, world selection, and forced resynchronization
- Paldex progress and player progress
- Roster data and extracted Pal icons
- Quality analysis
- Available and unavailable breeding pairings for each applicable target
- Breeding target metadata
- Base summaries
- Optional per-player variants
- Base-inclusive and base-excluded variants where supported

`base_summary` is not player-scoped and has no base-Pal variant because bases
belong to the guild rather than an individual player.

## Data handling

- Path: `apps/paldex-app/public/__fixture__/`
- Contents: save-derived player identifiers and roster data
- Git state: ignored and untracked
- Production: excluded by `vite.config.ts` (`publicDir: false`)
- Removal: delete the fixture directory

Verify the production/preview boundary after changing either Vite config:

```bash
pnpm -C apps/paldex-app verify:packaging-boundary
```
