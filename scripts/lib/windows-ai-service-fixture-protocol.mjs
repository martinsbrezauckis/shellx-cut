export const WINDOWS_AI_SERVICE_FIXTURE_PROVENANCE = 'fixture-protocol-only'
export const WINDOWS_AI_SERVICE_FIXTURE_DIARIZE_MODEL = 'shellx-cut-fixture-diarize-v1'
export const WINDOWS_AI_SERVICE_FIXTURE_DUB_MODEL = 'shellx-cut-fixture-dub-v1'
export const WINDOWS_AI_SERVICE_FIXTURE_PROTOCOL = 'shellx-cut/windows-ai-service-fixture@1'
export const WINDOWS_AI_SERVICE_FIXTURE_LIMITATION = 'Deterministic loopback protocol fixture only; it does not prove a live model, output quality, authentication, tunnel, speaker identity, or voice.'

function invariant(condition, message) {
  if (!condition) throw new Error(message)
}

export function wavDurationMs(wav) {
  invariant(Buffer.isBuffer(wav) && wav.length >= 44, 'fixture diarize request must contain a WAV body')
  invariant(wav.subarray(0, 4).toString('ascii') === 'RIFF' && wav.subarray(8, 12).toString('ascii') === 'WAVE',
    'fixture diarize request must contain a RIFF/WAVE body')
  let format = null
  let dataBytes = -1
  for (let offset = 12; offset + 8 <= wav.length;) {
    const id = wav.subarray(offset, offset + 4).toString('ascii')
    const size = wav.readUInt32LE(offset + 4)
    const start = offset + 8
    const end = start + size
    invariant(end <= wav.length, 'fixture diarize request has a truncated WAV chunk')
    if (id === 'fmt ' && size >= 16) {
      format = {
        code: wav.readUInt16LE(start), channels: wav.readUInt16LE(start + 2), sampleRate: wav.readUInt32LE(start + 4),
        byteRate: wav.readUInt32LE(start + 8), blockAlign: wav.readUInt16LE(start + 12), bitsPerSample: wav.readUInt16LE(start + 14),
      }
    }
    if (id === 'data') { dataBytes = size; break }
    offset = end + (size % 2)
  }
  invariant(format && format.code === 1 && format.channels === 1 && format.sampleRate === 16_000
    && format.byteRate === 32_000 && format.blockAlign === 2 && format.bitsPerSample === 16 && dataBytes % format.blockAlign === 0,
  'fixture diarize request must be 16 kHz mono signed 16-bit PCM WAV')
  invariant(dataBytes > 0, 'fixture diarize request has no measurable WAV duration')
  return Math.max(1, Math.round((dataBytes * 1000) / format.byteRate))
}
