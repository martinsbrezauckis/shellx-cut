#!/usr/bin/env node
/** Produce a real current-SDK Motion handoff without launching a browser. */
import { createHash } from "node:crypto";
import { cp, mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const SAMPLE_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAIGNIUk0AAHomAACAhAAA+gAAAIDoAAB1MAAA6mAAADqYAAAXcJy6UTwAAAAGYktHRAD/AP8A/6C9p5MAAAAHdElNRQfqBh4KOQqnS6Y6AAAAEGNhTnYAAAABAAAAAQAAAAAAAAAAmdvqagAAABFJREFUCNdjZGBg+P///38GAA4EA/75rp4uAAAAAElFTkSuQmCC",
  "base64",
);

function arg(name: string): string {
  const index = process.argv.indexOf(name);
  const value = index >= 0 ? process.argv[index + 1] : undefined;
  if (!value || value.startsWith("--")) throw new Error(`Missing ${name}.`);
  return resolve(value);
}

function sha256(bytes: Buffer): string {
  return createHash("sha256").update(bytes).digest("hex");
}

const motionRoot = arg("--motion-root");
const artifactRoot = arg("--artifact-root");
const sampleMedia = arg("--sample-media");
const packageRoot = join(motionRoot, "fixtures", "packages", "lower-third");
const localSdkUrl = pathToFileURL(join(motionRoot, "packages", "sdk", "src", "local.ts")).href;
const { createLocalMotionSdk } = await import(localSdkUrl) as {
  createLocalMotionSdk: (options: Record<string, unknown>) => {
    render(input: Record<string, unknown>): Promise<Record<string, unknown>>;
  };
};

const sdk = createLocalMotionSdk({
  browserFrameRenderer: async (pkg: Record<string, any>, options: Record<string, any>) => {
    const outputPath = options.outputPath ?? join(options.outDir, "frame.png");
    await mkdir(dirname(outputPath), { recursive: true });
    await writeFile(outputPath, SAMPLE_PNG);
    const output = {
      path: outputPath,
      sha256: sha256(SAMPLE_PNG),
      format: "png",
      width: pkg.motion.width,
      height: pkg.motion.height,
      atMs: options.atMs,
      browser: { name: "none", version: "motion-lineage-gate" },
      viewport: { width: pkg.motion.width, height: pkg.motion.height, deviceScaleFactor: 1 },
    };
    return {
      ok: true,
      output,
      receipt: {
        schema: "shellx-motion/receipt@1",
        id: `lineage-frame-${options.atMs}`,
        operation: "preview.frame",
        status: "passed",
        packageId: pkg.manifest.id,
        inputHashes: { source: "a".repeat(64) },
        createdAt: "2026-07-15T00:00:00.000Z",
        lane: "motion-lineage-gate",
        output,
        warnings: [],
      },
    };
  },
  ffmpegRunner: async (command: { args: string[] }) => {
    if (command.args[0] === "-version") {
      return { exitCode: 0, stdout: "ffmpeg version motion-lineage-gate", stderr: "" };
    }
    const outputPath = command.args.at(-1);
    if (!outputPath) throw new Error("Motion FFmpeg command had no output path.");
    await mkdir(dirname(outputPath), { recursive: true });
    await cp(sampleMedia, outputPath);
    return { exitCode: 0, stdout: "", stderr: "" };
  },
});

const rendered = await sdk.render({
  packageRoot,
  artifactRoot,
  outputPath: join(artifactRoot, "lineaged.mp4"),
  preset: "mp4-h264",
  cutHandoff: { target: "shellx-cut", mode: "rendered_media" },
});
if (rendered.ok !== true) throw new Error(`Motion SDK render failed: ${JSON.stringify(rendered)}`);
const output = rendered.output as Record<string, any>;
if (!output.cutHandoff?.path || !output.artifact?.packageLineage) {
  throw new Error("Motion SDK did not emit a lineaged Cut handoff.");
}
const planBytes = await readFile(output.cutHandoff.path);
process.stdout.write(`${JSON.stringify({
  schema: "shellx-motion/cut-lineage-gate-producer@1",
  planPath: output.cutHandoff.path,
  planSha256: sha256(planBytes),
  artifactHandleId: output.artifact.id,
  operationHash: output.artifact.operationHash,
  packageLineage: output.artifact.packageLineage,
})}\n`);
