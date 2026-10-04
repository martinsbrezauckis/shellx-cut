# ShellX Cut manual source

This directory is the canonical source for the ShellX Cut web manual and its
Cut-owned assets. Keep the `data-app-version` marker in `cut/index.html` equal
to the documented desktop version. Before publication, the adjacent release
truth attributes follow the candidate's `schema/verbs.json`. After GitHub
publication is verified, update the web manual to `data-release-status="published"`
and its actual `data-published-version`; the frozen package's candidate metadata
is historical and must not be rewritten to make this web-content update.

The shared manual theme is vendored here so the Cut page can be checked from
the project checkout. The deployed documentation site may aggregate sibling
product manuals around this page; those sibling manuals remain owned by their
respective projects.

For v0.6.114, update this existing HTML and its feature descriptions in
`manual.js`. Regenerate `ui/src/manual/content.generated.json` for text parity
with the already shipped read-only help panel. This does not activate the
deferred remastered interactive manual. Do not select `ui/manual.html` or run
`scripts/public/stage-cut-manual.mjs` to replace the current web manual until
that separate roadmap item is accepted.
