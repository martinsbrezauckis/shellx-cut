#!/usr/bin/env node
// generate-adapter-shim.test.mjs — protocol tests for the two Generate
// planner adapter shims (app/perception/py/*.py) WITHOUT any real CLI agent:
// fake "claude" executables prove the envelope contract (not_run / completed /
// needs_input / retry-on-invalid / honest error), and the bundle contract
// (tauri resources carry both shims into the perception payload) is tripwired
// so the installed app can never silently lose the Generate prompt backend.
//
// Run: node scripts/public-tests/generate-adapter-shim.test.mjs
// Needs: python3 (any >= 3.8; the shims are stdlib-only by contract).
import { strict as assert } from "node:assert";
import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const PROMPT_ADAPTER = join(ROOT, "app", "perception", "py", "generate_prompt_adapter.py");
const STORY_ADAPTER = join(ROOT, "app", "perception", "py", "generate_storyboard_adapter.py");

// ── bundle contract: the shims ship in the perception resource payload ──────
const tauriConf = JSON.parse(
  readFileSync(join(ROOT, "app", "desktop", "src-tauri", "tauri.conf.json"), "utf8"),
);
const resources = tauriConf.bundle?.resources ?? {};
assert.equal(
  resources["../../perception/py/generate_prompt_adapter.py"],
  "perception/generate_prompt_adapter.py",
  "tauri resources must bundle the Generate prompt adapter beside instruments.py",
);
assert.equal(
  resources["../../perception/py/generate_storyboard_adapter.py"],
  "perception/generate_storyboard_adapter.py",
  "tauri resources must bundle the Generate storyboard adapter beside instruments.py",
);

// ── python availability (honest skip keeps CI hosts without python green) ───
const py = spawnSync("python3", ["--version"], { encoding: "utf8" });
if (py.error || py.status !== 0) {
  console.log("SKIP generate-adapter-shim: python3 not available on this host");
  process.exit(0);
}

function runShim(adapter, request) {
  const out = execFileSync("python3", [adapter, "plan"], {
    input: JSON.stringify(request),
    encoding: "utf8",
  });
  return JSON.parse(out);
}

const work = mkdtempSync(join(tmpdir(), "gen-shim-"));
// Fake CLI = a script that drains stdin then cats a payload FILE — payloads
// carry backticks/quotes (markdown fences), so never embed them in shell text.
function fakeCli(name, payloads) {
  const files = payloads.map((obj, i) => {
    const f = join(work, `${name}.${i}.json`);
    writeFileSync(f, JSON.stringify(obj));
    return f;
  });
  const p = join(work, name);
  const flag = join(work, `${name}.flag`);
  const body =
    files.length === 1
      ? `cat ${JSON.stringify(files[0])}`
      : `if [ ! -f ${JSON.stringify(flag)} ]; then touch ${JSON.stringify(flag)}; cat ${JSON.stringify(files[0])}; else cat ${JSON.stringify(files[1])}; fi`;
  writeFileSync(p, `#!/usr/bin/env bash\ncat > /dev/null\n${body}\n`);
  chmodSync(p, 0o755);
  return p;
}

const TEMPLATES = [
  {
    id: "TID",
    kind: "title",
    title: "T",
    summary: "s",
    params: { text: { type: "string", required: true } },
  },
];

try {
  // 1. not_run when no CLI agent resolves (both shims)
  for (const adapter of [PROMPT_ADAPTER, STORY_ADAPTER]) {
    const env = runShim(adapter, { agent: "auto", agents: {}, templates: [] });
    assert.equal(env.status, "not_run");
    assert.match(env.reason, /probed: claude, codex, grok/);
    assert.equal(env.plan ?? env.storyboard ?? null, null);
  }

  // 2. explicit agent missing -> not_run probing ONLY that agent
  {
    const env = runShim(PROMPT_ADAPTER, {
      agent: "grok",
      agents: { claude: "/bin/true", grok: null },
      templates: [],
    });
    assert.equal(env.status, "not_run");
    assert.match(env.reason, /probed: grok/);
  }

  // 3. completed plan via fake claude (fenced JSON tolerated, model captured)
  {
    const cli = fakeCli("fake_plan", [
      {
        result: '```json\n{"template_id": "TID", "params": {"text": "Hello"}, "at_ms": null}\n```',
        model: "fake-model-1",
      },
    ]);
    const env = runShim(PROMPT_ADAPTER, {
      agent: "claude",
      agents: { claude: cli },
      templates: TEMPLATES,
      prompt: "hello",
      timeout_ms: 30000,
    });
    assert.equal(env.status, "completed");
    assert.equal(env.plan.template_id, "TID");
    assert.equal(env.plan.params.text, "Hello");
    assert.deepEqual(env.backend, { provider: "claude", model: "fake-model-1" });
  }

  // 4. invalid first answer -> ONE retry with feedback -> completed + warning
  {
    const cli = fakeCli("fake_retry", [
      { result: '{"template_id": "nope.missing", "params": {}}' },
      { result: '{"template_id": "TID", "params": {}}' },
    ]);
    const env = runShim(PROMPT_ADAPTER, {
      agent: "claude",
      agents: { claude: cli },
      templates: TEMPLATES,
      prompt: "x",
      timeout_ms: 30000,
    });
    assert.equal(env.status, "completed");
    assert.equal(env.plan.template_id, "TID");
    assert.ok(env.warnings.some((w) => /retry/.test(w)));
  }

  // 5. stubborn wrong answer -> honest error naming the validation failure
  {
    const cli = fakeCli("fake_stubborn", [{ result: '{"template_id": "TID"}' }]);
    const env = runShim(PROMPT_ADAPTER, {
      agent: "claude",
      agents: { claude: cli },
      templates: TEMPLATES,
      prompt: "x",
      template_id: "OTHER",
      timeout_ms: 30000,
    });
    assert.equal(env.status, "error");
    assert.match(env.reason, /OTHER/);
  }

  // 6. storyboard completed via fake claude (template + assemble_slot scenes)
  {
    const ir = {
      schema: "shellx-cut/generate-storyboard/1",
      storyboard_id: "fake-plan",
      mode: "quick_prompt",
      status: "valid",
      scenes: [
        {
          scene_id: "s1",
          index: 1,
          role: "hook",
          source: "generate_template",
          template_id: "TID",
          range_ms: [0, 4000],
          params: {},
        },
        {
          scene_id: "s2",
          index: 2,
          role: "broll",
          source: "assemble_slot",
          query: "wide shot of the product",
          range_ms: [4000, 9000],
        },
      ],
      brief_meta: { stated: ["purpose"], inferred: ["platform"], missing: [] },
      missing_assets: [],
      validation: { missing_inputs: [] },
    };
    const cli = fakeCli("fake_story", [
      { result: JSON.stringify({ storyboard: ir, questions: [] }) },
    ]);
    const env = runShim(STORY_ADAPTER, {
      agent: "claude",
      agents: { claude: cli },
      templates: TEMPLATES,
      input: "promo",
      mode: "quick_prompt",
      timeout_ms: 30000,
    });
    assert.equal(env.status, "completed");
    assert.equal(env.schema, "shellx-cut/generate-storyboard-result/1");
    assert.equal(env.storyboard.scenes.length, 2);
  }

  // 7. storyboard needs_input surfaces exactly ONE question
  {
    const ir = {
      schema: "shellx-cut/generate-storyboard/1",
      storyboard_id: "fake-brief",
      mode: "director_brief",
      status: "needs_input",
      scenes: [],
      brief_meta: { stated: [], inferred: [], missing: ["purpose"] },
      missing_assets: [],
      validation: { missing_inputs: ["purpose"] },
    };
    const q = [{ field: "purpose", question: "What is this video for?" }];
    const cli = fakeCli("fake_question", [
      { result: JSON.stringify({ storyboard: ir, questions: q }) },
    ]);
    const env = runShim(STORY_ADAPTER, {
      agent: "claude",
      agents: { claude: cli },
      templates: TEMPLATES,
      input: "make me a video",
      mode: "director_brief",
      timeout_ms: 30000,
    });
    assert.equal(env.status, "needs_input");
    assert.equal(env.questions.length, 1);
    assert.equal(env.questions[0].field, "purpose");
  }

  // 8. auto mode falls through when the first CLI HARD-fails (exit 1) — an
  //    installed-but-unauthenticated claude must not kill the feature for a
  //    user whose codex works.
  {
    const dead = join(work, "dead_cli");
    writeFileSync(dead, `#!/usr/bin/env bash\ncat > /dev/null\necho "auth expired" >&2\nexit 1\n`);
    chmodSync(dead, 0o755);
    // the fall-through target is spawned with the CODEX recipe: the final
    // message goes to the file named after -o, not to stdout.
    const payload = join(work, "codex_payload.txt");
    writeFileSync(payload, '{"template_id": "TID", "params": {}}');
    const good = join(work, "fake_codex");
    writeFileSync(
      good,
      `#!/usr/bin/env bash\ncat > /dev/null\nout=""; prev=""\nfor a in "$@"; do if [ "$prev" = "-o" ]; then out="$a"; fi; prev="$a"; done\nif [ -n "$out" ]; then cp ${JSON.stringify(payload)} "$out"; else cat ${JSON.stringify(payload)}; fi\n`,
    );
    chmodSync(good, 0o755);
    const env = runShim(PROMPT_ADAPTER, {
      agent: "auto",
      agents: { claude: dead, codex: good },
      templates: TEMPLATES,
      prompt: "x",
      timeout_ms: 30000,
    });
    assert.equal(env.status, "completed");
    assert.equal(env.backend.provider, "codex");
    assert.ok(env.warnings.some((w) => /claude failed, trying next/.test(w)));
  }

  // 9. explicit agent choice does NOT substitute on failure — honest error.
  {
    const dead = join(work, "dead_cli2");
    writeFileSync(dead, `#!/usr/bin/env bash\ncat > /dev/null\nexit 1\n`);
    chmodSync(dead, 0o755);
    const env = runShim(PROMPT_ADAPTER, {
      agent: "claude",
      agents: { claude: dead, codex: "/bin/true" },
      templates: TEMPLATES,
      prompt: "x",
      timeout_ms: 30000,
    });
    assert.equal(env.status, "error");
    assert.match(env.reason, /claude/);
  }

  // 10. a template marked available:false (Motion not installed) is REFUSED
  //     with a reason naming ShellX Motion — the retry feedback steers the
  //     model to a renderable template instead of a confusing insert failure.
  {
    const cli = fakeCli("fake_motion_pick", [
      { result: '{"template_id": "MOTION_TID", "params": {}}' },
      { result: '{"template_id": "TID", "params": {}}' },
    ]);
    const env = runShim(PROMPT_ADAPTER, {
      agent: "claude",
      agents: { claude: cli },
      templates: [
        ...TEMPLATES,
        { id: "MOTION_TID", kind: "motion", title: "M", summary: "m", params: {}, available: false },
      ],
      prompt: "x",
      timeout_ms: 30000,
    });
    assert.equal(env.status, "completed");
    assert.equal(env.plan.template_id, "TID", "retry must land on the available template");
    assert.ok(env.warnings.some((w) => /retry/.test(w)));
  }

  console.log("generate-adapter-shim.test.mjs PASS (10 protocol checks + bundle contract)");
} finally {
  rmSync(work, { recursive: true, force: true });
}
