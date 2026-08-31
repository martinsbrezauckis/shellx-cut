import { useCallback, useMemo, useState } from 'react'
import {
  qualityCapability,
  qualityResolution,
  type RecordingOutputSize,
  type RecordingQualityCapability,
  type RecordingQualityProfile,
  type RecordingQualityRequest,
  type RecordingQualityResolution,
} from './recordingQuality'

/** Keeps capability-gated choices and final verifier facts out of the Record page. */
export function useRecordingQuality() {
  const [capability, setCapability] = useState<RecordingQualityCapability | null>(null)
  const [outputSize, setOutputSize] = useState<RecordingOutputSize>('source')
  const [profile, setProfile] = useState<RecordingQualityProfile>('standard')
  const [resolution, setResolution] = useState<RecordingQualityResolution | null>(null)
  const request: RecordingQualityRequest | undefined = useMemo(() => capability
    && capability.output_sizes.includes(outputSize)
    && capability.profiles.includes(profile)
    ? { output_size: outputSize, profile }
    : undefined, [capability, outputSize, profile])
  const setCapabilityFromDoctor = useCallback((value: unknown) => setCapability(qualityCapability(value)), [])
  const setResolutionFromStop = useCallback((value: unknown) => setResolution(qualityResolution(value)), [])
  const clearResolution = useCallback(() => setResolution(null), [])

  return {
    capability,
    outputSize,
    profile,
    resolution,
    request,
    setOutputSize,
    setProfile,
    setCapability: setCapabilityFromDoctor,
    setResolution: setResolutionFromStop,
    clearResolution,
  }
}
