# Public UI tests

This directory contains deterministic contributor-facing product contracts
that are safe to publish with ShellX Cut. They use product source, synthetic
fixtures, and local temporary data only.

Native host orchestration, installed-build qualification, provider login or
quota checks, focus-driving desktop automation, release receipts, and
control-plane runners are maintained by the project outside this repository.

Run the public suite from the repository root:

```bash
npm --prefix ui run test:lib
```

Some of these tests drive a real browser; install Playwright's Chromium once with
`npx --prefix ui playwright install chromium`.
