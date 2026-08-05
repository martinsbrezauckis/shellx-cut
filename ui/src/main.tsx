// main.tsx — React entrypoint. Mounts App into #root; theme.css is global.
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import App from './App'
import { initTheme } from './lib/themePref'
import './theme.css'

// Apply the persisted colour theme BEFORE first paint so a light-theme reload
// never flashes the dark default. Dark = no attribute (the :root baseline).
initTheme()

// Tag the WebView engine. WebView2 (Windows, Chromium) needs the transport-bar
// compositor-layer promotion (translateZ + contain:paint) so the playing <video> doesn't
// re-rasterize the SVG icons. On WebKit (macOS WKWebView / Linux WebKitGTK) that SAME
// promotion makes the icons shimmer at sub-pixel positions even with NO video — so CSS
// gates it behind :not(.engine-webkit). Chromium keeps the fix; WebKit opts out.
{
  const ua = navigator.userAgent
  // `navigator.vendor` is the robust signal — "Apple Computer, Inc." for WKWebView/Safari,
  // "Google Inc." for WebView2/Chromium — so it's not fooled by UA quirks. UA check kept as
  // a fallback (Linux WebKitGTK reports a Safari-less AppleWebKit UA + empty vendor).
  const isWebKit =
    navigator.vendor === 'Apple Computer, Inc.' ||
    (/AppleWebKit/.test(ua) && !/Chrome|Chromium|Edg\//.test(ua))
  if (isWebKit) {
    document.documentElement.classList.add('engine-webkit')
  }
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
