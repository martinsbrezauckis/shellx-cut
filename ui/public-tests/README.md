# Public UI tests

This directory contains deterministic contributor-facing product contracts
that are safe to publish with ShellX Cut. They use product source, synthetic
fixtures, and local temporary data only.

Native host orchestration, installed-candidate qualification, provider login or
quota checks, focus-driving desktop automation, release receipts, and
control-plane runners belong in the tracked `ui/private-tests/` tree and are
excluded from the public export.

Run the public suite from the repository root:

```bash
npm --prefix ui run test:lib
```
