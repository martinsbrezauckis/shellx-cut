/// <reference types="vite/client" />
// Vite ambient types — makes CSS/asset imports typecheck (standard template file).

// H2: compile-time version constant injected by vite's `define` from
// package.json (vite.config.ts). The status bar's BUILD_ID reads this so the
// displayed version tracks the package version automatically.
declare const __APP_VERSION__: string
