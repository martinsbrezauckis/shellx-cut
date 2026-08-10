// Choose a real-media source that can fill an adjacent timeline gap within the
// engine's supported 0.25x-4x retiming range. Candidate order is significant:
// callers may prefer a newly imported asset while retaining existing project
// footage as a compatible fallback.

export function selectFitToFillCandidate(candidateIds, assets, maxGapMs, currentAssetId = '') {
  const candidates = [...new Set(candidateIds.filter((assetId) => assetId && assetId !== currentAssetId))].map((assetId) => {
    const durationMs = Number(assets?.[assetId]?.probe?.duration_ms || 0)
    const gapMs = Math.min(maxGapMs, Math.max(1_000, Math.ceil(durationMs / 2)))
    return { assetId, durationMs, gapMs, speed: gapMs > 0 ? durationMs / gapMs : 0 }
  })
  const selected = candidates.find((candidate) => (
    candidate.durationMs > 0
    && candidate.gapMs > 0
    && candidate.speed >= 0.25
    && candidate.speed <= 4
  )) || null
  return { selected, candidates }
}
