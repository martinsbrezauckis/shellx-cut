export function assessWebView2CdpVersion(payload = {}) {
  const browser = String(payload.Browser || payload.browser || "").trim()
  const userAgent = String(payload["User-Agent"] || payload.userAgent || "").trim()
  const identity = `${browser} ${userAgent}`.trim()
  const webSocketDebuggerUrl = String(
    payload.webSocketDebuggerUrl || payload.webSocketDebuggerURL || "",
  ).trim()
  const missing = []

  if (!/\b(?:Edg|Microsoft Edge|WebView2)[A-Za-z]*[\/ ]/i.test(identity)) {
    missing.push("CDP /json/version does not identify Microsoft Edge or WebView2")
  }
  if (!/^wss?:\/\//i.test(webSocketDebuggerUrl)) {
    missing.push("CDP /json/version is missing a WebSocket debugger URL")
  }

  return {
    ok: missing.length === 0,
    browser,
    userAgent,
    webSocketDebuggerUrl,
    missing,
  }
}
