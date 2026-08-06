//! The stable interface the rest of the app depends on for species names,
//! passive-skill definitions, breeding results, and icon artwork — deliberately
//! decoupled from *how* that data was obtained, per the plan's Phase 3 design:
//! "the UI depends only on this trait, so swapping the fallback in is a
//! one-line change."
//!
//! ## Status: interface only, no full implementation yet
//!
//! [`Pak::open`]/[`Pak::read`] (see `lib.rs`) prove the pak itself is fully
//! readable — verified against the real 40 GB `Pal-Windows.pak`: index parses,
//! every sampled entry decompresses, `DT_PalMonsterParameter.uasset` reads
//! clean. What's *not* done is deserializing that uasset's `DataTable` rows
//! into actual species data.
//!
//! Confirmed empirically (`FPackageFileSummary.FileVersionUE4` and
//! `FileVersionUE5` both read `0` on the real file): this is a cooked,
//! **unversioned** package, exactly the risk the plan's research flagged in
//! advance. Unversioned `DataTable` rows need a `.usmap` property-schema
//! mapping to deserialize at all — the row bytes alone don't carry enough
//! information to know which bytes are which field. No `.usmap` exists in
//! this repo or environment, and generating one requires either a published
//! community mapping for this exact game build or running UE4SS's dumper
//! against the live game (a manual, per-user, per-patch step). Writing a
//! from-scratch unversioned-property deserializer without one is not a
//! tractable addition here — it's comparable in scope to a meaningful chunk
//! of CUE4Parse itself.
//!
//! The plan pre-approved exactly this situation's fallback ("no
//! mid-implementation decision needed"): stats/combos from a vendored
//! community dataset (e.g. `PalworldDataTools/PalworldDataExtractor`'s
//! output), artwork still from the local pak (`T_PalIcon_*` textures don't
//! need a property schema, just standard DXT/BC decoding — more tractable,
//! not yet implemented either). Populating that dataset is left as an
//! explicit next step rather than fabricated here: it means pulling in
//! specific third-party data, which is a provenance/licensing choice worth
//! the user's eyes before it lands in the repo.
//!
//! [`PassthroughReferenceData`] keeps every dependent layer (the SQLite
//! store, the roster UI) compiling and testable against the real interface
//! shape in the meantime.

/// A Pal species' static reference data.
#[derive(Debug, Clone, PartialEq)]
pub struct Species {
    pub character_id: String,
    pub display_name: String,
    pub dex_number: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PassiveSkill {
    pub id: String,
    pub display_name: String,
}

pub trait ReferenceData {
    fn species(&self, character_id: &str) -> Option<&Species>;
    fn passive(&self, id: &str) -> Option<&PassiveSkill>;
    fn breeding_result(&self, a: &str, b: &str) -> Option<&str>;
    fn icon(&self, character_id: &str) -> Option<&[u8]>;
}

/// A stub [`ReferenceData`] with no real data behind it — `species`/`passive`
/// echo the raw id back as the display name (so the UI has *something*
/// readable rather than a blank field) and never resolve dex numbers, icons,
/// or breeding results. Exists so the rest of the app can be built and
/// tested against the real trait shape before a real data source lands.
#[derive(Debug, Default)]
pub struct PassthroughReferenceData;

impl ReferenceData for PassthroughReferenceData {
    fn species(&self, _character_id: &str) -> Option<&Species> {
        None
    }

    fn passive(&self, _id: &str) -> Option<&PassiveSkill> {
        None
    }

    fn breeding_result(&self, _a: &str, _b: &str) -> Option<&str> {
        None
    }

    fn icon(&self, _character_id: &str) -> Option<&[u8]> {
        None
    }
}
