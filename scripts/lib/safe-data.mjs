const BASE64_RE = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;
const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const DANGEROUS_JSON_KEYS = new Set(["__proto__", "constructor", "prototype"]);

export function stripPngDataUrl(value) {
  return String(value ?? "").replace(/^data:image\/png;base64,/, "");
}

export function base64ToBuffer(value, { expectPng = true } = {}) {
  const text = String(value ?? "").trim();
  if (!text || text.length % 4 !== 0 || !BASE64_RE.test(text)) {
    throw new Error("Invalid base64 payload");
  }

  let binary = "";
  try {
    binary = atob(text);
  } catch (err) {
    throw new Error(`Invalid base64 payload: ${err?.message || String(err)}`);
  }

  const bytes = Uint8Array.from(binary, (ch) => ch.charCodeAt(0));
  const buffer = Buffer.alloc(bytes.length);
  buffer.set(bytes);
  if (expectPng) {
    const ok = PNG_SIGNATURE.every((byte, index) => buffer[index] === byte);
    if (!ok) throw new Error("Decoded data is not a PNG");
  }
  return buffer;
}

export function safeJsonParse(text, { fallback } = {}) {
  try {
    return JSON.parse(String(text), (key, value) => {
      if (DANGEROUS_JSON_KEYS.has(key)) {
        throw new Error(`Unsafe JSON key: ${key}`);
      }
      return value;
    });
  } catch (err) {
    if (arguments.length > 1 && Object.prototype.hasOwnProperty.call(arguments[1] ?? {}, "fallback")) {
      return fallback;
    }
    throw err;
  }
}
