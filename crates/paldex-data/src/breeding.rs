//! Palworld's breeding rules, built from `DT_PalMonsterParameter` and
//! `DT_PalCombiUnique`.
//!
//! Two rules, in order:
//!
//! 1. **Unique combos.** `DT_PalCombiUnique` lists 258 specific parent pairings
//!    that produce a specific child — Relaxaurus + Sparkit gives Relaxaurus
//!    Lux, and so on. These override everything else.
//! 2. **The generic rule.** Otherwise the child is the species whose
//!    `CombiRank` sits closest to `floor((rankA + rankB + 1) / 2)`.
//!
//! ## Everything here keys on *tribe*, not species
//!
//! `DT_PalCombiUnique` identifies parents by `EPalTribeID`, and the parameter
//! table's own data agrees: `SheepBall` and `Quest_Farmer03_SheepBall` are
//! byte-identical in every breeding field and share tribe `SheepBall`. So the
//! candidate pool holds exactly one entry per tribe — 261 of them — which is
//! also what removes the duplicate ranks that a per-row pool suffers from.
//!
//! Verified: with a per-tribe pool there are **zero** `CombiRank` collisions,
//! and `X + X == X` holds for all 261 tribes, which pins the target formula.
//!
//! ## Ties resolve by the lower Paldeck number
//!
//! Every `CombiRank` is a multiple of 10, so whenever `rankA + rankB` is not a
//! multiple of 20 the target lands exactly halfway between two candidates and
//! *both* are equally close. That happens for **46% of pairs**, so the rule
//! matters a great deal — and the pak does not contain it.
//! `DT_PalCombiUnique` is the only breeding table; the resolution lives in the
//! game's compiled code.
//!
//! It was therefore settled empirically, by breeding pairs chosen so that the
//! competing rules predict different children:
//!
//! | pair | target | result | eliminates |
//! | --- | --- | --- | --- |
//! | Lamball + Chikipi | 3065 | **Vixy** (3060, Paldeck 6) | highest `CombiRank` and lowest row index, which both predict Teafant (3070, Paldeck 11, row 362 vs 436) |
//! | Lamball + Fuack | 3015 | **Lifmunk** (3020, Paldeck 4) | lowest `CombiRank`, which predicts Sparkit (3010, Paldeck 42) |
//!
//! Lifmunk is the *higher*-ranked of its tied pair and Vixy the lower, so no
//! rank-ordering rule explains both. The lower `ZukanIndex` wins in each case,
//! and that is what [`BreedingIndex::child_of`] implements.
//!
//! Note this only makes sense because a tie is always between exactly two
//! candidates: the per-tribe pool has no duplicate ranks, so the two tied
//! entries sit one step either side of the target.

use std::collections::HashMap;

/// A species eligible to be produced by the generic rule.
#[derive(Debug, Clone)]
struct Candidate {
    rank: u32,
    /// Paldeck number, which is what resolves a tie. Zero for a species with
    /// no Paldeck entry, which sorts it last rather than first.
    zukan: u32,
    tribe: String,
    character_id: String,
}

/// One parent's breeding-relevant parameters.
#[derive(Debug, Clone)]
struct Parent {
    tribe: String,
    rank: u32,
}

/// The breeding-relevant fields of one `DT_PalMonsterParameter` row.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SpeciesRow<'a> {
    /// The caller's normalized lookup key.
    pub key: &'a str,
    /// The raw row name — what a unique combo names as its child.
    pub character_id: &'a str,
    pub tribe: &'a str,
    pub rank: u32,
    pub is_pal: bool,
    /// Excluded from being *produced* by the generic rule, though still
    /// usable as a parent.
    pub ignore_combi: bool,
    /// Paldeck number, or zero when the species has no Paldeck entry.
    pub zukan: u32,
}

/// Breeding lookups over the whole species set.
#[derive(Debug, Clone, Default)]
pub struct BreedingIndex {
    /// Species lookup key -> its tribe and combi rank.
    parents: HashMap<String, Parent>,
    /// Lowercased, order-independent tribe pair -> child `CharacterID`.
    unique: HashMap<(String, String), String>,
    /// One entry per tribe, sorted by rank.
    pool: Vec<Candidate>,
}

/// A `CombiRank` at or above this marks a species the generic rule never
/// produces; the shipped data uses it as a sentinel.
const RANK_SENTINEL: u32 = 9999;

fn tribe_pair(a: &str, b: &str) -> (String, String) {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

impl BreedingIndex {
    /// Register a species from a `DT_PalMonsterParameter` row.
    pub(crate) fn add_species(&mut self, row: &SpeciesRow<'_>) {
        let &SpeciesRow {
            key,
            character_id,
            tribe,
            rank,
            is_pal,
            ignore_combi,
            zukan,
        } = row;
        if tribe.is_empty() || rank == 0 || rank >= RANK_SENTINEL {
            return;
        }
        // `IgnoreCombi` is the game's own "this one does not breed" flag, and
        // a non-Pal cannot enter a farm either. Such a row is not a parent at
        // all, so it is kept out of `parents` entirely rather than merely out
        // of the candidate pool: leaving it in made `child_of` answer for
        // Panthalus and Astralym, and the answer was nearest-rank noise — a
        // species the pair cannot actually produce. Callers read a `Some` as
        // "this pairing works", so the only honest reply here is `None`.
        if !is_pal || ignore_combi {
            return;
        }
        self.parents.insert(
            key.to_owned(),
            Parent {
                tribe: tribe.to_owned(),
                rank,
            },
        );
        // One candidate per tribe. The canonical member is the row named after
        // the tribe; a Paldeck number breaks any remaining tie. Without this,
        // quest duplicates (`Quest_Farmer03_SheepBall`) and unique-combo-only
        // variants (`PlantSlime_Flower`) enter the pool and collide on rank
        // with the species they shadow.
        let score = (character_id.eq_ignore_ascii_case(tribe), zukan > 0);
        match self
            .pool
            .iter_mut()
            .find(|c| c.tribe.eq_ignore_ascii_case(tribe))
        {
            Some(existing) => {
                let current = (
                    existing.character_id.eq_ignore_ascii_case(tribe),
                    existing.zukan > 0,
                );
                if score > current {
                    existing.rank = rank;
                    existing.zukan = zukan;
                    existing.character_id = character_id.to_owned();
                }
            }
            None => self.pool.push(Candidate {
                rank,
                zukan,
                tribe: tribe.to_owned(),
                character_id: character_id.to_owned(),
            }),
        }
    }

    /// Register a `DT_PalCombiUnique` row.
    pub(crate) fn add_unique(&mut self, tribe_a: &str, tribe_b: &str, child: &str) {
        if tribe_a.is_empty() || tribe_b.is_empty() || child.is_empty() {
            return;
        }
        self.unique
            .insert(tribe_pair(tribe_a, tribe_b), child.to_owned());
    }

    /// Sort the candidate pool. Call once after all rows are added.
    pub(crate) fn finish(&mut self) {
        self.pool
            .sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.tribe.cmp(&b.tribe)));
    }

    #[must_use]
    pub fn candidate_count(&self) -> usize {
        self.pool.len()
    }

    #[must_use]
    pub fn unique_combo_count(&self) -> usize {
        self.unique.len()
    }

    /// The child of two parents, as a `CharacterID`.
    ///
    /// Both arguments are normalized lookup keys. Returns `None` when either
    /// parent is not a breedable species.
    #[must_use]
    pub fn child_of(&self, a: &str, b: &str) -> Option<&str> {
        let pa = self.parents.get(a)?;
        let pb = self.parents.get(b)?;

        if let Some(child) = self.unique.get(&tribe_pair(&pa.tribe, &pb.tribe)) {
            return Some(child);
        }

        // The game's formula is `floor((a + b + 1) / 2)`, i.e. the average
        // rounded up. Every shipped rank is a multiple of 10, so the rounding
        // is a no-op today, but it is kept because a patch may add odd ranks.
        let target = (pa.rank + pb.rank).div_ceil(2);

        let best = self.pool.iter().map(|c| c.rank.abs_diff(target)).min()?;

        // Ties go to the lower Paldeck number — see the module docs for the
        // two in-game results that establish this. A species with no Paldeck
        // entry sorts last rather than first, since zero would otherwise beat
        // every real number.
        self.pool
            .iter()
            .filter(|c| c.rank.abs_diff(target) == best)
            .min_by_key(|c| if c.zukan == 0 { u32::MAX } else { c.zukan })
            .map(|c| c.character_id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index() -> BreedingIndex {
        let mut b = BreedingIndex::default();
        // Ranks 10 apart so a mismatched pair lands exactly between two.
        fn add(
            b: &mut BreedingIndex,
            key: &str,
            id: &str,
            tribe: &str,
            rank: u32,
            ignore_combi: bool,
            zukan: u32,
        ) {
            b.add_species(&SpeciesRow {
                key,
                character_id: id,
                tribe,
                rank,
                is_pal: true,
                ignore_combi,
                zukan,
            });
        }
        // Paldeck numbers deliberately run opposite to rank so the two
        // tie-break rules are distinguishable here.
        add(&mut b, "a", "A", "TribeA", 100, false, 20);
        add(&mut b, "b", "B", "TribeB", 110, false, 10);
        add(&mut b, "c", "C", "TribeC", 120, false, 30);
        // A quest duplicate of TribeA must not enter the pool.
        add(&mut b, "quest_a", "Quest_A", "TribeA", 100, false, 0);
        // `IgnoreCombi`: the game does not breed this one at all — neither a
        // parent nor a possible child.
        add(&mut b, "x", "X", "TribeX", 130, true, 40);
        b.finish();
        b
    }

    #[test]
    fn one_candidate_per_tribe() {
        let b = index();
        assert_eq!(
            b.candidate_count(),
            3,
            "TribeX is ignored, Quest_A is deduped"
        );
    }

    /// The invariant that pins the target formula against the real data.
    #[test]
    fn self_breeding_is_a_fixed_point() {
        let b = index();
        for (key, want) in [("a", "A"), ("b", "B"), ("c", "C")] {
            assert_eq!(b.child_of(key, key), Some(want));
        }
    }

    #[test]
    fn exact_midpoint_resolves_to_the_middle_candidate() {
        let b = index();
        // (100 + 120 + 1) / 2 = 110 — an exact rank, so no tie.
        assert_eq!(b.child_of("a", "c"), Some("B"));
    }

    #[test]
    fn a_tie_goes_to_the_lower_paldeck_number() {
        let b = index();
        // (100 + 110 + 1) / 2 = 105 — equidistant from 100 and 110. A has the
        // lower rank but B has the lower Paldeck number, so B must win.
        assert_eq!(b.child_of("a", "b"), Some("B"));
    }

    #[test]
    fn unique_combos_override_the_generic_rule() {
        let mut b = index();
        b.add_unique("TribeA", "TribeB", "SpecialChild");
        assert_eq!(b.child_of("a", "b"), Some("SpecialChild"));
        // Order must not matter.
        assert_eq!(b.child_of("b", "a"), Some("SpecialChild"));
    }

    /// An `IgnoreCombi` species does not breed at all.
    ///
    /// It is neither a candidate child nor a usable parent, so every lookup
    /// naming it answers `None`. Answering with the nearest rank instead would
    /// read as a working pairing — the caller cannot tell a real result from a
    /// fallback — and the four species this covers in the shipped data
    /// (Panthalus, Astralym and the two Yakushima raid bosses) genuinely
    /// cannot be put in a farm.
    #[test]
    fn an_ignored_species_does_not_breed_at_all() {
        let b = index();
        assert_eq!(b.child_of("x", "x"), None, "not a parent");
        assert_eq!(b.child_of("x", "a"), None, "not a parent alongside anything else");
        assert_eq!(b.child_of("a", "x"), None, "either way round");
        assert!(
            b.pool.iter().all(|c| c.character_id != "X"),
            "and never a child"
        );
    }

    /// A species merely *shadowed* by its tribe's representative is different:
    /// nothing breeds into it, but it breeds perfectly well itself.
    ///
    /// `Quest_A` stands in for the real `PlantSlime_Flower`, whose pairings
    /// yield the base species. That is why "cannot be bred into" and "cannot
    /// breed" have to be judged separately.
    #[test]
    fn a_shadowed_species_is_still_a_parent() {
        let b = index();
        assert_eq!(
            b.child_of("quest_a", "quest_a"),
            Some("A"),
            "it breeds, and a pair of them gives the species it shadows"
        );
        assert!(
            b.pool.iter().all(|c| c.character_id != "Quest_A"),
            "but nothing produces it"
        );
    }

    #[test]
    fn unknown_parents_yield_nothing() {
        let b = index();
        assert_eq!(b.child_of("a", "nope"), None);
        assert_eq!(b.child_of("nope", "nope"), None);
    }
}
