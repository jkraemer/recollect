//! Reciprocal rank fusion and recency decay, as pure functions over memory ids.

use std::collections::HashMap;
use std::f64::consts::LN_2;

use crate::config::RecencyConfig;

pub const RRF_K: f64 = 60.0;
pub const FTS_WEIGHT: f64 = 0.6;
pub const VECTOR_WEIGHT: f64 = 0.4;

/// Fuses two best-first id lists: `score = Σ weight / (RRF_K + rank)`, ranks from 1.
pub fn rrf_merge(fts: &[i64], vector: &[i64]) -> Vec<(i64, f64)> {
    let mut scores: HashMap<i64, f64> = HashMap::new();
    for (list, weight) in [(fts, FTS_WEIGHT), (vector, VECTOR_WEIGHT)] {
        for (index, id) in list.iter().enumerate() {
            *scores.entry(*id).or_insert(0.0) += weight / (RRF_K + (index + 1) as f64);
        }
    }
    let mut merged: Vec<(i64, f64)> = scores.into_iter().collect();
    sort_best_first(&mut merged);
    merged
}

/// `1 − aging_factor + aging_factor · e^(−ln 2 · age / half_life)`, with the
/// age clamped at 0 and the half-life floored at 0.1 days.
pub fn recency_factor(aging_factor: f64, half_life_days: f64, age_days: f64) -> f64 {
    let decay = (-LN_2 * age_days.max(0.0) / half_life_days.max(0.1)).exp();
    1.0 - aging_factor + aging_factor * decay
}

/// Scales each score by its memory's recency factor and re-sorts; ids missing
/// from `ages` count as new.
pub fn apply_recency(
    scored: Vec<(i64, f64)>,
    ages: &HashMap<i64, f64>,
    recency: RecencyConfig,
) -> Vec<(i64, f64)> {
    if recency.aging_factor <= 0.0 {
        return scored;
    }
    let mut adjusted: Vec<(i64, f64)> = scored
        .into_iter()
        .map(|(id, score)| {
            let age = ages.get(&id).copied().unwrap_or(0.0);
            (
                id,
                score * recency_factor(recency.aging_factor, recency.half_life_days, age),
            )
        })
        .collect();
    sort_best_first(&mut adjusted);
    adjusted
}

/// Highest score first; equal scores put the newer (higher) id first.
fn sort_best_first(scored: &mut [(i64, f64)]) {
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(scored: &[(i64, f64)]) -> Vec<i64> {
        scored.iter().map(|(id, _)| *id).collect()
    }

    #[test]
    fn a_single_list_keeps_its_order() {
        let merged = rrf_merge(&[3, 1, 2], &[]);
        assert_eq!(ids(&merged), [3, 1, 2]);
        assert!((merged[0].1 - 0.6 / 61.0).abs() < 1e-12);
    }

    #[test]
    fn appearing_in_both_lists_beats_one_list() {
        assert_eq!(ids(&rrf_merge(&[1, 2], &[2, 3])), [2, 1, 3]);
    }

    #[test]
    fn full_text_outweighs_vectors_at_equal_rank() {
        assert_eq!(ids(&rrf_merge(&[1], &[2])), [1, 2]);
    }

    #[test]
    fn equal_scores_prefer_the_newer_id() {
        assert!(rrf_merge(&[], &[]).is_empty());
        let merged = rrf_merge(&[5], &[]);
        let mut tied = merged.clone();
        tied.push((9, merged[0].1));
        let resorted = apply_recency(
            tied,
            &HashMap::new(),
            RecencyConfig {
                aging_factor: 0.5,
                half_life_days: 30.0,
            },
        );
        assert_eq!(ids(&resorted), [9, 5]);
    }

    #[test]
    fn recency_factor_follows_the_half_life() {
        assert_eq!(recency_factor(0.0, 30.0, 100.0), 1.0);
        assert!((recency_factor(1.0, 30.0, 30.0) - 0.5).abs() < 1e-12);
        assert!((recency_factor(0.5, 30.0, 30.0) - 0.75).abs() < 1e-12);
        assert_eq!(
            recency_factor(1.0, 30.0, -5.0),
            1.0,
            "future timestamps count as new"
        );
        assert!(
            recency_factor(1.0, 0.0, 1.0) < 1e-3,
            "half-life is floored at 0.1 days"
        );
    }

    #[test]
    fn recency_reorders_old_results_below_new_ones() {
        let scored = vec![(1, 0.02), (2, 0.019)];
        let ages = HashMap::from([(1, 90.0), (2, 0.0)]);
        let on = RecencyConfig {
            aging_factor: 1.0,
            half_life_days: 30.0,
        };
        assert_eq!(ids(&apply_recency(scored.clone(), &ages, on)), [2, 1]);
        let off = RecencyConfig::default();
        assert_eq!(ids(&apply_recency(scored, &ages, off)), [1, 2]);
    }
}
