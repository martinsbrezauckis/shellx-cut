# Public scripts

The scripts in this directory are contributor-facing source, build, test, and
release-integrity helpers that are safe to publish with ShellX Cut. They do not
contain credentials, private host inventory, provider authentication, installed
qualification evidence, or Release Studio control data.

- `public/` stages reproducible public artifacts. Staging never publishes them.
- `public-tests/` contains deterministic contract tests and its fail-closed test
  inventory.
- `release/` checks public release inputs and GitHub check results; it does not
  sign, publish, or access private evidence by itself.
- `lib/` contains shared implementation used by those public entrypoints.

Machine-specific orchestration and working evidence belong under
`scripts/private/` or `scripts/private-tests/`. Both are excluded from the
public export, as are `ui/private-tests/` and `docs/private/`.
