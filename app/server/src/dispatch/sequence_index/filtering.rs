//! Shared status predicates for Sequence Index row projection.

#[derive(Clone, Copy)]
pub(crate) struct StatusFacts {
    pub(crate) gap: bool,
    pub(crate) offline: bool,
    pub(crate) effect_count: usize,
    pub(crate) visible: bool,
    pub(crate) locked: bool,
    pub(crate) muted: bool,
}

pub(crate) fn status_matches(status: &str, facts: StatusFacts) -> bool {
    match status {
        "all" => true,
        "issues" => facts.gap || facts.offline,
        "offline" => facts.offline,
        "gaps" => facts.gap,
        "effects" => facts.effect_count > 0,
        "hidden" => !facts.visible,
        "locked" => facts.locked,
        "muted" => facts.muted,
        _ => false,
    }
}
