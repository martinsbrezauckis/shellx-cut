export const REAL_DIARIZE_MODEL = 'sortformer-v2'

export function isRealDiarizeModel(result) {
  return result?.backend === 'sortformer' && result?.model === REAL_DIARIZE_MODEL
}
