# Palworld usmap tooling

This directory contains the tools used to generate and validate the Unreal
property mappings bundled with Paldex at
[`crates/paldex-data/data/Mappings.usmap`](../../crates/paldex-data/data/Mappings.usmap).

Palworld cooks DataTables with `PKG_UnversionedProperties`. Numeric fields in
those assets do not carry property names or types, so `paldex-data` requires a
matching `.usmap` schema to decode them.

## Contents

| File | Purpose |
| --- | --- |
| [`regen-usmap.ps1`](regen-usmap.ps1) | Builds a patched mappings dumper, injects it into Palworld, and collects `Mappings.usmap` |
| [`palworld-ue51.patch`](palworld-ue51.patch) | Adapts the pinned dumper revision to Palworld's Unreal Engine 5.1 layout |
| [`verify_usmap.py`](verify_usmap.py) | Validates the generated file's header, name and enum sections, known enum values, and required schema names |

Environment-specific wrapper scripts may use a `*.local.sh` suffix. That pattern
is ignored by Git and is not part of the portable workflow documented here.

## Requirements

Generation must run on a Windows x64 system with:

- Palworld installed through Steam
- An interactive desktop session with a working GPU
- PowerShell
- Git on `PATH`
- Visual Studio Build Tools with the C++ workload and MSBuild

The game must initialize its rendered object graph. Headless sessions,
`-nullrhi`, and remote sessions without GPU acceleration do not expose enough
objects for a complete dump.

Validation additionally requires Python 3 and can run on any platform.

## Generate a mapping

Run from PowerShell at the repository root:

```powershell
.\tools\usmap\regen-usmap.ps1 `
  -OutFile .\crates\paldex-data\data\Mappings.usmap
```

The script performs the following steps:

1. Locates Palworld through the Steam registry and `libraryfolders.vdf`.
2. Locates MSBuild through Visual Studio's `vswhere.exe`.
3. Clones `gameknife/UnrealMappingsDumper` into `-SrcDir` if needed.
4. Checks out pinned revision `3bf7e24` and initializes its submodules.
5. Resets tracked changes in that checkout and applies
   `palworld-ue51.patch` with `git apply`.
6. Builds `UnrealMappingsDumper.sln` as Release x64.
7. Starts Palworld, or attaches to an existing Palworld process.
8. Injects the generated DLL by calling `LoadLibraryW` in the game process.
9. Waits for the dumper to write `Mappings.usmap` beside the game executable.
10. Copies the result to `-OutFile` and stops the game unless `-KeepGame` is set.

The source checkout is reusable, but it is treated as disposable: each normal
run executes `git checkout -- .` before applying the patch. Do not keep unrelated
tracked changes in `-SrcDir`.

### Parameters

| Parameter | Default | Description |
| --- | --- | --- |
| `-OutFile` | `Mappings.usmap` in the current directory | Destination for the generated mapping |
| `-GameDir` | Auto-detected | Directory containing `Palworld-Win64-Shipping.exe` |
| `-SrcDir` | `%TEMP%\UnrealMappingsDumper` | Dumper checkout and build directory |
| `-MSBuild` | Auto-detected | Full path to `MSBuild.exe` |
| `-SkipBuild` | Disabled | Reuse `x64\UE4SS.dll` already present under `-SrcDir` |
| `-KeepGame` | Disabled | Leave Palworld running after collection |

Display PowerShell's generated parameter help with:

```powershell
Get-Help .\tools\usmap\regen-usmap.ps1 -Detailed
```

### Process side effects

The generator intentionally interacts with the running game process:

- It removes a previous `Mappings.usmap` beside the game executable before
  injection.
- It loads the generated dumper DLL into `Palworld-Win64-Shipping.exe`.
- It terminates `CrashReportClient` before linking and during cleanup.
- It terminates Palworld after collection unless `-KeepGame` is supplied.
- It writes build artifacts and `build.log` under `-SrcDir`.
- It writes the final mapping to `-OutFile`.

If the script attaches to a game that was already running, the default cleanup
still stops that process. Use `-KeepGame` when that is not desired.

## Validate a mapping

Run the validator before using or committing a generated file:

```bash
python3 tools/usmap/verify_usmap.py \
  crates/paldex-data/data/Mappings.usmap
```

A successful run reports the file size, SHA-256 digest, usmap version, name and
enum counts, declared struct count, known enum checks, and required-name checks.
It exits with status 0 only when all checks pass.

By default, the validator requires these names:

- `ZukanIndex`
- `ZukanIndexSuffix`

Override the required-name set when validating another schema dependency:

```bash
python3 tools/usmap/verify_usmap.py path/to/Mappings.usmap \
  --require ZukanIndex CombiRank
```

The validator currently accepts uncompressed usmap files only. It fully walks
the length-prefixed name and enum sections and checks the declared file size,
but it reports the struct count without decoding every struct entry. Successful
validation therefore detects malformed headers, section misalignment, invalid
enum data, and missing required names; application integration tests remain the
final check that the required DataTables decode correctly.

Run those tests with explicit local data paths when needed:

```bash
PALDEX_TEST_SAVE_DIR=/path/to/SaveGames \
PALDEX_TEST_PAK=/path/to/Pal-Windows.pak \
  cargo test -p paldex-data --tests
```

## Patch scope

`palworld-ue51.patch` makes four targeted changes to the pinned dumper:

1. Selects the Unreal Engine 5.1 `FProperty` layout for
   `Palworld-Win64-Shipping.exe`.
2. Adjusts the `UEnum::Names` offset for UE 5.1.
3. Bounds-checks enum counts before walking or serializing them.
4. Writes an unbuffered `dumper.log` beside the injected DLL and fixes unsafe
   format-string calls in object-name logging.

If the pinned upstream revision changes, `git apply` may reject the patch. Update
the pin and patch together, inspect the resulting source diff, generate a fresh
mapping, and run both the validator and the real-data tests.

## Troubleshooting

### Palworld is not found

Pass the directory containing the shipping executable explicitly:

```powershell
.\tools\usmap\regen-usmap.ps1 `
  -GameDir 'D:\SteamLibrary\steamapps\common\Palworld\Pal\Binaries\Win64' `
  -OutFile .\Mappings.usmap
```

### MSBuild is not found

Install the Visual Studio C++ workload or pass `-MSBuild` with the full path to
`MSBuild.exe`.

### The build reports a mapped section

A crashed game may leave `CrashReportClient` holding the previous DLL open. The
script attempts to stop that process before rebuilding. If the error persists,
close Palworld and its crash reporter, then rerun without `-SkipBuild`.

### No mapping is produced

Confirm that Palworld rendered successfully in an interactive GPU-backed
session. The dumper waits before traversing the object graph and writes
`dumper.log` beside the built DLL. The game may hold that log open until it
exits; rerun without `-KeepGame` when inspecting final log output.

### Validation reports empty or malformed enums

The dumper probably used an incompatible `UEnum::Names` offset, or the game
update changed the relevant layout. Do not use the generated file. Re-evaluate
`palworld-ue51.patch` against the current game and dumper revision.

## Licensing

The scripts and patch are covered by the repository's MIT/Apache-2.0 license.
The generated `Mappings.usmap` is mechanically extracted from Palworld and is
excluded from that license grant. See the root [`README.md`](../../README.md#license)
for the repository's licensing and redistribution notes.
