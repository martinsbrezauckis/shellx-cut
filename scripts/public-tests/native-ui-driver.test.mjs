import assert from "node:assert/strict"
import test from "node:test"

import { assessWebView2CdpVersion } from "../lib/native-ui-driver.mjs"

test("installed WebView2 CDP identity is accepted", () => {
  const result = assessWebView2CdpVersion({
    Browser: "Edg/138.0.3351.77",
    "User-Agent": "Mozilla/5.0 Edg/138.0.3351.77",
    webSocketDebuggerUrl: "ws://127.0.0.1:9223/devtools/browser/fixture",
  })
  assert.equal(result.ok, true)
})

test("a regular Chromium endpoint cannot masquerade as installed WebView2", () => {
  const result = assessWebView2CdpVersion({
    Browser: "Chrome/138.0.0.0",
    "User-Agent": "HeadlessChrome/138.0.0.0",
    webSocketDebuggerUrl: "ws://127.0.0.1:9223/devtools/browser/fixture",
  })
  assert.equal(result.ok, false)
  assert.match(result.missing.join("\n"), /Microsoft Edge or WebView2/)
})

test("an Edge label without a debugger endpoint is rejected", () => {
  const result = assessWebView2CdpVersion({ Browser: "Microsoft Edge 138" })
  assert.equal(result.ok, false)
  assert.match(result.missing.join("\n"), /WebSocket debugger URL/)
})
