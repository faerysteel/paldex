# paldex-data

Runtime reference-data extraction from the installed `Pal-Windows.pak`.
Names, stats, breeding tables, and artwork are read locally, not vendored.
The property schema, `data/Mappings.usmap`, is bundled separately.

## API

| API | Contract |
| --- | --- |
| `Pak::open(path)` | Open and index a pak for path-based reads |
| `ReferenceIndex::extract(&mut pak, language)` | Build the index using localized text tables; see `TEXT_LANGUAGES` |
| `ReferenceData` | Species, passive-name, breeding-result, and icon-path lookups |
| `PassthroughReferenceData` | Return `None` for every lookup; callers provide display fallbacks |
| `load_icon_png(&mut pak, base)` | Decode a texture's first mip to PNG; `base` has no extension |
| `BUNDLED_MAPPINGS` | Embedded `Mappings.usmap` bytes |

`ReferenceIndex` also exposes NPC classification, technology/item/map-object
names, and localized element/work labels. Passive entries contain identifiers
and display names, not effect values or quality tiers.

## Data sources

| Data | Source | Decoder |
| --- | --- | --- |
| Pal/NPC classification | Name tables in `DT_PalMonsterParameter`, `DT_PalHumanParameter`, and their `_Common` companions | `uasset` |
| Species, skill, technology, item, and map-object names | Per-language text DataTables | `text_table` |
| Element and work labels | `DT_UI_Common_Text_Common` | `text_table` |
| Paldeck numbers, elements, stats, rarity, work levels, breeding ranks | Monster parameter rows | `datatable`, `unversioned`, `usmap` |
| Breeding exceptions | `DT_PalCombiUnique` | `datatable`, `breeding` |
| Icons | `Texture/PalIcon/Normal/` textures | `texture` |

Package name tables contain row-name candidates plus other referenced names;
classification does not decode the parameter rows. Species lookups normalize
case and strip `BOSS_`/`PREDATOR_` prefixes.

Text extraction scans supported `FText` records without a row schema. Texture
extraction locates platform data by pixel-format strings and decodes BC1/BC2/BC3.
Row-value extraction requires the bundled schema. Breeding combines authored
exceptions with the generic `CombiRank` rule; see [`breeding`](src/breeding.rs).

## Mappings

Palworld's unversioned DataTable properties omit names and types.
`data/Mappings.usmap` supplies their positional schema. The reader supports
version 0, uncompressed.

After a game update, regenerate and validate the schema using the
[mappings workflow](../../tools/usmap/README.md) and
[`regen-usmap.ps1`](../../tools/usmap/regen-usmap.ps1).
See the [license exception](../../README.md#license) before redistributing it.

## Failure behavior

- Required text-table failures and malformed readable parameter-package headers
  return `ExtractError`. An empty species text table also fails extraction.
- Schema, row-decoding, breeding-exception, and UI-label errors are non-fatal and
  recorded in `warnings()`.
- Unreadable parameter packages are skipped and do not necessarily produce a
  warning. An empty warning list does not prove complete extraction.
- A row-decoding failure can leave names and icons available but numeric fields
  at defaults and breeding results incomplete. Missing parameter name tables
  can also reduce classification coverage.
- Icon decoding returns its own error; the app handles missing artwork per icon.

## Source map

- [`lib.rs`](src/lib.rs): pak access, icon loading, bundled schema.
- [`extract.rs`](src/extract.rs): index construction and normalization.
- [`reference.rs`](src/reference.rs): lookup trait and data types.
- [`uasset.rs`](src/uasset.rs): package headers, name/import/export tables.
- [`usmap.rs`](src/usmap.rs), [`datatable.rs`](src/datatable.rs),
  [`unversioned.rs`](src/unversioned.rs): schema and row decoding.
- [`text_table.rs`](src/text_table.rs), [`texture.rs`](src/texture.rs),
  [`breeding.rs`](src/breeding.rs): text, artwork, and breeding logic.

## Verification and probes

Run from the repository root:

```bash
cargo test -p paldex-data
cargo run -p paldex-data --example inspect_pak -- /path/to/Pal-Windows.pak
```

Real-data tests accept `PALDEX_TEST_PAK`; save-joining tests also accept
`PALDEX_TEST_SAVE_DIR`. Default pak lookup varies: most use discovery, while
`real_pak` uses a fixed CrossOver path. Missing data skips dependent cases;
read/parse failures may either skip or fail, depending on the test helper.
`real_parameters` checks bundled schema structure without a game install and
parameter decoding when a pak is available.

[`examples/`](examples/) contains package, row, text, and texture probes, reference
and icon dumps, and extraction timing tools. Arguments vary by example; check
its entry point before running.
