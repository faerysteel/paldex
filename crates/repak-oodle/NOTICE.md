# Vendored from trumank/repak

This crate is a vendored, modified copy of [trumank/repak](https://github.com/trumank/repak)
(commit `355b5f6`), dual-licensed MIT/Apache-2.0 (see `LICENSE-MIT` and
`LICENSE-APACHE`).

## Vendoring rationale

`repak::Entry` is crate-private. Public entry reads (`PakReader::get` and
`read_file`) perform decompression internally, so callers cannot replace the
Oodle decoder without modifying the crate.

The upstream `oodle` feature uses `oodle_loader` to load a native `oo2core`
library. This fork uses the pure-Rust `oozextract` decoder to support Apple
Silicon without a native Oodle library, as `paldex-sav` does for save files.

## Patch scope

| File | Change |
| --- | --- |
| `src/entry.rs` | Decode Oodle blocks with `oozextract::Extractor` |
| `src/data.rs` | Return an error for Oodle compression; no encoder is provided |
| `src/error.rs` | Report unsupported Oodle writing |
| `Cargo.toml` | Replace `oodle_loader` and the `oodle` feature with an unconditional `oozextract` dependency; use workspace package metadata |

Paldex reads existing paks; it does not write them.
