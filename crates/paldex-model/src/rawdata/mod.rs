//! Decoders for Palworld-specific data embedded in GVAS containers.
//!
//! - `character`: `CharacterSaveParameterMap` nested tagged-property lists.
//! - `player`: standalone player GVAS records.
//! - `guild`: `GroupSaveDataMap` metadata and best-effort binary id extraction.
//! - `base_camp`: `BaseCampSaveData` guild and worker-container ids.
//!
//! Character/item containers, dynamic items, work state, map objects, and
//! foliage are not domain-decoded.

pub mod base_camp;
pub mod character;
pub mod guild;
pub mod player;
