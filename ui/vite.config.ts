// vite.config.ts — ShellX Cut UI build config.
// Dev: vite dev server proxies /api (REST + WS) to cutd on 127.0.0.1:6161 so
// `npm run dev` works against a live engine. Prod: `npm run build` → ui/dist,
// served BY cutd itself at / (server contract) — relative /api paths just work.
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// H2: single source of truth for the displayed version — read package.json at
// build time and inject it as a compile-time constant. The status bar's BUILD_ID
// reads `__APP_VERSION__` instead of a hardcoded string, so the UI can never
// drift from the package version again (bump package.json, the chip follows).
const pkg = JSON.parse(readFileSync(fileURLToPath(new URL('./package.json', import.meta.url)), 'utf8')) as {
  version: string
}

const CUTD_TARGET = process.env.CUTD_DEV_TARGET || 'http://127.0.0.1:6161'
const cutdProxy = {
  target: CUTD_TARGET,
  changeOrigin: true,
  ws: true,
}

function manualChunks(id: string): string | undefined {
  const normalized = id.replace(/\\/g, '/')
  if (normalized.includes('/node_modules/')) return 'vendor'
  return undefined
}

export default defineConfig({
  plugins: [react()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  server: {
    proxy: {
      // Override the engine target when running the dev UI against a non-default
      // cutd (e.g. the surface-sweep's :6171 dev engine, so it never clobbers a
      // live app on :6161). Set CUTD_DEV_TARGET=http://127.0.0.1:6171.
      '/api': cutdProxy, // /api/events is a WebSocket upgrade
      '/frames': cutdProxy,
      '/filmstrip': cutdProxy,
      '/proxies': cutdProxy,
    },
  },
  build: {
    outDir: 'dist',
    // Security hardening: production source maps are stripped for the public release,
    // smaller bundle + the readable UI source isn't shipped to whoever installs
    // the app. (Low-stakes either way: the UI is a thin client over the verb API;
    // the IP is the compiled Rust engine, not the React code.) Re-enable for
    // debugging a field build with `SHELLX_CUT_SOURCEMAPS=1 npm run build`.
    sourcemap: process.env.SHELLX_CUT_SOURCEMAPS === '1',
    rollupOptions: {
      output: {
        manualChunks,
      },
    },
  },
})
