//! One decoder per known `RawData`-bearing container, dispatched by the
//! plan's phase-2 table. Only `character` is implemented so far; the rest
//! (`GroupSaveDataMap`, `CharacterContainerSaveData`, `ItemContainerSaveData`,
//! `DynamicItemSaveData`, `BaseCampSaveData`, `WorkSaveData`,
//! `MapObjectSaveData`) are follow-up work. `FoliageGridSaveDataMap` is
//! deliberately never decoded — large, no tracker value.

pub mod character;
pub mod guild;
pub mod player;
