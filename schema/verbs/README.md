# Verb contract sources

`base.json` and the ordered files in `fragments/` are the editable canonical
sources for the ShellX Cut verb contract. `manifest.json` fixes their public
order.

After an edit, run:

```sh
node scripts/generate-verbs.mjs
node scripts/generate-verb-contract.mjs
```

The generated `schema/verbs.json` stays at its established path because the
server embeds it and external clients consume it. CI runs both generators in
check mode and rejects aggregate drift, duplicate verbs, mixed-domain
fragments, missing or orphan fragments, and fragments over 600 lines.
