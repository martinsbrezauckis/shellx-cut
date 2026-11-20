//! How many libx264 threads a render uses (items 82, 165).
//!
//! Default: libx264's own automatic count (1.5 x logical cores), the default
//! of ffmpeg, HandBrake and x264 itself. Fewer threads only when the render's
//! memory estimate with the automatic count does not fit the memory free when
//! it starts: then the largest count from a fixed ladder that fits, so a
//! render never uses fewer threads than the memory requires. The plan, the
//! progress message and the receipt say so (`reduction_sentence`).
//!
//! Why memory decides and not a fixed cap: each x264 frame thread holds a
//! full reference set (about 11 bytes per output pixel), so an 8K render needs
//! about 24 GB with 54 threads (36 cores) and about 10 GB with 12. On the
//! 36-core Mac Studio the fixed 12-thread cap of item 82 made 8K60 renders 1.9x
//! slower (15.5 s against 29.6 s for 12 s of 8K60, ffmpeg 9, the item-165
//! record) on a machine with 256 GB. Fitting the parallelism to free memory is
//! what memory-aware schedulers do (Bazel's `--local_ram_resources`, Gradle's
//! worker memory limits); x264 has no memory limit of its own.
//!
//! The thread count changes the x264 bitstream (not its quality settings).
//! The ladder keeps the possible counts few, and the receipt records the count
//! whenever it was reduced, so a reduced render can be reproduced.

/// Counts tried, largest first, when the automatic count does not fit.
pub const LADDER: &[u32] = &[32, 24, 16, 12, 8, 6, 4];

/// Environment override: treat at most this many MB as free for a render
/// (a person's own limit, and the test hook for a small machine).
pub const MEMORY_LIMIT_ENV: &str = "SHELLX_CUT_RENDER_MEMORY_MB";

pub(crate) fn logical_cores() -> u64 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u64)
        .unwrap_or(4)
}

/// libx264's own automatic thread count (x264 uses 1.5 x logical cores).
pub fn x264_auto_threads(cores: u64) -> u32 {
    (cores * 3 / 2).clamp(1, 128) as u32
}

/// The limit set by [`MEMORY_LIMIT_ENV`], when it is a positive number.
pub fn memory_limit_mb() -> Option<u64> {
    std::env::var(MEMORY_LIMIT_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
}

/// The memory a render may plan with: free memory now, lowered to the
/// person's limit when one is set.
pub fn budget_mb(available_mb: Option<u64>, limit_mb: Option<u64>) -> Option<u64> {
    match (available_mb, limit_mb) {
        (Some(a), Some(l)) => Some(a.min(l)),
        (a, l) => a.or(l),
    }
}

/// The thread decision for one render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadChoice {
    /// libx264's automatic count on this computer.
    pub automatic: u32,
    /// The count the render uses.
    pub chosen: u32,
    /// MB the render needs with the automatic count, and with `chosen`.
    pub automatic_mb: u64,
    pub chosen_mb: u64,
}

impl ThreadChoice {
    pub fn reduced(&self) -> bool {
        self.chosen < self.automatic
    }
}

/// Pick the libx264 thread count: automatic when its estimate fits `budget_mb`
/// (or the budget is unknown), else the largest ladder count that fits, else
/// the smallest. `need_mb(threads)` is the render's memory estimate.
pub fn choose(
    automatic: u32,
    budget_mb: Option<u64>,
    need_mb: impl Fn(u32) -> u64,
) -> ThreadChoice {
    let automatic_mb = need_mb(automatic);
    let fits = |mb: u64| budget_mb.is_none_or(|b| mb <= b);
    let mut choice = ThreadChoice {
        automatic,
        chosen: automatic,
        automatic_mb,
        chosen_mb: automatic_mb,
    };
    if fits(automatic_mb) {
        return choice;
    }
    for &n in LADDER.iter().filter(|&&n| n < automatic) {
        choice.chosen = n;
        choice.chosen_mb = need_mb(n);
        if fits(choice.chosen_mb) {
            break;
        }
    }
    choice
}

fn gb(mb: u64) -> String {
    format!("{:.1} GB", mb as f64 / 1024.0)
}

/// The sentence a person and an agent read when the threads were reduced
/// (progress message, plan warning, receipt `encode.memory_note`).
pub fn reduction_sentence(c: &ThreadChoice, budget_mb: u64, limited: bool) -> String {
    let (free, full_speed) = if limited {
        (
            format!(
                "the {} this render may use ({MEMORY_LIMIT_ENV})",
                gb(budget_mb)
            ),
            format!("Raise {MEMORY_LIMIT_ENV} for full speed"),
        )
    } else {
        (
            format!("the {} of memory free now", gb(budget_mb)),
            "Close other programs for full speed".to_string(),
        )
    };
    format!(
        "Software encoding uses {} of {} threads so this export fits in {free} (about {} instead \
         of {}); it will take longer. {full_speed}, or choose HEVC to encode in hardware.",
        c.chosen,
        c.automatic,
        gb(c.chosen_mb),
        gb(c.automatic_mb)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// About 25 GB at 54 threads, 10 GB at 12 (the 8K encoder slope).
    fn need(n: u32) -> u64 {
        7_000 + 330 * u64::from(n)
    }

    #[test]
    fn automatic_when_it_fits_or_memory_is_unknown() {
        let c = choose(54, Some(200_000), need);
        assert_eq!((c.chosen, c.reduced()), (54, false));
        assert_eq!(choose(54, None, need).chosen, 54);
    }

    #[test]
    fn reduced_only_as_far_as_the_free_memory_needs() {
        // 16 GB free: 24 threads need 14.9 GB and fit; 32 need 17.6 GB.
        let c = choose(54, Some(16_384), need);
        assert_eq!(c.chosen, 24);
        assert!(c.reduced() && c.chosen_mb <= 16_384);
        // 12,000 MB free: 16 threads need 12,280, 12 threads 10,960.
        assert_eq!(choose(54, Some(12_000), need).chosen, 12);
        // Nothing fits: the smallest count; the preflight then warns.
        let c = choose(54, Some(4_000), need);
        assert_eq!(c.chosen, 4);
        assert!(c.chosen_mb > 4_000);
        // A machine whose automatic count is already below a ladder step.
        assert_eq!(choose(15, Some(11_000), need).chosen, 12);
    }

    #[test]
    fn sentence_names_counts_numbers_and_the_way_out() {
        let c = choose(54, Some(16_384), need);
        let s = reduction_sentence(&c, 16_384, false);
        assert!(s.contains("24 of 54 threads"), "{s}");
        assert!(s.contains("16.0 GB of memory free now"), "{s}");
        assert!(s.contains("HEVC"), "{s}");
        assert!(s.contains("Close other programs"), "{s}");
        let s = reduction_sentence(&c, 16_384, true);
        assert!(s.contains(MEMORY_LIMIT_ENV), "{s}");
        assert!(
            s.contains("Raise SHELLX_CUT_RENDER_MEMORY_MB for full speed"),
            "{s}"
        );
    }
}
