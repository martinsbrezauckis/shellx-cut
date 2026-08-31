# ShellX Cut manual source

This directory is the canonical source for the ShellX Cut web manual and its
Cut-owned assets. Keep the `data-app-version` marker in `cut/index.html` equal
to the desktop candidate version, and keep the adjacent candidate/published
truth attributes synchronized with `schema/verbs.json`.

The shared manual theme is vendored here so the Cut page can be checked from
the project checkout. The deployed documentation site may aggregate sibling
product manuals around this page; those sibling manuals remain owned by their
respective projects.
