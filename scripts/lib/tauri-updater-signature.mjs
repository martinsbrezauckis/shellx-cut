const BASE64_RX = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/
const PUBLIC_SIGNATURE_HEADER = 'Public signature:'

// `cargo tauri signer sign` emits a human-readable report around this field,
// whereas a package build writes just the payload to its `.sig`. Extract only
// the explicitly-labelled record and leave Minisign verification to Rust.
export function normalizeTauriUpdaterSignature(value, label = 'updater signature') {
  if (typeof value !== 'string') throw new Error(`${label} must be text`)
  const trimmed = value.trim()
  if (!trimmed) throw new Error(`${label} is empty`)

  if (!/[\r\n]/.test(trimmed)) return validateBase64Signature(trimmed, label)

  const lines = value.replace(/\r\n?/g, '\n').split('\n')
  const headers = lines
    .map((line, index) => (line.trim() === PUBLIC_SIGNATURE_HEADER ? index : -1))
    .filter((index) => index >= 0)
  if (headers.length !== 1) {
    throw new Error(`${label} must contain exactly one ${PUBLIC_SIGNATURE_HEADER} field`)
  }
  const encoded = lines[headers[0] + 1]?.trim()
  if (!encoded) throw new Error(`${label} has no value after ${PUBLIC_SIGNATURE_HEADER}`)
  return validateBase64Signature(encoded, label)
}

function validateBase64Signature(value, label) {
  if (/\s/.test(value)) throw new Error(`${label} base64 value contains whitespace`)
  if (!BASE64_RX.test(value)) throw new Error(`${label} is not canonical base64`)
  return value
}
