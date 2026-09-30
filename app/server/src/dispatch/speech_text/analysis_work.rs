//! Work bounds for transcript chaptering and exact retake classification.

use cut_core::{error_codes, CutError};

// Fifty million token-DP cells per complete removal action, across every
// comparison and asset. Ordinary short attempts need only small bands; long
// identical or nearly identical attempts are trimmed before any DP work.
// This bounds ambiguous sequence analysis, without limiting recording length,
// changing similarity thresholds, or treating unfinished analysis as no match.
const RETAKE_DP_CELLS: usize = 50_000_000;

pub(super) struct RetakeWork {
    remaining: usize,
}

impl Default for RetakeWork {
    fn default() -> Self {
        Self::new(RETAKE_DP_CELLS)
    }
}

impl RetakeWork {
    pub(super) fn new(cells: usize) -> Self {
        Self { remaining: cells }
    }

    fn spend(&mut self, cells: usize) -> Result<(), CutError> {
        self.remaining = self.remaining.checked_sub(cells).ok_or_else(|| {
            CutError::new(
                error_codes::GUARDRAIL,
                "retake analysis exceeded its work limit; no timeline edits were made",
                "the complete pass is limited to 50 million token edit-distance cells",
            )
            .with_suggested_action(
                "narrow the pass to one asset, or review and cut these attempts manually",
            )
        })?;
        Ok(())
    }
}

/// Exactly the former outward nondecreasing walks, including equal plateaus.
/// Reuse the peak already reached from the adjacent gap instead of rescanning.
pub(super) fn cohesion_depths(coh: &[f64]) -> Vec<f64> {
    if coh.is_empty() {
        return Vec::new();
    }
    let mut left = Vec::with_capacity(coh.len());
    for (k, &value) in coh.iter().enumerate() {
        left.push(if k > 0 && coh[k - 1] >= value {
            left[k - 1]
        } else {
            value
        });
    }
    let mut right = coh[coh.len() - 1];
    for k in (0..coh.len()).rev() {
        if k + 1 == coh.len() || coh[k + 1] < coh[k] {
            right = coh[k];
        }
        left[k] = (left[k] - coh[k]) + (right - coh[k]);
    }
    left
}

/// Find the greatest accepted distance using the original floating expression.
/// Multiplying (1-threshold) by length can round differently at exact cutoffs.
fn distance_limit(maxlen: usize, threshold: f64) -> usize {
    let (mut lo, mut hi) = (0, maxlen);
    while lo < hi {
        let span = hi - lo;
        let mid = lo + span / 2 + span % 2;
        if 1.0 - mid as f64 / maxlen as f64 >= threshold {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

/// Classify by the exact token Levenshtein threshold, without finding distances
/// that cannot affect the answer. Failure propagates before any ripple edits.
pub(super) fn retake_matches<'a>(
    mut a: &'a [String],
    mut b: &'a [String],
    threshold: f64,
    work: &mut RetakeWork,
) -> Result<bool, CutError> {
    let maxlen = a.len().max(b.len());
    if maxlen == 0 || threshold <= 0.0 {
        return Ok(true);
    }
    let limit = distance_limit(maxlen, threshold);
    if a.len().abs_diff(b.len()) > limit {
        return Ok(false);
    }
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    a = &a[prefix..];
    b = &b[prefix..];
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    a = &a[..a.len() - suffix];
    b = &b[..b.len() - suffix];
    if a.is_empty() || b.is_empty() {
        return Ok(a.len().max(b.len()) <= limit);
    }
    if limit == 0 {
        return Ok(false);
    }
    if a.len() < b.len() {
        std::mem::swap(&mut a, &mut b);
    }
    // The remaining distance is at most the longer residual length.
    if a.len() <= limit {
        return Ok(true);
    }
    // Exact token IDs keep each DP cell constant-cost even when hostile tokens
    // have very long common character prefixes. HashMap checks full equality;
    // hash collisions never turn unequal strings into equal tokens.
    let mut ids = std::collections::HashMap::new();
    let mut intern = |tokens: &'a [String]| {
        tokens
            .iter()
            .map(|token| {
                let next = ids.len();
                *ids.entry(token.as_str()).or_insert(next)
            })
            .collect::<Vec<usize>>()
    };
    let a_ids = intern(a);
    let b_ids = intern(b);
    // Aligned substitutions plus the trailing length difference are a valid
    // edit script. This proves a match for long, lightly changed speech without
    // exploring a wide DP band; failure of this upper bound proves nothing.
    let aligned = a_ids.iter().zip(&b_ids).filter(|(x, y)| x != y).count();
    if aligned.saturating_add(a_ids.len().abs_diff(b_ids.len())) <= limit {
        return Ok(true);
    }
    let inf = limit.saturating_add(1);
    let mut prev = vec![inf; b.len() + 1];
    let mut cur = vec![inf; b.len() + 1];
    for (j, value) in prev.iter_mut().enumerate().take(limit.saturating_add(1)) {
        *value = j;
    }
    for (row, ta) in a_ids.iter().enumerate() {
        let i = row + 1;
        let start = i.saturating_sub(limit).max(1);
        let end = i.saturating_add(limit).min(b.len());
        // Reserve precisely the cells this row will evaluate, before evaluating.
        work.spend(end - start + 1)?;
        cur[start - 1] = if start == 1 { i.min(inf) } else { inf };
        let mut minimum = cur[start - 1];
        for j in start..=end {
            cur[j] = prev[j]
                .saturating_add(1)
                .min(cur[j - 1].saturating_add(1))
                .min(prev[j - 1].saturating_add(usize::from(ta != &b_ids[j - 1])))
                .min(inf);
            minimum = minimum.min(cur[j]);
        }
        if end < b.len() {
            cur[end + 1] = inf;
        }
        if minimum > limit {
            return Ok(false);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    Ok(prev[b.len()] <= limit)
}

#[cfg(test)]
mod tests;
