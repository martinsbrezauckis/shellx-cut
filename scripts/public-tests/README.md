# Public script tests

These deterministic contract tests are deliberately part of the public source
snapshot. They verify public schemas, documentation, packaging inputs, update
metadata, and generic cross-platform harness behavior without depending on
private hosts, credentials, media, receipts, or release evidence.

Working-only scripts belong under `scripts/private/`, and private release
evidence belongs in the governed Release Studio project directory.

`inventory.json` classifies every public test. It is a fail-closed inventory:
adding a new `*.test.mjs` here without an entry makes the local guard and CI
fail. Run the source-only class with:

```bash
node scripts/check-public-test-inventory.mjs --run --profile source
```

The Agent Chat containment test is deliberately `requires-exact-cutd`: it must
receive `CUTD_BIN` for a binary built from this checkout, rather than using its
bounded MCP exchange timer to cold-compile Cargo. CI provisions that binary;
locally run the class explicitly after building it:

```bash
CUTD_BIN=app/target/debug/cutd \
  node scripts/check-public-test-inventory.mjs --run --profile requires-exact-cutd
```
