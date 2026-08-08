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
//! ## The one unverified degree of freedom
//!
//! Every `CombiRank` is a multiple of 10, so whenever `rankA + rankB` is not a
//! multiple of 20 the target lands exactly halfway between two candidates and
//! *both* are equally close. This happens for **46% of pairs**, and the pak
//! does not say which side wins: `DT_PalCombiUnique` is the only breeding
//! table, so the resolution lives in the game's compiled code.
//!
//! [`TIE_PREFERS_HIGHER_RANK`] encodes the choice. It defaults to the higher
//! rank because the target formula's own `+1` expresses a round-half-up
//! intent — but that is an inference, not a measurement. See its docs for the
//! single in-game check that settles it.

use std::collections::HashMap;

/// On a tie, prefer the candidate with the **higher** `CombiRank`.
///
/// Unverified — see the module docs. To settle it, breed **Lamball + Cattiva**
/// in game, a pair with no unique-combo override:
///
/// - `true` (current) predicts **Depresso**
/// - `false` predicts **Tanzee**
///
/// Flip this constant if the game disagrees; `breeding_lamball_cattiva` in
/// `tests/real_breeding.rs` pins whichever answer is correct.
pub const TIE_PREFERS_HIGHER_RANK: bool = true;

/// A species eligible to be produced by the generic rule.
#[derive(Debug, Clone)]
struct Candidate {
    rank: u32,
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
    pub has_dex_number: bool,
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
            has_dex_number,
        } = row;
        if tribe.is_empty() || rank == 0 || rank >= RANK_SENTINEL {
            return;
        }
        self.parents.insert(
            key.to_owned(),
            Parent {
                tribe: tribe.to_owned(),
                rank,
            },
        );

        if !is_pal || ignore_combi {
            return;
        }
        // One candidate per tribe. The canonical member is the row named after
        // the tribe; a Paldeck number breaks any remaining tie. Without this,
        // quest duplicates (`Quest_Farmer03_SheepBall`) and unique-combo-only
        // variants (`PlantSlime_Flower`) enter the pool and collide on rank
        // with the species they shadow.
        let score = (character_id.eq_ignore_ascii_case(tribe), has_dex_number);
        match self
            .pool
            .iter_mut()
            .find(|c| c.tribe.eq_ignore_ascii_case(tribe))
        {
            Some(existing) => {
                let current = (
                    existing.character_id.eq_ignore_ascii_case(tribe),
                    existing.rank > 0,
                );
                if score > current {
                    existing.rank = rank;
                    existing.character_id = character_id.to_owned();
                }
            }
            None => self.pool.push(Candidate {
                rank,
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
        let mut tied = self.pool.iter().filter(|c| c.rank.abs_diff(target) == best);
        let chosen = if TIE_PREFERS_HIGHER_RANK {
            tied.next_back()
        } else {
            tied.next()
        };
        chosen.map(|c| c.character_id.as_str())
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
            has_dex: bool,
        ) {
            b.add_species(&SpeciesRow {
                key,
                character_id: id,
                tribe,
                rank,
                is_pal: true,
                ignore_combi,
                has_dex_number: has_dex,
            });
        }
        add(&mut b, "a", "A", "TribeA", 100, false, true);
        add(&mut b, "b", "B", "TribeB", 110, false, true);
        add(&mut b, "c", "C", "TribeC", 120, false, true);
        // A quest duplicate of TribeA must not enter the pool.
        add(&mut b, "quest_a", "Quest_A", "TribeA", 100, false, false);
        // Excluded from being a child, but still usable as a parent.
        add(&mut b, "x", "X", "TribeX", 130, true, true);
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
    fn a_tie_follows_the_documented_preference() {
        let b = index();
        // (100 + 110 + 1) / 2 = 105 — equidistant from 100 and 110.
        let expected = if TIE_PREFERS_HIGHER_RANK { "B" } else { "A" };
        assert_eq!(b.child_of("a", "b"), Some(expected));
    }

    #[test]
    fn unique_combos_override_the_generic_rule() {
        let mut b = index();
        b.add_unique("TribeA", "TribeB", "SpecialChild");
        assert_eq!(b.child_of("a", "b"), Some("SpecialChild"));
        // Order must not matter.
        assert_eq!(b.child_of("b", "a"), Some("SpecialChild"));
    }

    /// A species the generic rule never produces can still be a parent.
    #[test]
    fn an_ignored_species_still_breeds() {
        let b = index();
        assert!(b.child_of("x", "x").is_some());
        assert!(b.pool.iter().all(|c| c.character_id != "X"));
    }

    #[test]
    fn unknown_parents_yield_nothing() {
        let b = index();
        assert_eq!(b.child_of("a", "nope"), None);
        assert_eq!(b.child_of("nope", "nope"), None);
    }
}
