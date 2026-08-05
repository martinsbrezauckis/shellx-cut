# Generate storyboard planning

Craft skill for turning prompt, brief, script, or selected media context into a ShellX Cut Generate Storyboard IR.

Two ways to plan: call `generate.storyboard` and let the ENGINE plan (it ships a bundled adapter that routes to the user's local CLI agent — claude/codex/grok), or produce the IR yourself with the rules below when you ARE the planning agent. Either way the engine validates the IR before anything previews or inserts.

## Boundaries

- `generate.storyboard` plans multi-scene generated structure.
- `generate.from_prompt` stays for one editable visual element.
- `assemble.*` keeps transcript/media ranking.
- `render.storyboard` remains timeline contact-sheet evidence after insertion.

## Plan Rules

- Return `schema:"shellx-cut/generate-storyboard/1"`.
- Every scene has `scene_id`, `index`, `role`, `range_ms`, `source`, and evidence flags.
- `generate_template` scenes use real ids from `generate.list`.
- `assemble_slot` scenes describe existing-media needs without claiming the match has happened.
- Missing assets stay in `missing_assets`.
- Unsupported previews stay explicit; never invent preview evidence.
- Inserted work is not claimed until checkpoint, op ids, clip ids, and state proof exist.

## Evidence Rules

- Plan evidence: schema, scene count, duration, template ids, missing inputs.
- Preview evidence: image/frame result or unsupported marker per scene.
- Insert evidence: checkpoint, lowered verbs, op ids, clip ids, assets, restore hint.
- Review evidence: call `render.storyboard` only after insertion to inspect the actual timeline.
