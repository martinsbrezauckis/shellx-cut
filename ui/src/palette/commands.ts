// commands.ts — the command registry behind the Cmd-K palette.
//
// ShellX Cut exposes 260 verbs across 32 domains; the palette is the agent-first
// discoverability surface that makes them reachable by keyboard, the thing the
// native incumbents (Resolve/FCP) lack. v1 covers the high-value SURFACE launchers
// (every editing drawer + the review surfaces) routed through Cut's existing
// `cut:open-*` event bus — so the palette opens the real tools, not stubs. Deeper
// verb EXECUTION commands (split/render/export with typed args) layer on next.
//
// Each command names an INTENT; `run()` dispatches a DOM event the App already
// handles (or a new `cut:open-drawer` we add one handler for). To add a command:
// one entry here — never wire a new key handler per command.
import type { IconName } from "../icons";
import { openUiSurface, type UiSurfaceId } from "../app/uiSurfaceRegistry";

export interface Command {
  /** Stable id (for keys + recents). */
  id: string;
  /** What the user reads. */
  label: string;
  /** Short plain-language outcome shown in the result row and tooltip. */
  description?: string;
  /** Section header in the list. */
  group: string;
  /** Registry glyph. */
  icon: IconName;
  /** Right-aligned shortcut hint that teaches the keyboard binding. */
  hint?: string;
  /** Extra fuzzy-match terms not in the label. */
  keywords?: string;
  /** Online manual feature id. The palette does not open docs by default. */
  manualId?: string;
  /** Local UI highlight to show after opening the matching surface. */
  highlight?: {
    selector?: string;
    clip?: string;
    panel?: string;
    label?: string;
    description?: string;
    duration_ms?: number;
    scroll?: boolean;
  };
  /** Fire the command (then the palette closes). */
  run: () => void;
}

/** Open through the same stable surface id used by ui.open and ui.state. */
const surface = (id: UiSurfaceId) => () => openUiSurface(id);
const drawer = (name: string) => surface(
  (name === "grade" ? "color" : name === "mixer" ? "audio" : name) as UiSurfaceId,
);
/** Fire an existing bare App event (receipts / comments / import). */
const emit = (event: string) => () => document.dispatchEvent(new CustomEvent(event));
/** Open one review rail tab by name. */
const reviewTab = (name: string) => surface(name as UiSurfaceId);
/** Open one left-sidebar tab by name. */
const leftTab = (name: string) => surface(name as UiSurfaceId);
/** Open the Settings > Environment drawer. */
const environment = surface("environment");

const localHighlight = (spec?: Command["highlight"], delayMs = 180) => {
  if (!spec) return;
  window.setTimeout(() => {
    document.dispatchEvent(new CustomEvent("cut:local-highlight", {
      detail: {
        duration_ms: 4500,
        ...spec,
      },
    }));
  }, delayMs);
};

const openAndHighlight = (open: () => void, spec?: Command["highlight"], delayMs?: number) => () => {
  open();
  localHighlight(spec, delayMs);
};

/**
 * The command list. Order is the default (pre-search) order; search re-ranks by
 * match score. Groups cluster by editing intent, production-order-ish.
 */
export const COMMANDS: Command[] = [
  // ── Setup / readiness ──
  {
    id: "media-health",
    label: "Media health",
    description: "Check missing sources, proxy readiness, and heavy-clip playback.",
    group: "Setup",
    icon: "assets",
    keywords: "offline missing relink proxy proxies health codec 4k phone camera lag playback source media check",
    manualId: "cut.left.media_health",
    highlight: {
      selector: "[data-cut-media-health]",
      label: "Media Health",
      description: "Shows missing files and whether large clips are using proxies or source playback.",
    },
    run: openAndHighlight(leftTab("assets"), {
      selector: "[data-cut-media-health]",
      label: "Media Health",
      description: "Shows missing files and whether large clips are using proxies or source playback.",
    }),
  },
  {
    id: "proxy-imports",
    label: "Proxy imports",
    description: "Turn proxy generation on for smoother 4K and camera-file editing.",
    group: "Setup",
    icon: "video",
    keywords: "proxy proxies optimized playback 4k lag heavy phone camera import source smooth",
    manualId: "cut.left.proxies",
    highlight: {
      selector: "[data-cut-proxy-toggle]",
      label: "Proxy imports",
      description: "Keep this on for smoother playback with 4K, phone, and camera files.",
    },
    run: openAndHighlight(leftTab("assets"), {
      selector: "[data-cut-proxy-toggle]",
      label: "Proxy imports",
      description: "Keep this on for smoother playback with 4K, phone, and camera files.",
    }),
  },
  {
    id: "ffmpeg-setup",
    label: "Video tools setup",
    description: "Install FFmpeg so preview, import, and export work.",
    group: "Setup",
    icon: "settings",
    keywords: "ffmpeg video processing install preview export import setup missing blank no preview",
    manualId: "cut.preview.ffmpeg_setup",
    highlight: {
      selector: "[data-cut-env-card=\"ffmpeg\"]",
      label: "Video processing",
      description: "Install this first when media will not preview, import, or export.",
    },
    run: openAndHighlight(environment, {
      selector: "[data-cut-env-card=\"ffmpeg\"]",
      label: "Video processing",
      description: "Install this first when media will not preview, import, or export.",
    }, 260),
  },
  {
    id: "cli-agent-setup",
    label: "CLI agent setup",
    description: "Connect a local CLI for Agent Chat or other AI workflows.",
    group: "Setup",
    icon: "agent",
    keywords: "codex claude grok cli agent ai generate chat install connect subscription",
    manualId: "cut.setup.cli_install",
    highlight: {
      selector: "[data-cut-environment]",
      label: "CLI agents",
      description: "Install a supported CLI agent before using Generate planning or chat.",
    },
    run: openAndHighlight(environment, {
      selector: "[data-cut-environment]",
      label: "CLI agents",
      description: "Install a supported CLI agent before using Generate planning or chat.",
    }, 260),
  },
  // ── Edit ──
  { id: "layer", label: "Transform & layer", description: "Move, scale, crop, and animate the selected clip.", group: "Edit", icon: "transform", keywords: "crop scale position move keyframe pip ken burns", manualId: "cut.inspector.transform", run: drawer("layer") },
  { id: "mask", label: "Region mask / blur", description: "Hide faces, text, or regions with blur, pixelate, or box masks.", group: "Edit", icon: "mask", keywords: "blur pixelate privacy redact region shape censor cover black box face label password", manualId: "cut.header.region_mask", run: openAndHighlight(drawer("mask"), { selector: "[data-cut-mask]", label: "Region mask", description: "Draw a region on the preview, then choose blur, pixelate, or box." }, 220) },
  { id: "clips", label: "Clip candidates", description: "Find moments that can work as shorts or highlight clips.", group: "Edit", icon: "film", keywords: "scenes cuts repurpose shorts highlights", manualId: "cut.header.repurpose", run: drawer("clips") },
  // ── Color ──
  { id: "grade", label: "Color grade", description: "Open the Color tab for grade, match, and auto color controls.", group: "Color", icon: "grade", keywords: "contrast brightness saturation lut temperature", manualId: "cut.inspector.color", run: drawer("grade") },
  { id: "matte", label: "Remove background", description: "Cut out a subject without a green screen.", group: "Color", icon: "matte", keywords: "matte key chroma green screen ai cutout", manualId: "cut.timeline.grade", run: drawer("matte") },
  // ── Add ──
  { id: "title", label: "Add title", description: "Create animated title cards or lower thirds.", group: "Add", icon: "text", keywords: "text caption lower third", manualId: "cut.header.title", run: drawer("title") },
  { id: "shape", label: "Add shape", description: "Add boxes, circles, arrows, and callout overlays.", group: "Add", icon: "shape", keywords: "box circle line rectangle arrow callout", manualId: "cut.header.shape", run: drawer("shape") },
  { id: "stock", label: "Stock footage", description: "Search local folders or public media providers for b-roll.", group: "Add", icon: "video", keywords: "broll b-roll public domain creative commons local folder", manualId: "cut.top.find.media", run: drawer("stock") },
  { id: "import", label: "Import media", description: "Browse for video, audio, or image files for this project.", group: "Add", icon: "import", keywords: "open file add footage phone camera", manualId: "cut.left.import", run: openAndHighlight(emit("cut:open-import"), { selector: "[data-cut-action=\"import-asset\"]", label: "Import media", description: "Browse for files; imported clips appear in Assets." }) },
  // ── Captions ──
  { id: "kinetic", label: "Caption styles", description: "Style static and kinetic captions for the current project.", group: "Captions", icon: "captions", keywords: "kinetic subtitles karaoke animated", manualId: "cut.workflow.captions", run: drawer("kinetic") },
  // ── Audio ──
  { id: "music", label: "Music bed", description: "Place music under speech and auto-duck it.", group: "Audio", icon: "music", keywords: "soundtrack duck background", manualId: "cut.header.music", run: drawer("music") },
  { id: "mixer", label: "Audio mixer", description: "Adjust track levels, pan, mute, solo, and loudness.", group: "Audio", icon: "mixer", keywords: "gain volume levels mute solo listen pan loudness lufs", manualId: "cut.header.mixer", run: openAndHighlight(drawer("mixer"), { selector: "[data-cut-mixer]", label: "Audio mixer", description: "Track controls for level, pan, mute, solo, and listen." }, 220) },
  // ── Agent ──
  { id: "autopilot", label: "Autopilot", description: "Let the agent plan low-risk fixes from measured receipts.", group: "Agent", icon: "autopilot", keywords: "ai auto edit agent assemble", manualId: "cut.header.autopilot", run: drawer("autopilot") },
  { id: "recipes", label: "Recipes", description: "Run named workflows such as podcast or screen-demo cleanup.", group: "Agent", icon: "bolt", keywords: "workflow pipeline template podcast talking head screen demo one click gated", manualId: "cut.header.recipes", run: drawer("recipes") },
  // ── Navigate / Review ──
  { id: "manual", label: "Open manual", description: "Open the current online ShellX Cut manual.", group: "Navigate", icon: "manual", keywords: "docs documentation help wiki guide features", manualId: "cut.top.manual", run: emit("cut:open-manual") },
  { id: "search", label: "Search project", description: "Find media or visual moments in the left sidebar.", group: "Navigate", icon: "search", keywords: "find assets clips text visual moment", manualId: "cut.top.find", run: drawer("search") },
  { id: "receipts", label: "Render receipts", description: "Review checks and evidence from renders and QC passes.", group: "Review", icon: "receipt", keywords: "qc verify checks evidence", manualId: "cut.review.receipts", run: surface("receipts") },
  { id: "scopes", label: "Video scopes", description: "Check luma, saturation, white balance, clipping, and broadcast range.", group: "Review", icon: "waveform", keywords: "vectorscope waveform histogram color clipping levels verify scopes", manualId: "cut.review.scopes", run: openAndHighlight(reviewTab("scopes"), { selector: "[data-cut-scopes]", label: "Video scopes", description: "Run measured color checks for the current frame." }, 220) },
  { id: "comments", label: "Comments", description: "Open timecoded review notes and suggested fixes.", group: "Review", icon: "comment", hint: "Ctrl/Cmd+Shift+C", keywords: "notes review feedback", manualId: "cut.header.comments.add", run: surface("comments") },
];
