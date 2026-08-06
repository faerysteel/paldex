# Vendored from trumank/repak

This crate is a vendored, modified copy of [trumank/repak](https://github.com/trumank/repak)
(commit `355b5f6`), dual-licensed MIT/Apache-2.0 (see `LICENSE-MIT` and
`LICENSE-APACHE`).

## Why vendored rather than a dependency

`repak`'s `Entry` type — which carries a file's offset, compression blocks,
and sizes inside the pak — is `pub(crate)`, so there's no way to read a pak's
raw entry metadata from outside the crate; the only public API
(`PakReader::get`/`read_file`) does decompression internally. For every
compression method except Oodle that's fine. For Oodle it isn't: `repak`'s
`oodle` feature loads the game's native `oo2core_*.dll` via `oodle_loader`,
which is Windows-only and can't be used to read Palworld's own pak on Apple
Silicon — the same constraint that shaped `paldex-sav`'s Phase 1 design (see
that crate's docs).

## What changed

- `entry.rs`: the `Oodle` decompression branch now calls
  `oozextract::Extractor` (a pure-Rust, clean-room Kraken/Mermaid/Selkie/
  Leviathan decoder, already verified against real Palworld save files in
  `paldex-sav`) instead of `oodle_loader::oodle()`.
- `data.rs`: the `Oodle` *compression* branch (used only when writing paks)
  is left unsupported — `oozextract` has no encoder, and paldex never writes
  paks, only reads the game's own.
- `Cargo.toml`: dropped the `oodle_loader` path dependency and the `oodle`
  feature; Oodle decompression is unconditional now, since it no longer
  depends on anything platform-specific.

Otherwise this is upstream `repak`'s footer/index/entry parsing, unmodified.
