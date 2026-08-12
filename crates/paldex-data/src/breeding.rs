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

use std::collections::{HashMap, HashSet};

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
    /// Rows carrying `IgnoreCombi`, as (lookup key, `CharacterID`). Resolved
    /// in [`BreedingIndex::finish`] once the unique table is complete — see
    /// there for why the decision cannot be made as rows arrive.
    ignored: Vec<(String, String)>,
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
        self.parents.insert(
            key.to_owned(),
            Parent {
                tribe: tribe.to_owned(),
                rank,
            },
        );

        // `IgnoreCombi` keeps a species out of the *candidate pool* — the set
        // the generic `CombiRank` rule can land on — and nothing more. It is
        // emphatically not a "cannot breed" flag: Frostallion, Jetragon,
        // Paladius, Necromus and Bellanoir all carry it and all breed. What it
        // means is that no arbitrary pairing produces them; they come only
        // from a `DT_PalCombiUnique` row. So they stay in `parents`, and a
        // caller asking what two of them make gets the generic answer, which
        // is the game's answer too.
        if !is_pal || ignore_combi {
            self.ignored.push((key.to_owned(), character_id.to_owned()));
            return;
        }
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

    /// Sort the candidate pool and drop the species the game gives no breeding
    /// route to. Call once after all rows are added.
    ///
    /// `IgnoreCombi` on its own means only "the generic rule never lands on
    /// this" — Frostallion, Jetragon, Paladius, Necromus and Bellanoir all
    /// carry it and all breed, because a `DT_PalCombiUnique` row names each of
    /// them. A species carrying it with *no* unique combo naming it is a
    /// different thing: nothing produces it, and it is not a farm parent
    /// either. In the shipped data that is Panthalus, Astralym and the two
    /// Yakushima raid bosses.
    ///
    /// The "not a farm parent" half is an inference from the absence of a
    /// unique combo, not something any `DataTable` states, so it was confirmed
    /// in-game: Panthalus cannot be assigned to a breeding farm or a breeding
    /// lab at all.
    ///
    /// This has to happen here rather than as rows arrive, because whether a
    /// unique combo names a species is not known until every row has been read.
    pub(crate) fn finish(&mut self) {
        self.pool
            .sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.tribe.cmp(&b.tribe)));

        // Normalized to lookup keys, not raw ids: several rows share one key
        // (`BOSS_BlackCentaur` and `BlackCentaur` are both `blackcentaur`), and
        // comparing raw ids removed Necromus because its *alpha* row is not
        // itself a breeding result.
        let producible: HashSet<String> = self
            .pool
            .iter()
            .map(|c| crate::extract::normalize_key(&c.character_id))
            .chain(
                self.unique
                    .values()
                    .map(|v| crate::extract::normalize_key(v)),
            )
            .collect();
        for (key, _) in std::mem::take(&mut self.ignored) {
            if !producible.contains(&crate::extract::normalize_key(&key)) {
                self.parents.remove(&key);
            }
        }
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
        // Two `IgnoreCombi` species, which differ only in whether a unique
        // combo names them — the distinction `finish` acts on.
        //
        // `X` has none, so nothing produces it and it is not a parent either:
        // the shipped data's Panthalus and Astralym.
        add(&mut b, "x", "X", "TribeX", 130, true, 40);
        // `Y` is named by one, so it breeds true and is a perfectly good
        // parent: the shipped data's Frostallion, Jetragon and friends.
        add(&mut b, "y", "Y", "TribeY", 140, true, 50);
        b.add_unique("TribeY", "TribeY", "Y");
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

    /// An `IgnoreCombi` species is still a parent — the flag only keeps it out
    /// of the generic candidate pool.
    ///
    /// It is emphatically not "cannot breed". Frostallion, Jetragon, Paladius,
    /// Necromus and Bellanoir all carry it in the shipped data and all breed;
    /// what they cannot be is the result of an *arbitrary* pairing. They come
    /// from a `DT_PalCombiUnique` row instead, which `child_of` consults first
    /// — see `unique_combos_override_the_generic_rule`.
    /// An `IgnoreCombi` species named by a unique combo breeds normally.
    ///
    /// This is how every legendary behaves. The flag only keeps it out of the
    /// generic candidate pool; the unique table, which `child_of` consults
    /// first, still produces it, and it stays a usable parent.
    #[test]
    fn an_ignored_species_with_a_unique_combo_still_breeds() {
        let b = index();
        assert_eq!(b.child_of("y", "y"), Some("Y"), "breeds true via its unique combo");
        assert!(b.child_of("y", "a").is_some(), "and is a parent alongside anything else");
        assert!(
            b.pool.iter().all(|c| c.character_id != "Y"),
            "even though the generic rule never lands on it"
        );
    }

    /// An `IgnoreCombi` species with no unique combo has no breeding route at
    /// all, so it is dropped as a parent too.
    ///
    /// Answering with the nearest rank would read as a working pairing when the
    /// game offers none.
    #[test]
    fn an_ignored_species_with_no_unique_combo_does_not_breed() {
        let b = index();
        assert_eq!(b.child_of("x", "x"), None, "not a parent");
        assert_eq!(b.child_of("x", "a"), None, "nor alongside anything else");
        assert_eq!(b.child_of("a", "x"), None, "either way round");
        assert!(b.pool.iter().all(|c| c.character_id != "X"), "and never a child");
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
