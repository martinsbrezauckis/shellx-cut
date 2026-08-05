import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

export const AGENT_DOCS_SCHEMA = "shellx-cut/agent-docs/2";

export const AGENT_DOCS = Object.freeze([
  { id: "start-here", path: "START_HERE_FOR_AGENT.txt", advertised: true },
  { id: "agent-rules", path: "AGENTS.md", advertised: true },
  { id: "readme", path: "README.md", advertised: true },
  { id: "features", path: "docs/public/FEATURES.md", advertised: true },
  { id: "debug-api", path: "docs/public/DEBUG_API.md", advertised: true },
  { id: "judge-review", path: "docs/public/JUDGE_REVIEW.md", advertised: false },
  { id: "feature-workflow", path: "docs/public/FEATURE_CHANGE_WORKFLOW.md", advertised: true },
  { id: "motion-boundary", path: "docs/public/SHELLX_MOTION_BOUNDARY.md", advertised: true },
  { id: "verbs", path: "schema/verbs.json", advertised: true },
  { id: "skill", path: "skill/shellx-cut/SKILL.md", advertised: true },
  { id: "reference", path: "skill/shellx-cut/reference.md", advertised: true },
  { id: "craft-index", path: "skill/shellx-cut/craft/INDEX.md", advertised: true },
  { id: "craft-audio", path: "skill/shellx-cut/craft/audio-baseline.md" },
  { id: "craft-captions", path: "skill/shellx-cut/craft/captions-that-work.md" },
  { id: "craft-failed-checks", path: "skill/shellx-cut/craft/fix-failed-checks.md" },
  { id: "craft-director", path: "skill/shellx-cut/craft/generate-director-questioning.md" },
  { id: "craft-storyboard", path: "skill/shellx-cut/craft/generate-storyboard-planning.md" },
  { id: "craft-pacing", path: "skill/shellx-cut/craft/pacing-and-rhythm.md" },
  { id: "craft-deliverables", path: "skill/shellx-cut/craft/platform-deliverables.md" },
  { id: "craft-podcast", path: "skill/shellx-cut/craft/podcast-episode.md" },
  { id: "craft-review", path: "skill/shellx-cut/craft/review-discipline.md" },
  { id: "craft-screen-demo", path: "skill/shellx-cut/craft/screen-demo-polish.md" },
  { id: "craft-talking-head", path: "skill/shellx-cut/craft/talking-head-cleanup.md" },
]);

export const AGENT_DOC_PATHS = Object.freeze(AGENT_DOCS.map((doc) => doc.path));

function loopbackBase(value) {
  const base = new URL(value);
  const loopbackHosts = new Set(["127.0.0.1", "localhost", "[::1]"]);
  if (base.protocol !== "http:" || !loopbackHosts.has(base.hostname)) {
    throw new Error(`Agent-doc verification requires an HTTP loopback URL, got ${base.origin}`);
  }
  return base;
}

function docUrl(base, path) {
  const encoded = path.split("/").map(encodeURIComponent).join("/");
  return new URL(`/api/agent-doc/${encoded}`, base);
}

async function fetchBytes(url, timeoutMs) {
  const response = await fetch(url, {
    headers: { connection: "close" },
    signal: AbortSignal.timeout(timeoutMs),
  });
  const bytes = Buffer.from(await response.arrayBuffer());
  return { response, bytes };
}

export async function verifyAgentDocsApi({
  engineBase = "http://127.0.0.1:6161",
  sourceRoot,
  expectedVersion = "",
  timeoutMs = 10000,
} = {}) {
  if (!sourceRoot) throw new Error("sourceRoot is required for exact agent-doc verification");
  const base = loopbackBase(engineBase);
  const failures = [];
  let info = null;

  try {
    const response = await fetch(new URL("/api/agent", base), {
      headers: { connection: "close" },
      signal: AbortSignal.timeout(timeoutMs),
    });
    if (!response.ok) {
      failures.push(`/api/agent returned ${response.status}`);
    } else {
      info = await response.json();
      if (info?.schema !== AGENT_DOCS_SCHEMA) failures.push("/api/agent schema mismatch");
      if (info?.docs_available !== true) failures.push("/api/agent reports docs_available=false");
      if (expectedVersion && info?.version !== expectedVersion) {
        failures.push(`/api/agent version ${info?.version || "missing"} != ${expectedVersion}`);
      }
      const advertised = new Map((info?.read_first || []).map((entry) => [entry?.path, entry]));
      for (const doc of AGENT_DOCS.filter((entry) => entry.advertised)) {
        const entry = advertised.get(doc.path);
        if (!entry || entry.id !== doc.id || entry.url !== `/api/agent-doc/${doc.path}`) {
          failures.push(`/api/agent does not advertise ${doc.path} correctly`);
        }
      }
    }
  } catch (error) {
    failures.push(`/api/agent failed: ${error?.message || String(error)}`);
  }

  let served = 0;
  for (const doc of AGENT_DOCS) {
    try {
      const expected = await readFile(resolve(sourceRoot, doc.path));
      const { response, bytes } = await fetchBytes(docUrl(base, doc.path), timeoutMs);
      if (!response.ok) {
        failures.push(`${doc.path} returned ${response.status}`);
      } else if (!bytes.equals(expected)) {
        failures.push(`${doc.path} differs from candidate source`);
      } else {
        served += 1;
      }
    } catch (error) {
      failures.push(`${doc.path} failed: ${error?.message || String(error)}`);
    }
  }

  return {
    ok: failures.length === 0,
    schema: "shellx-cut/installed-agent-docs-verify@1",
    version: info?.version || null,
    checked: AGENT_DOCS.length,
    served,
    failures,
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (process.argv.includes("--paths")) {
    process.stdout.write(`${AGENT_DOC_PATHS.join("\n")}\n`);
  } else {
    process.stderr.write("Usage: node scripts/lib/agent-docs.mjs --paths\n");
    process.exitCode = 2;
  }
}
