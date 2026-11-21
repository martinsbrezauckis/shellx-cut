export interface RecordingDoctorTarget {
  fps: number
  sourceKind: 'display' | 'window' | 'region'
  monitor: number | null
  monitorId: string | null
  windowId: string | null
}

/** Check the same source and cadence the next recording would use. */
export function recordingDoctorArgs(target: RecordingDoctorTarget): {
  fps: number; monitor?: number; monitor_id?: string; window?: string
} {
  const args: { fps: number; monitor?: number; monitor_id?: string; window?: string } = { fps: target.fps }
  if (target.sourceKind === 'window') {
    if (target.windowId) args.window = target.windowId
  } else {
    if (target.monitor !== null) args.monitor = target.monitor
    if (target.monitorId) args.monitor_id = target.monitorId
  }
  return args
}
