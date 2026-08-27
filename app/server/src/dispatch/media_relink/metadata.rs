//! Private, conservative metadata comparison for bulk-relink review hints.
//!
//! These facts stay inside plan construction. Preview callers receive only a
//! disposition and safe names for facts that matched, never candidate paths,
//! raw ffprobe output, numeric values, ranks, or filesystem timestamps.

use super::*;
use cut_media::probe::kinds;

mod facts;
#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;

use facts::ProbeFacts;
pub(super) use facts::{candidate_metadata, stored_metadata};

const FACT_BASENAME: &str = "basename_match";
const FACT_KIND: &str = "kind_match";
const FACT_SIZE: &str = "byte_size_match";
const FACT_DURATION: &str = "duration_match";
const FACT_DIMENSIONS: &str = "dimensions_match";
const FACT_FORMAT: &str = "format_match";
const FACT_CODECS: &str = "codecs_match";

#[derive(Debug, Clone)]
pub(super) struct StoredMetadata {
    probe_available: bool,
    raw_size: Option<u64>,
    facts: ProbeFacts,
}

#[derive(Debug, Clone)]
pub(super) struct CandidateMetadata {
    bytes: u64,
    facts: Option<ProbeFacts>,
}

#[derive(Debug, Clone)]
pub(super) struct MetadataAssessment {
    pub(super) disposition: &'static str,
    pub(super) diagnostics: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
struct MatchFacts {
    basename: bool,
    kind: bool,
    size: bool,
    duration: bool,
    dimensions: bool,
    format: bool,
    codecs: bool,
}

impl MatchFacts {
    /// This rank is private ordering evidence only. A high partial score never
    /// admits a review row; `is_strong` separately enforces every requirement.
    fn rank(self) -> u16 {
        u16::from(self.basename) * 64
            + u16::from(self.kind) * 32
            + u16::from(self.size) * 16
            + u16::from(self.duration) * 8
            + u16::from(self.dimensions) * 4
            + u16::from(self.format) * 2
            + u16::from(self.codecs) * 2
    }

    fn labels(self) -> Vec<String> {
        [
            (self.basename, FACT_BASENAME),
            (self.kind, FACT_KIND),
            (self.size, FACT_SIZE),
            (self.duration, FACT_DURATION),
            (self.dimensions, FACT_DIMENSIONS),
            (self.format, FACT_FORMAT),
            (self.codecs, FACT_CODECS),
        ]
        .into_iter()
        .filter(|(matched, _)| *matched)
        .map(|(_, label)| label.to_owned())
        .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comparison {
    Match,
    Mismatch,
    Unavailable,
}

#[derive(Debug)]
struct CandidateAssessment<'a> {
    candidate: &'a Candidate,
    facts: MatchFacts,
    dimensions: Comparison,
    format: Comparison,
    codecs: Comparison,
    is_strong: bool,
}

/// Return a human-review disposition without exposing candidate data. Every
/// candidate considered here has the exact stored basename and media kind.
pub(super) fn assess(asset: &OfflineAssetSnapshot, candidates: &[Candidate]) -> MetadataAssessment {
    let stored = &asset.metadata;
    if !stored.probe_available || stored.raw_size.is_none() || !has_required_source_facts(stored) {
        return MetadataAssessment {
            disposition: "metadata_insufficient",
            diagnostics: Vec::new(),
        };
    }

    let mut assessed = candidates
        .iter()
        .filter_map(|candidate| assess_candidate(asset, stored, candidate))
        .collect::<Vec<_>>();
    if assessed.is_empty() {
        return MetadataAssessment {
            disposition: "no_match",
            diagnostics: Vec::new(),
        };
    }

    // Stable path ordering makes private diagnostics deterministic, while tied
    // top scores remain honestly ambiguous rather than choosing a path.
    assessed.sort_by(|left, right| {
        right
            .facts
            .rank()
            .cmp(&left.facts.rank())
            .then_with(|| left.candidate.path.cmp(&right.candidate.path))
    });
    let top_rank = assessed[0].facts.rank();
    let tied = assessed
        .iter()
        .filter(|candidate| candidate.facts.rank() == top_rank)
        .collect::<Vec<_>>();
    if tied.len() > 1 {
        return MetadataAssessment {
            disposition: "ambiguous_metadata",
            diagnostics: shared_labels(&tied),
        };
    }

    let top = &assessed[0];
    if top.is_strong {
        return MetadataAssessment {
            disposition: "metadata_review",
            diagnostics: top.facts.labels(),
        };
    }
    if [top.dimensions, top.format, top.codecs]
        .into_iter()
        .any(|comparison| comparison == Comparison::Mismatch)
    {
        return MetadataAssessment {
            disposition: "metadata_mismatch",
            diagnostics: top.facts.labels(),
        };
    }
    MetadataAssessment {
        disposition: "no_match",
        diagnostics: top.facts.labels(),
    }
}

fn has_required_source_facts(stored: &StoredMetadata) -> bool {
    let Some(kind) = stored.facts.kind.as_deref() else {
        return false;
    };
    match kind {
        kinds::AUDIO | kinds::VIDEO => stored.facts.duration_ms.is_some(),
        kinds::IMAGE => dimensions_complete(&stored.facts),
        _ => false,
    }
}

fn assess_candidate<'a>(
    asset: &OfflineAssetSnapshot,
    stored: &StoredMetadata,
    candidate: &'a Candidate,
) -> Option<CandidateAssessment<'a>> {
    let candidate_facts = candidate.metadata.facts.as_ref()?;
    let basename = candidate.display_name == asset.display_name;
    let kind = exact_match(
        stored.facts.kind.as_deref(),
        candidate_facts.kind.as_deref(),
    );
    // The strong-review path always begins with exact basename and kind.
    if !basename || !kind {
        return None;
    }
    let size = stored.raw_size == Some(candidate.metadata.bytes);
    let duration = exact_match(
        stored.facts.duration_ms.as_ref(),
        candidate_facts.duration_ms.as_ref(),
    );
    let dimensions = compare_dimensions(&stored.facts, candidate_facts);
    let format = compare_optional(
        stored.facts.format.as_deref(),
        candidate_facts.format.as_deref(),
    );
    let codecs = compare_optional(
        stored.facts.codecs.as_deref(),
        candidate_facts.codecs.as_deref(),
    );
    let facts = MatchFacts {
        basename,
        kind,
        size,
        duration,
        dimensions: dimensions == Comparison::Match,
        format: format == Comparison::Match,
        codecs: codecs == Comparison::Match,
    };
    let is_still = stored.facts.kind.as_deref() == Some(kinds::IMAGE);
    let timing_or_geometry = if is_still {
        dimensions == Comparison::Match
    } else {
        duration
    };
    let has_conflict = [dimensions, format, codecs]
        .into_iter()
        .any(|comparison| comparison == Comparison::Mismatch);
    Some(CandidateAssessment {
        candidate,
        facts,
        dimensions,
        format,
        codecs,
        is_strong: size && timing_or_geometry && !has_conflict,
    })
}

fn shared_labels(candidates: &[&CandidateAssessment<'_>]) -> Vec<String> {
    let shared = MatchFacts {
        basename: candidates.iter().all(|candidate| candidate.facts.basename),
        kind: candidates.iter().all(|candidate| candidate.facts.kind),
        size: candidates.iter().all(|candidate| candidate.facts.size),
        duration: candidates.iter().all(|candidate| candidate.facts.duration),
        dimensions: candidates
            .iter()
            .all(|candidate| candidate.facts.dimensions),
        format: candidates.iter().all(|candidate| candidate.facts.format),
        codecs: candidates.iter().all(|candidate| candidate.facts.codecs),
    };
    shared.labels()
}

fn exact_match<T: Eq + ?Sized>(left: Option<&T>, right: Option<&T>) -> bool {
    matches!((left, right), (Some(left), Some(right)) if left == right)
}

fn compare_optional(left: Option<&str>, right: Option<&str>) -> Comparison {
    match (left, right) {
        (Some(left), Some(right)) if left == right => Comparison::Match,
        (Some(_), Some(_)) => Comparison::Mismatch,
        _ => Comparison::Unavailable,
    }
}

fn compare_dimensions(left: &ProbeFacts, right: &ProbeFacts) -> Comparison {
    let pairs = [(left.width, right.width), (left.height, right.height)];
    if pairs
        .into_iter()
        .any(|(left, right)| matches!((left, right), (Some(left), Some(right)) if left != right))
    {
        return Comparison::Mismatch;
    }
    if dimensions_complete(left) && dimensions_complete(right) {
        Comparison::Match
    } else {
        Comparison::Unavailable
    }
}

fn dimensions_complete(facts: &ProbeFacts) -> bool {
    facts.width.is_some() && facts.height.is_some()
}
