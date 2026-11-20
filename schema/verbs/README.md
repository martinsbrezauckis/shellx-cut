# Verb contract sources

`base.json` and the ordered files in `fragments/` are the editable canonical
sources for the ShellX Cut verb contract. `manifest.json` fixes their public
order.

After an edit, run:

```sh
node scripts/generate-verbs.mjs
node scripts/generate-verb-contract.mjs
```

A new verb must also be placed in exactly one feature of `schema/requirements.json`
with what it needs; then run `node scripts/generate-requirements-docs.mjs`, which
writes the "what needs what" sections of the agent skill (`--check` fails when
they are out of date).

The generated `schema/verbs.json` stays at its established path because the
server embeds it and external clients consume it. CI runs both generators in
check mode and rejects aggregate drift, duplicate verbs, mixed-domain
fragments, missing or orphan fragments, and fragments over 600 lines.
