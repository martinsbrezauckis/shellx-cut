const areas = {
  setup: { left: 0.396, top: 0.009, width: 0.055, height: 0.022, label: "Settings" },
  recordMode: { left: 0.214, top: 0.009, width: 0.043, height: 0.022, label: "Record" },
  projects: { left: 0.292, top: 0.009, width: 0.048, height: 0.022, label: "Projects" },
  libraryTop: { left: 0.345, top: 0.009, width: 0.045, height: 0.022, label: "Library" },
  manualTop: { left: 0.456, top: 0.009, width: 0.047, height: 0.022, label: "Manual" },
  render: { left: 0.854, top: 0.009, width: 0.034, height: 0.022, label: "Render" },
  exportMenu: { left: 0.95, top: 0.009, width: 0.042, height: 0.022, label: "Export" },
  titleTool: { left: 0.632, top: 0.009, width: 0.014, height: 0.022, label: "Title" },
  shapeTool: { left: 0.647, top: 0.009, width: 0.014, height: 0.022, label: "Shape" },
  regionMaskTool: { left: 0.662, top: 0.009, width: 0.014, height: 0.022, label: "Mask / privacy" },
  musicTool: { left: 0.678, top: 0.009, width: 0.014, height: 0.022, label: "Music" },
  mixerTool: { left: 0.693, top: 0.009, width: 0.014, height: 0.022, label: "Mixer" },
  repurposeTool: { left: 0.708, top: 0.009, width: 0.014, height: 0.022, label: "Repurpose" },
  autopilotTool: { left: 0.724, top: 0.009, width: 0.014, height: 0.022, label: "Autopilot" },
  recipesTool: { left: 0.739, top: 0.009, width: 0.014, height: 0.022, label: "Recipes" },
  assembleTool: { left: 0.754, top: 0.009, width: 0.014, height: 0.022, label: "Assemble" },
  storyboardTool: { left: 0.769, top: 0.009, width: 0.014, height: 0.022, label: "Storyboard" },
  commentsTool: { left: 0.79, top: 0.009, width: 0.014, height: 0.022, label: "Comments" },
  gpuToggle: { left: 0.81, top: 0.009, width: 0.041, height: 0.022, label: "GPU" },

  transcriptTab: { left: 0.004, top: 0.04, width: 0.036, height: 0.024, label: "Transcript" },
  assetsTab: { left: 0.04, top: 0.04, width: 0.036, height: 0.024, label: "Assets" },
  generateTab: { left: 0.077, top: 0.04, width: 0.034, height: 0.024, label: "Generate" },
  libraryTab: { left: 0.111, top: 0.04, width: 0.027, height: 0.024, label: "Library tab" },
  projectsTab: { left: 0.139, top: 0.04, width: 0.03, height: 0.024, label: "Projects tab" },
  findTab: { left: 0.17, top: 0.04, width: 0.02, height: 0.024, label: "Find tab" },
  findMediaTab: { left: 0.004, top: 0.071, width: 0.196, height: 0.022, label: "Find media" },
  findMomentTab: { left: 0.2, top: 0.071, width: 0.196, height: 0.022, label: "Find moment" },
  stockSearch: { left: 0.007, top: 0.271, width: 0.385, height: 0.026, label: "Search field" },
  proxyToggle: { left: 0.111, top: 0.112, width: 0.022, height: 0.02, label: "Proxies" },
  assetSearch: { left: 0.007, top: 0.148, width: 0.29, height: 0.026, label: "Filter field" },
  assetFilters: { left: 0, top: 0.142, width: 0.305, height: 0.065, label: "Kind filters" },
  assetNeedsAction: { left: 0.195, top: 0.176, width: 0.086, height: 0.023, label: "Needs action" },
  mediaHealth: { left: 0, top: 0.207, width: 0.305, height: 0.074, label: "Media Health" },
  importButton: { left: 0.175, top: 0.106, width: 0.052, height: 0.027, label: "Import" },
  generateButton: { left: 0.232, top: 0.106, width: 0.065, height: 0.027, label: "Generate asset" },
  assetCard: { left: 0.006, top: 0.29, width: 0.294, height: 0.082, label: "Asset card" },
  addAtPlayhead: { left: 0.202, top: 0.318, width: 0.07, height: 0.027, label: "Add at playhead" },

  preview: { left: 0.306, top: 0.06, width: 0.457, height: 0.583, label: "Preview" },
  previewSetup: { left: 0.44, top: 0.085, width: 0.2, height: 0.055, label: "FFmpeg setup notice" },
  playbackButtons: { left: 0.48, top: 0.599, width: 0.17, height: 0.044, label: "Playback" },
  frameButton: { left: 0.595, top: 0.603, width: 0.031, height: 0.038, label: "Frame" },
  renderSelectionButton: { left: 0.629, top: 0.603, width: 0.062, height: 0.038, label: "Render selection" },
  audioButton: { left: 0.692, top: 0.603, width: 0.033, height: 0.038, label: "Audio" },
  composedToggle: { left: 0.733, top: 0.603, width: 0.06, height: 0.038, label: "Composed" },
  guidesButton: { left: 0.775, top: 0.604, width: 0.016, height: 0.032, label: "Guides" },
  fullscreenButton: { left: 0.791, top: 0.604, width: 0.016, height: 0.032, label: "Fullscreen" },

  timelineTracks: { left: 0, top: 0.682, width: 0.763, height: 0.273, label: "Timeline" },
  trackControls: { left: 0.004, top: 0.711, width: 0.095, height: 0.18, label: "Track controls" },
  trackVisibility: { left: 0.006, top: 0.714, width: 0.019, height: 0.035, label: "Show / hide" },
  trackOrder: { left: 0.026, top: 0.714, width: 0.038, height: 0.035, label: "Layer order" },
  trackLock: { left: 0.065, top: 0.714, width: 0.026, height: 0.035, label: "Lock" },
  trackMuteSolo: { left: 0.006, top: 0.796, width: 0.047, height: 0.035, label: "Mute / solo" },
  trackListen: { left: 0.054, top: 0.796, width: 0.026, height: 0.035, label: "Listen" },
  trackGain: { left: 0.006, top: 0.835, width: 0.052, height: 0.035, label: "Gain" },
  trackPan: { left: 0.06, top: 0.835, width: 0.037, height: 0.035, label: "Pan" },
  timecode: { left: 0.006, top: 0.645, width: 0.098, height: 0.035, label: "Timecode" },
  razorButton: { left: 0.11, top: 0.635, width: 0.045, height: 0.056, label: "Razor" },
  trimButton: { left: 0.157, top: 0.635, width: 0.04, height: 0.056, label: "Trim" },
  snapButton: { left: 0.2, top: 0.635, width: 0.042, height: 0.056, label: "Snap" },
  rippleButton: { left: 0.251, top: 0.635, width: 0.047, height: 0.056, label: "Ripple del" },
  liftButton: { left: 0.301, top: 0.635, width: 0.036, height: 0.056, label: "Lift del" },
  speedControls: { left: 0.346, top: 0.635, width: 0.126, height: 0.056, label: "Speed" },
  syncButton: { left: 0.481, top: 0.635, width: 0.045, height: 0.056, label: "Sync" },
  multicamButton: { left: 0.536, top: 0.635, width: 0.048, height: 0.056, label: "Multicam" },
  beatButton: { left: 0.589, top: 0.635, width: 0.04, height: 0.056, label: "Beat" },
  timelineCleanupTools: { left: 0.632, top: 0.635, width: 0.18, height: 0.056, label: "Cleanup tools" },
  trimDeadAirButton: { left: 0.632, top: 0.635, width: 0.064, height: 0.056, label: "Trim dead air" },
  splitScenesButton: { left: 0.699, top: 0.635, width: 0.056, height: 0.056, label: "Split scenes" },
  markScenesButton: { left: 0.758, top: 0.635, width: 0.054, height: 0.056, label: "Mark scenes" },
  gradeLayerMatte: { left: 0.632, top: 0.635, width: 0.082, height: 0.056, label: "Grade/Layer/Matte" },
  saveAssetsGif: { left: 0.716, top: 0.635, width: 0.061, height: 0.056, label: "Save/GIF" },

  propertiesTab: { left: 0.769, top: 0.06, width: 0.059, height: 0.035, label: "Properties" },
  colorTab: { left: 0.829, top: 0.06, width: 0.039, height: 0.035, label: "Color" },
  audioTab: { left: 0.87, top: 0.06, width: 0.041, height: 0.035, label: "Audio" },
  chatTab: { left: 0.912, top: 0.06, width: 0.036, height: 0.035, label: "Chat" },
  toolsStrip: { left: 0.764, top: 0.06, width: 0.236, height: 0.896, label: "Tools strip" },
  railPin: { left: 0.96, top: 0.064, width: 0.017, height: 0.027, label: "Pin tools" },
  scoreButton: { left: 0.774, top: 0.273, width: 0.21, height: 0.032, label: "Score clip" },
  fadeSliders: { left: 0.774, top: 0.324, width: 0.21, height: 0.125, label: "Fade sliders" },
  transformSliders: { left: 0.774, top: 0.49, width: 0.21, height: 0.105, label: "Transform sliders" },
  opsTab: { left: 0.764, top: 0.642, width: 0.059, height: 0.033, label: "Ops" },
  receiptsTabs: { left: 0.823, top: 0.642, width: 0.12, height: 0.033, label: "Receipts/QC/Scopes/Diff" },

  cutdStatus: { left: 0.008, top: 0.968, width: 0.055, height: 0.02, label: "cutd status" },
  envStatus: { left: 0.074, top: 0.968, width: 0.115, height: 0.02, label: "Environment" },
  versionStatus: { left: 0.895, top: 0.968, width: 0.1, height: 0.02, label: "Version" },
};

function opened(title, items, left, top, width = 0.16) {
  return { title, items, left, top, width };
}

const opens = {
  settings: opened("Settings", ["System doctor", "FFmpeg path", "Caption tools", "CLI agents"], 0.36, 0.047, 0.18),
  projects: opened("Projects", ["Open project", "Recent projects", "Create project", "Project folder"], 0.252, 0.047, 0.16),
  library: opened("Library", ["Saved assets", "Reusable bins", "Add to project", "Manage library"], 0.319, 0.047, 0.16),
  find: opened("Find", ["Find media", "Find moment", "Sequence Index", "Search field", "Import result"], 0.015, 0.094, 0.28),
  render: opened("Render", ["Quality preset", "Delivery aspect", "Loudness target", "Subject reframe", "Advanced · timeline format"], 0.72, 0.047, 0.17),
  exportMenu: opened("Export", ["Video (.mp4)", "Audio, GIF, still frame", "Publish presets", "Interchange XML / OTIO / EDL", "Captions and transcript", "Render queue / batch", "Default export folder"], 0.82, 0.047, 0.19),
  title: opened("Title", ["Add title", "Templates", "Lower third", "Edit style"], 0.55, 0.047, 0.16),
  shape: opened("Shape", ["Rectangle", "Circle", "Arrow", "Update shape"], 0.57, 0.047, 0.16),
  regionMask: opened("Mask / privacy", ["Blur face", "Blur rectangle", "Hide plate/text", "From playhead", "Custom shape", "Apply mask"], 0.59, 0.047, 0.18),
  music: opened("Music bed", ["Add music", "Pick source", "Loop to fit", "Duck voice"], 0.61, 0.047, 0.16),
  mixer: opened("Audio mixer", ["Track gain", "Pan", "Mute / solo", "EQ", "Cleanup voice"], 0.63, 0.047, 0.16),
  repurpose: opened("Repurpose", ["Find highlights", "Vertical crop", "Shorts variants", "Export set"], 0.65, 0.047, 0.17),
  autopilot: opened("Autopilot", ["Plan edit", "Apply safe ops", "Review changes", "Create receipts"], 0.67, 0.047, 0.17),
  recipes: opened("Recipes", ["Browse recipes", "Edit for clarity", "Phone cleanup", "Social bundle", "Privacy mask", "Add captions", "YouTube export", "TikTok export"], 0.69, 0.047, 0.2),
  assemble: opened("Assemble", ["Prompt", "Source clips", "Build draft", "Insert timeline"], 0.71, 0.047, 0.16),
  storyboard: opened("Storyboard", ["Generate plan", "Review shots", "Preview sequence", "Insert storyboard"], 0.73, 0.047, 0.17),
  comments: opened("Review comments", ["Add comment", "Draft reply", "Apply note", "Resolve"], 0.75, 0.047, 0.17),
  transcript: opened("Transcript", ["Phrase search", "Clip / Program / Source", "Generate captions", "Transcript tools"], 0.01, 0.087, 0.28),
  assets: opened("Assets", ["Media Health", "Readiness badges", "Needs action filter", "Proxy imports", "Relink missing files"], 0.01, 0.087, 0.28),
  generate: opened("Generate", ["Templates", "Prompt plan", "Storyboard plan", "Reference media", "History / compare", "Insert / replace"], 0.01, 0.087, 0.28),
  color: opened("Color", ["Basic grade", "Looks", "Exposure", "Contrast"], 0.825, 0.087, 0.16),
  audio: opened("Audio", ["Gain", "Waveform", "Linked audio", "Ducking"], 0.825, 0.087, 0.16),
  chat: opened("Chat", ["Project context", "Attach assets", "Prompt library", "Ask agent", "Turn review"], 0.825, 0.087, 0.16),
  selectedTools: opened("Selected-clip tools", ["Properties", "Color", "Audio", "Chat", "Pin tools"], 0.79, 0.087, 0.2),
  receipts: opened("Review tabs", ["Receipts", "QC", "Scopes", "Diff", "Accept / reject"], 0.825, 0.665, 0.16),
  record: opened("Record", ["Studio preview", "Camera capture", "Background", "Raw streams", "Hotkeys", "Start / Stop"], 0.19, 0.047, 0.18),
};

function feature(title, where, description, requirement, api, highlight, open) {
  return { title, where, description, requirement, api, highlight, open };
}

function selectedOpen(open, selected) {
  return { ...open, selected };
}

function menuChoice(title, where, description, api, highlight, open, selected, requirement = "Requires an open project.") {
  return feature(title, where, description, requirement, api, highlight, selectedOpen(open, selected));
}

function renderOpenedSurface(popover, open) {
  if (!popover) return;
  if (!open) {
    popover.hidden = true;
    popover.replaceChildren();
    return;
  }

  popover.hidden = false;

  const title = document.createElement("p");
  title.className = "manual-popover-title";
  title.textContent = open.title;

  const list = document.createElement("ul");
  list.className = "manual-popover-list";
  const selected = open.selected || open.items[0];
  open.items.forEach((item, index) => {
    const row = document.createElement("li");
    row.textContent = item;
    if (item === selected || (!open.selected && index === 0)) row.className = "is-primary";
    list.append(row);
  });

  popover.replaceChildren(title, list);

  const surface = popover.closest(".manual-surface");
  const surfaceWidth = surface ? surface.clientWidth : 0;
  const surfaceHeight = surface ? surface.clientHeight : 0;
  const maxWidth = Math.max(120, surfaceWidth - 16);
  const wantedWidth = Math.max(168, surfaceWidth * open.width);
  const popoverWidth = Math.min(260, maxWidth, wantedWidth);
  const left = Math.min(open.left * surfaceWidth, surfaceWidth - popoverWidth - 8);

  popover.style.width = `${popoverWidth}px`;
  popover.style.left = `${Math.max(8, left)}px`;
  popover.style.top = `${Math.max(8, open.top * surfaceHeight)}px`;
}

const features = {
  "cut.setup.ffmpeg": feature(
    "Why FFmpeg is required",
    "First-run setup and Settings",
    "FFmpeg powers probing, preview media, proxies, screen recording, and final renders. If it is missing, Preview shows a setup notice that opens the Video processing card.",
    "Install FFmpeg and make it visible on PATH, or set the binary path in Cut.",
    "system.doctor, system.set_ffmpeg, system.fetch_tool",
    areas.setup,
    opens.settings,
  ),
  "cut.setup.doctor": feature(
    "Run system doctor",
    "Settings",
    "Use the doctor check when Cut cannot import, preview, transcribe, record, or render. It separates missing tools from project problems.",
    "Open Settings and run the readiness check.",
    "system.doctor",
    areas.setup,
    opens.settings,
  ),
  "cut.setup.ffmpeg_path": feature(
    "Set FFmpeg path",
    "Settings",
    "Point Cut at a specific FFmpeg binary when it is installed outside the normal system PATH.",
    "Requires a local ffmpeg executable.",
    "system.set_ffmpeg",
    areas.setup,
    opens.settings,
  ),
  "cut.setup.captions": feature(
    "Install captions and STT",
    "Transcript tab and Settings",
    "Captions need speech-to-text tooling in addition to FFmpeg. Install this when transcript actions show a setup message.",
    "Requires Python plus a supported speech-to-text engine.",
    "system.setup_perception, system.set_stt_model, media.transcribe",
    areas.transcriptTab,
    opens.transcript,
  ),
  "cut.setup.cli_install": feature(
    "Install a CLI agent",
    "Settings",
    "Install at least one supported CLI agent before using AI planning, prompt Generate, storyboard Generate, or review assistance.",
    "Requires the selected CLI tool installed and signed in outside Cut.",
    "system.doctor, system.mcp_test",
    areas.setup,
    opens.settings,
  ),
  "cut.setup.cli_connect": feature(
    "Connect agent in Settings",
    "Settings",
    "After installing the CLI agent, connect it in Cut so Generate, chat, and review workflows can call it.",
    "Requires a working CLI agent command.",
    "system.doctor, system.mcp_test, agent.chat",
    areas.setup,
    opens.settings,
  ),

  "cut.top.projects": feature("Projects", "Top bar", "Open, switch, create, or return to recent Cut projects.", "Project files use .cutproj directories.", "project.list, project.open, project.create", areas.projects, opens.projects),
  "cut.record.open": feature("Record workspace", "Top bar", "Switch from Edit into the Recording Studio workspace for screen capture, background choice, raw streams, and auto-polished recording clips.", "Requires a desktop capture backend; FFmpeg is required for recording and output.", "ui.open {panel:\"record\"}, screen_record.doctor", areas.recordMode, opens.record),
  "cut.top.library": feature("Library", "Top bar", "Open saved media, reusable assets, and library-backed material.", "Requires media already saved to the library.", "library.list, library.add_to_project", areas.libraryTop, opens.library),
  "cut.top.find": feature("Find tab", "Left sidebar", "Search for reusable media and indexed visual moments without opening a header menu.", "Find media works immediately; Find moment works best after video indexing.", "assets.search, assets.fetch, media.search, media.index, ui.open", areas.findTab, opens.find),
  "cut.top.settings": feature("Settings", "Top bar", "Configure tools, environment readiness, agents, paths, and editor preferences.", "Some checks require FFmpeg or CLI tools installed locally.", "system.doctor, system.mcp_test", areas.setup, opens.settings),
  "cut.top.manual": feature("Manual", "Top bar", "Open the current ShellX Cut web manual at docs.theshellx.com/manual/cut/ from inside Cut.", "Requires a browser and network access for the latest online manual.", "CUT_MANUAL_URL, docs.theshellx.com/manual/cut/", areas.manualTop),

  "cut.header.title": feature("Title", "Header tools", "Add title cards, lower thirds, or styled text overlays at the current playhead.", "Requires an open project; title templates are optional.", "title.add, title.templates, title.update", areas.titleTool, opens.title),
  "cut.header.shape": feature("Shape", "Header tools", "Add basic shape overlays such as rectangles, circles, arrows, and callout blocks.", "Requires an open project.", "edit.add_shape, shape.update", areas.shapeTool, opens.shape),
  "cut.header.region_mask": feature("Mask / privacy", "Header tools", "Draw a region for blur, pixelation, black-box privacy, or timed redaction from the playhead.", "Requires a base-track video clip to target.", "edit.add_mask, edit.redact", areas.regionMaskTool, opens.regionMask),
  "cut.header.music": feature("Music bed", "Header tools", "Add or manage a background music layer for the current edit.", "Requires an audio file or available library music.", "audio.add_music, edit.duck", areas.musicTool, opens.music),
  "cut.header.mixer": feature("Audio mixer", "Header tools", "Open audio controls for track gain, pan, mute, solo, cleanup, and mix balance.", "Requires audio tracks for most controls.", "edit.gain, edit.pan, edit.mute, edit.solo, audio.cleanup_voice", areas.mixerTool, opens.mixer),
  "cut.header.repurpose": feature("Repurpose into shorts", "Header tools", "Create short-form variants from highlights, framing, captions, and export presets.", "Works best with indexed media and transcript data.", "clip.candidates, score.clip, render.reframe, render.bundle", areas.repurposeTool, opens.repurpose),
  "cut.header.autopilot": feature("Autopilot", "Header tools", "Let an agent plan safe edits, apply reversible operations, and leave reviewable receipts.", "Requires a configured CLI agent.", "autopilot.run", areas.autopilotTool, opens.autopilot),
  "cut.header.recipes": feature("Recipes", "Header tools", "Browse and run repeatable edit recipes for common production tasks: phone cleanup, social bundle, privacy mask, captions, YouTube export, and TikTok export. Timeline-changing recipes ask you to preview the exact plan before Run is enabled.", "Recipe availability depends on the current project state.", "recipe.list, recipe.describe, recipe.run", areas.recipesTool, opens.recipes),
  "cut.header.assemble": feature("Assemble AI", "Header tools", "Ask an agent to assemble a rough cut from prompts, selected media, or storyboard intent.", "Requires a configured CLI agent and source media.", "assemble.repurpose, assemble.shorts, assemble.from_script, assemble.broll", areas.assembleTool, opens.assemble),
  "cut.header.storyboard": feature("Storyboard", "Header tools", "Generate or inspect storyboard plans before inserting a sequence into the timeline.", "Prompt/storyboard generation needs a configured CLI agent.", "generate.storyboard, generate.preview, generate.insert", areas.storyboardTool, opens.storyboard),
  "cut.header.comments": feature("Review comments", "Header tools", "Open review notes, add comments, apply comment-driven changes, or resolve feedback.", "Requires project review context for existing comments.", "comment.add, comment.list, comment.apply, comment.resolve", areas.commentsTool, opens.comments),
  "cut.header.gpu": feature("Faster exports (hardware encoding)", "Header tools", "The Faster ON / Faster OFF chip picks the encoder for renders and exports. On uses available video hardware for speed; off forces repeatable software encoding, which is what you want when two renders must match byte for byte.", "Requires compatible video hardware and driver support for the ON state.", "system.doctor, render.final {hardware}", areas.gpuToggle),

  "cut.top.render": feature("Render", "Top bar", "Render the current project or selected range using the active project settings.", "Requires FFmpeg and a valid output folder.", "render.preview, render.final", areas.render, opens.render),
  "cut.top.export": feature("Export menu", "Top bar", "Choose delivery targets, output presets, and platform-oriented export paths.", "Requires a renderable timeline.", "render.final, export.publish, export.xml, render.queue", areas.exportMenu, opens.exportMenu),

  "cut.left.transcript": feature("Transcript tab", "Left sidebar", "Work with phrases, transcript modes, caption generation, and transcript tools.", "Requires speech-to-text tooling for generated transcripts.", "media.transcribe, transcript.timeline", areas.transcriptTab, opens.transcript),
  "cut.left.assets": feature("Assets tab", "Left sidebar", "Use Assets to confirm imported media, read per-clip readiness, filter by type or action needed, and add clips to the base timeline at the playhead.", "Requires a project and at least one imported media file.", "media.import, media.check, library.add_to_project", areas.assetsTab, opens.assets),
  "cut.left.asset_filters": feature("Asset filters", "Assets tab", "Filter imported media by kind, unused state, 4K, missing files, recent changes, or clips that need action.", "Requires imported media.", "media.search, media.bin_list, media.check", areas.assetFilters),
  "cut.left.asset_needs_action": feature("Needs action filter", "Assets tab", "Show only media that needs a user decision, such as relinking a missing file or handling a large source-only clip.", "Requires imported media. The filter is view-only and does not change project media.", "media.check, proxy preference", areas.assetNeedsAction, opens.assets),
  "cut.left.media_health": feature("Media Health", "Assets tab", "Shows whether source files are missing, which large clips are using source playback, how many proxies are ready, and how many items need action.", "Requires imported media. Use Relink when a source file moved or was renamed.", "media.check, media.relink, proxy preference", areas.mediaHealth, opens.assets),
  "cut.left.proxies": feature("Proxy imports", "Assets tab", "Turn proxy generation on for future imports when editing 4K, phone, or camera files that preview roughly from source media.", "Requires FFmpeg and newly imported video files.", "media.import {proxy:true}, media.filmstrip", areas.proxyToggle, opens.assets),
  "cut.left.import": feature("Import media", "Assets tab", "Bring video, audio, and image files into the current project. The first import can become the starting timeline.", "Requires readable local media and FFmpeg for probe/proxy work.", "media.import, media.probe", areas.importButton),
  "cut.left.add_at_playhead": feature("Add at playhead", "Asset card", "Insert an asset at the current playhead on the base story timeline. Linked audio is placed with the video so preview and export stay audible.", "Requires an imported asset and an open project.", "edit.insert", areas.addAtPlayhead),
  "cut.left.generate": feature("Generate tab", "Left sidebar", "Create prompt plans, storyboard material, generated inserts, and reusable generated assets.", "Prompt and storyboard planning need a configured CLI agent.", "generate.list, generate.preview, generate.insert, generate.from_prompt, generate.storyboard", areas.generateTab, opens.generate),
  "cut.left.generated_history": feature("Generated history and compare", "Generate tab", "Review integrity-checked project generation history, compare takes, choose one, then insert it or replace the selected clip without regenerating.", "Requires at least one completed generated image or video in the open project.", "assets.generated_list, edit.insert, edit.replace", areas.generateTab, selectedOpen(opens.generate, "History / compare")),
  "cut.left.generated_references": feature("References and variations", "Generate tab", "Attach up to four registered project image or video assets as generation references and label the requested variation. Retry reuses the saved provenance and never exposes arbitrary source paths.", "Provider-backed generation requires the selected local generation CLI and may spend provider quota only after confirmation.", "assets.generate, assets.generated_list", areas.generateTab, selectedOpen(opens.generate, "Reference media")),
  "cut.left.motion_edit": feature("Edit in Motion", "Timeline and Inspector", "Open the selected linked Motion package in Canvas without exposing the package or return-request paths. Canvas publishes a new immutable ready revision only after its render is verified.", "Requires a selected Motion-linked clip and an installed or configured ShellX Canvas editor.", "motion.link.edit", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.left.motion_refresh": feature("Refresh linked Motion render", "Timeline and Inspector", "Adopt the newest verified Canvas return for the same package, motion identity, and authored source revision, replacing pixels in the existing Cut clip without changing its editorial identity.", "Run Edit in Motion and complete a verified Canvas render first. A mismatch or failed render leaves the last good Cut clip untouched.", "motion.link.refresh, project.undo", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.left.motion_tracking": feature("Track and stabilize linked footage", "Inspector", "Choose manifest-declared footage and a visual target, analyze a point or planar region, apply ordinary Motion transform keyframes, verify the attachment, or detach back to the exact prior keyframes.", "Requires a Motion-linked package with footage and a target visual layer. Refresh remains explicit after applying or detaching.", "motion.link.tracking.inventory, motion.link.tracking.request, motion.link.tracking.apply, motion.link.tracking.verify, motion.link.tracking.detach", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.left.find": feature("Find tab", "Left sidebar", "Keep search available at all times in the sidebar. Use it for reusable media, local folders, and indexed visual moments.", "Find media needs no project-specific indexing; Find moment needs imported video and indexing for content search.", "assets.search, assets.fetch, media.index, media.search, ui.open", areas.findTab, opens.find),
  "cut.left.find.media": menuChoice("Find media", "Find tab", "Search reusable media providers or a local folder, then import a result into the open project.", "assets.search, assets.fetch", areas.findMediaTab, opens.find, "Find media"),
  "cut.left.find.moment": menuChoice("Find moment", "Find tab", "Search indexed frames inside the project to jump to matching visual moments.", "media.index, media.search", areas.findMomentTab, opens.find, "Find moment", "Requires imported video; index the clip before content search."),
  "cut.left.sequence_index": menuChoice("Sequence Index", "Find tab", "Search clips and markers across every project sequence, filter by result kind, sequence, or track, then switch sequences and seek the exact result time.", "project.sequence_index, project.sequence_switch, ui.playhead", areas.findTab, opens.find, "Sequence Index", "Requires an open project; results remain path-light and project-scoped."),

  "cut.preview.monitor": feature("Video preview", "Center editor surface", "Preview the current frame, selected edit, and rendered composition while you work.", "Requires imported media for real playback.", "ui.screenshot, render.preview", areas.preview),
  "cut.preview.ffmpeg_setup": feature("Preview setup notice", "Preview monitor", "When FFmpeg is missing, Preview shows a clear setup notice with an Install action that opens the Video processing card.", "Requires the system doctor to confirm FFmpeg is missing.", "system.doctor, system.fetch_tool, ui.highlight", areas.previewSetup),
  "cut.preview.transport": feature("Play controls", "Preview monitor", "Jump to start or end, shuttle backward or forward, and play or pause the timeline.", "Requires an open timeline.", "ui.playhead (play/pause is a preview transport, not a verb)", areas.playbackButtons),
  "cut.preview.frame": feature("Frame button", "Preview controls", "Capture or save the current preview frame for review or reuse.", "Requires a visible preview frame.", "render.frame, media.import", areas.frameButton),
  "cut.preview.render_selection": feature("Render selection", "Preview controls", "Render only the selected range when a timeline range is active.", "Requires an export range on the ruler.", "render.preview", areas.renderSelectionButton),
  "cut.preview.audio": feature("Audio monitor", "Preview controls", "Toggle and monitor preview audio while checking cuts, sync, and narration.", "Requires audio in the timeline for level movement.", "media.waveform, edit.gain", areas.audioButton),
  "cut.preview.composed": feature("Composed toggle", "Preview controls", "Switch composed preview on or off when checking generated frames, overlays, or render previews.", "Depends on the active render or preview mode.", "render.preview", areas.composedToggle),
  "cut.preview.guides": feature("Guides", "Preview controls", "Cycle visual guides for safe areas, thirds, or both when framing titles and overlays.", "No setup required.", "no verb — a preview-only control", areas.guidesButton),
  "cut.preview.fullscreen": feature("Full-screen preview", "Preview controls", "Expand the monitor when checking focus, titles, caption placement, or visual defects.", "No setup required.", "no verb — a preview-only control", areas.fullscreenButton),

  "cut.timeline.timecode": feature("Timecode", "Timeline", "Read or jump the current playhead time while trimming and reviewing edits.", "Requires an open timeline.", "ui.playhead", areas.timecode),
  "cut.timeline.razor": feature("Razor", "Timeline toolbar", "Split a clip at the playhead so each side can be moved, trimmed, graded, or deleted independently.", "Requires a clip under the playhead.", "edit.split", areas.razorButton),
  "cut.timeline.trim": feature("Trim", "Timeline toolbar", "Adjust clip in and out points while keeping the edit on the timeline.", "Requires a selected clip or trim handle.", "edit.trim", areas.trimButton),
  "cut.timeline.snap": feature("Snap", "Timeline toolbar", "Toggle magnetic alignment to clip edges, markers, and playhead positions.", "No setup required.", "edit.move, edit.trim", areas.snapButton),
  "cut.timeline.ripple": feature("Ripple delete", "Timeline toolbar", "Delete a selection and close the gap so following clips move left.", "Requires a selected clip or range.", "edit.ripple_delete", areas.rippleButton),
  "cut.timeline.lift": feature("Lift delete", "Timeline toolbar", "Delete a selection while leaving a gap in its original time span.", "Requires a selected clip or range.", "edit.ripple_delete {ripple:false}", areas.liftButton),
  "cut.timeline.speed": feature("Speed controls", "Timeline toolbar", "Retiming controls change clip playback speed while preserving the edit span rules.", "Requires a selected clip.", "edit.speed", areas.speedControls),
  "cut.timeline.sync": feature("Sync by audio", "Timeline toolbar", "Align clips using their audio waveforms when matching camera or recorder sources.", "Requires clips with usable audio.", "edit.multicam_sync", areas.syncButton),
  "cut.timeline.multicam": feature("Auto multicam", "Timeline toolbar", "Build a multicam-style alignment from multiple sources when their audio can be matched.", "Requires multiple compatible clips.", "edit.multicam_switch", areas.multicamButton),
  "cut.timeline.beat": feature("Cut to beat", "Timeline toolbar", "Place cuts or timing choices against detected beats in the audio.", "Requires analyzed audio.", "edit.cut_to_beat", areas.beatButton),
  "cut.timeline.cleanup_tools": feature("Cleanup tools", "Timeline toolbar", "Run cleanup and scene-detection actions directly beside editing tools instead of opening a top-bar menu.", "Requires an open project. Scene actions need at least one imported video asset.", "edit.trim_edges, edit.split_at_scenes, edit.mark_scenes", areas.timelineCleanupTools),
  "cut.timeline.trim_dead_air": feature("Trim dead air", "Timeline toolbar", "Trim silence from the beginning and end of the current timeline while keeping the operation reversible.", "Requires an open timeline with audio analysis available.", "edit.trim_edges", areas.trimDeadAirButton),
  "cut.timeline.split_scenes": feature("Split scenes", "Timeline toolbar", "Split the first imported video asset at detected scene cuts so each scene can be moved, trimmed, or deleted.", "Requires an imported video asset with scene detection data.", "edit.split_at_scenes", areas.splitScenesButton),
  "cut.timeline.mark_scenes": feature("Mark scenes", "Timeline toolbar", "Add timeline markers at detected scene cuts without changing clip timing.", "Requires an imported video asset with scene detection data.", "edit.mark_scenes", areas.markScenesButton),
  "cut.timeline.grade": feature("Grade, Layer, Matte", "Timeline toolbar", "Open visual editing groups for color, compositing, layer behavior, and matte work.", "Requires a selected visual clip.", "edit.grade, edit.effect", areas.gradeLayerMatte),
  "cut.timeline.track_controls": feature("Track controls", "Timeline track header", "Use lane headers for show/hide, lock, layer order, mute, solo, listen, gain, and pan without opening the mixer first.", "Requires at least one timeline track; controls vary by track kind.", "edit.track_visible, edit.track_lock, edit.reorder_track, edit.mute, edit.solo, edit.gain, edit.pan, export.audio", areas.trackControls),
  "cut.timeline.track_visibility": feature("Show or hide a track", "Timeline track header", "Hide a video or caption lane from preview and export without deleting its clips. Use mute for audio tracks.", "Requires a video or caption track.", "edit.track_visible", areas.trackVisibility),
  "cut.timeline.track_lock": feature("Lock track edits", "Timeline track header", "Lock a lane when it should stay in place. Drag/drop, trim, move, split targets, delete, and context-menu edits skip locked tracks until you unlock them.", "Works on any timeline track.", "edit.track_lock", areas.trackLock),
  "cut.timeline.track_order": feature("Layer order", "Timeline track header", "Send an overlay video track backward or bring it forward in the visual stack.", "Requires more than one video track.", "edit.reorder_track", areas.trackOrder),
  "cut.timeline.track_mute": feature("Mute track", "Timeline track header", "Silence an audio-bearing track without changing its gain value.", "Requires an audio-bearing track.", "edit.mute", areas.trackMuteSolo),
  "cut.timeline.track_solo": feature("Solo track", "Timeline track header", "Hear only soloed tracks while checking a mix. Explicit mute still wins.", "Requires an audio-bearing track.", "edit.solo", areas.trackMuteSolo),
  "cut.timeline.track_listen": feature("Listen to one track", "Timeline track header", "Render or check the selected audio track by itself when diagnosing a mix.", "Requires an audio track.", "export.audio", areas.trackListen),
  "cut.timeline.track_gain": feature("Track gain", "Timeline track header", "Set a compact per-track gain value directly from the lane header.", "Requires an audio-bearing track.", "edit.gain", areas.trackGain),
  "cut.timeline.track_pan": feature("Track pan", "Timeline track header", "Set common pan positions from the lane header: left, center, right, or half-left/half-right.", "Requires an audio-bearing track.", "edit.pan", areas.trackPan),
  "cut.timeline.base_overlay": feature("Base track and overlays", "Timeline tracks", "Normal Insert and normal drops build the base story timeline and ripple later clips. Extra video tracks are overlays: use Alt-drag, a new overlay lane, or an existing overlay lane when the clip should appear on top.", "Requires an open project; overlay placement requires an overlay lane or Alt-drag.", "edit.insert, edit.add_track", areas.timelineTracks),
  "cut.timeline.save_assets": feature("Save to Assets and GIF", "Timeline toolbar", "Save selected output back into project assets or create a GIF from the current selection.", "Requires a selected range or renderable clip.", "media.import, render.preview", areas.saveAssetsGif),

  "cut.inspector.properties": feature("Properties tab", "Inspector", "Show selected clip details, engagement scoring, fades, clip actions, and transform controls.", "Requires a selected clip for clip-specific controls.", "ui.select, edit.fade, edit.transform", areas.propertiesTab),
  "cut.inspector.color": feature("Color tab", "Inspector", "Adjust grade and color controls for the selected visual clip.", "Requires a selected visual clip.", "edit.grade", areas.colorTab, opens.color),
  "cut.inspector.audio": feature("Audio tab", "Inspector", "Adjust audio controls, gain, and related clip audio settings.", "Requires an audio clip or linked audio.", "edit.gain, media.waveform", areas.audioTab, opens.audio),
  "cut.inspector.chat": feature("Chat tab", "Inspector", "Use the connected agent to discuss the current project or selected edit context.", "Requires a configured CLI agent.", "agent.chat", areas.chatTab, opens.chat),
  "cut.inspector.chat_assets": feature("Attach project assets", "Chat tab", "Attach up to eight registered project assets to a turn. Cut validates the IDs, shows them on the request, and never accepts an arbitrary source path through Chat.", "Requires an open project with imported assets and a configured CLI agent.", "agent.chat {attachments:[asset_ids]}", areas.chatTab, selectedOpen(opens.chat, "Attach assets")),
  "cut.inspector.chat_review": feature("Review each agent turn", "Chat tab and Review Diff", "Every editing turn records its plan, baseline, tip, exact diff, and safe-revert verdict. Preview or inspect Diff, then Accept, Revert, or Try again. Concurrent human or agent operations disable whole-turn revert.", "Requires an agent turn that applied at least one operation.", "agent.chat, project.diff, project.revert", areas.chatTab, selectedOpen(opens.chat, "Turn review")),
  "cut.inspector.tools_overlay": feature("Tools overlay", "Right edge", "Open selected-clip tools without permanently narrowing the timeline. The overlay closes with the close button, Escape, or an outside click.", "No setup required.", "ui.open, ui.highlight", areas.toolsStrip, opens.selectedTools),
  "cut.inspector.pin": feature("Pin tools", "Tools header", "Pin the selected-clip tools beside the editor when you want a persistent inspector; unpin to return to the full-width timeline.", "Open the Tools overlay first.", "ui.state", areas.railPin, opens.selectedTools),
  "cut.inspector.fades": feature("Fades", "Inspector Properties", "Set fade-in and fade-out values for the selected clip.", "Requires a selected clip.", "edit.fade", areas.fadeSliders),
  "cut.inspector.transform": feature("Transform", "Inspector Properties", "Adjust position, scale, crop, and related visual transform settings.", "Requires a selected visual clip.", "edit.transform, edit.effect", areas.transformSliders),
  "cut.review.ops": feature("Review Ops", "Review panel", "Read every applied operation so edits remain auditable and reversible.", "Requires project operations in the current session or file.", "project.ops, project.undo, project.redo", areas.opsTab),
  "cut.review.receipts": feature("Receipts, QC, Scopes, Diff", "Review panel", "Switch review tabs to inspect receipts, quality checks, video scopes, and project diffs before delivery.", "Some tabs depend on completed review, render, or scope-check jobs.", "project.diff, verify.checks, verify.scopes, ui.open", areas.receiptsTabs, opens.receipts),
  "cut.review.scopes": feature("Video scopes", "Review panel", "Run an objective frame check for luma, saturation, white balance, broadcast range, and clipping. Turn on images when you want vectorscope, waveform, or histogram evidence. Agents and tests can open this exact tab with ui.open {panel:\"scopes\"}.", "Requires an open project with a renderable frame and FFmpeg.", "verify.scopes, ui.open {panel:\"scopes\"}", areas.receiptsTabs, selectedOpen(opens.receipts, "Scopes")),

  "cut.record.studio": feature("Studio preview", "Record workspace", "The large Studio preview shows the screen composition and the elapsed recording state before and during capture.", "Requires the Record workspace. The preview is a composition guide; real capture still depends on OS screen permissions.", "screen_record.doctor, screen_record.start", areas.recordMode, selectedOpen(opens.record, "Studio preview")),
  "cut.record.camera_enable": feature("Camera overlay", "Record workspace", "Live camera capture is not available in this release. The Record workspace says so where the controls used to be, and no enable, position, size or shape control is shown. Screen, microphone and supported system-audio recording are unaffected.", "Nothing to enable — plan a webcam pass as a separate recording for now.", "screen_record.start (no webcam stream in this release)", areas.recordMode, selectedOpen(opens.record, "Camera capture")),
  "cut.record.camera_visible": feature("Show or hide camera", "Record workspace", "Not available in this release: with no live camera stream there is nothing to show or hide. The F10 binding stays in the keymap but has no effect.", "Returns when live camera capture returns.", "screen_record.studio_event {source:\"camera\", kind:\"visibility\"} — accepted by the API, no live stream to drive it", areas.recordMode, selectedOpen(opens.record, "Camera capture")),
  "cut.record.camera_position": feature("Camera position", "Record workspace", "Not available in this release. The F11 and Shift+F11 bindings stay in the keymap but have no effect while there is no camera stream.", "Returns when live camera capture returns.", "screen_record.studio_event {source:\"camera\", kind:\"transform\", x, y} — accepted by the API, no live stream to drive it", areas.recordMode, selectedOpen(opens.record, "Camera capture")),
  "cut.record.camera_size": feature("Camera size", "Record workspace", "Not available in this release. Camera size is still a timed Studio event in the API, but live capture produces no camera stream to size.", "Returns when live camera capture returns.", "screen_record.studio_event {source:\"camera\", kind:\"transform\", size}", areas.recordMode, selectedOpen(opens.record, "Camera capture")),
  "cut.record.camera_shape": feature("Camera shape", "Record workspace", "Not available in this release. Circle and rounded-rectangle shaping remains in the compositor for a camera file supplied by an agent, but the recorder captures no camera of its own.", "Returns when live camera capture returns.", "screen_record.studio_event {source:\"camera\", kind:\"transform\", shape}", areas.recordMode, selectedOpen(opens.record, "Camera capture")),
  "cut.record.background": feature("Background", "Record workspace", "Choose the recording background style for polished screen-demo output while raw streams remain untouched.", "Works with auto-edit/polish recording output.", "screen_record.studio_event {source:\"background\", kind:\"style\"}", areas.recordMode, selectedOpen(opens.record, "Background")),
  "cut.record.raw_streams": feature("Raw streams", "Record workspace", "After a capture, Cut reports the raw screen, microphone, system audio, and Studio event artifacts so you can diagnose or reuse what was recorded. The camera chip stays dark: live camera capture is not available in this release. On Windows 10 build 20348 or newer, system audio uses endpoint-independent process loopback; if security software blocks it, screen and microphone capture continue and the missing stream is reported.", "Requires a completed capture. A new Windows build may need one audio-capture approval from security software.", "screen_record.stop raw_streams", areas.recordMode, selectedOpen(opens.record, "Raw streams")),
  "cut.record.hotkeys": feature("Recording hotkeys", "Record workspace", "F9 starts or stops recording and F12 drops a marker — the two the Studio shows. The camera bindings (F10, F11, Shift+F11) remain in the keymap but do nothing while live camera capture is unavailable.", "Global F9 is available in the desktop app; focused-window fallback works while Cut has focus.", "screen_record.start, screen_record.stop, screen_record.studio_event", areas.recordMode, selectedOpen(opens.record, "Hotkeys")),
  "cut.record.raw_mode": feature("Raw capture mode", "Record workspace", "Raw capture saves the recording as captured without auto-edit or polish, then lets you add the saved file to the timeline manually.", "Requires FFmpeg and a writable output location.", "screen_record.stop {mux_raw:true}, media.import", areas.recordMode, selectedOpen(opens.record, "Raw streams")),
  "cut.record.autoedit": feature("Auto-edit recording", "Record workspace", "Auto-edit stops the capture, builds a recorder plan, replays Studio camera/background metadata, polishes the clip, and places it on the timeline.", "Requires FFmpeg and a finalized capture. Background and marker metadata is replayed from the capture's Studio events; there is no camera layer in this release.", "screen_record.stop {autoedit:true}, screen_record.autoedit, screen_record.polish", areas.recordMode, selectedOpen(opens.record, "Start / Stop")),
  "cut.record.studio_event_api": feature("Studio event API", "Debug API", "Agents and the UI append timed Studio events for background style and recording markers. The camera event sources are still accepted by the API for a camera file supplied by an agent, but live capture emits no camera stream in this release.", "Requires an open project and a valid capture_id returned by screen_record.start.", "screen_record.studio_event", areas.recordMode, selectedOpen(opens.record, "Background")),

  "cut.workflow.import": feature("Import and organize media", "Assets tab and timeline", "Import a file, verify it in Assets, then add it at the playhead or drag it into the base timeline.", "Requires readable media and FFmpeg.", "media.import, edit.insert", areas.importButton),
  "cut.workflow.base_overlay": feature("Build the base timeline, then overlays", "Assets tab and timeline", "Drop ordinary clips onto the base timeline first. Use Alt-drag or drop on an overlay lane for B-roll, picture-in-picture, titles, masks, or any video that should composite above the main story.", "Requires imported media; overlays need a video overlay lane.", "edit.insert, edit.add_track, edit.transform", areas.timelineTracks),
  "cut.workflow.split_trim": feature("Split and trim clips", "Timeline", "Use Razor, Trim, Snap, Ripple delete, and Lift delete to shape the sequence.", "Requires clips on the timeline.", "edit.split, edit.trim, edit.ripple_delete {ripple:false}", areas.razorButton),
  "cut.workflow.captions": feature("Captions and transcript", "Transcript tab and Inspector", "Generate or import captions, style them, and translate caption or transcript text.", "Requires speech-to-text setup for generated transcript content.", "media.transcribe, captions.import, transcript.timeline", areas.transcriptTab, opens.transcript),
  "cut.workflow.edit_for_clarity": feature("Edit for Clarity", "Recipes", "Preview and run the conservative clarity pass: transcribe, analyze pauses, remove retakes and fillers, then tighten pauses at the chosen intensity without forcing a delivery render.", "Requires an open project with speech media and local speech-to-text readiness.", "recipe.describe {name:\"edit-for-clarity\"}, recipe.run", areas.recipesTool, selectedOpen(opens.recipes, "Edit for clarity")),
  "cut.workflow.generate": feature("Prompt and storyboard Generate", "Generate tab", "Use prompt and storyboard flows to create planned inserts, generated assets, and template-backed material.", "Requires a configured CLI agent for planning flows.", "generate.from_prompt, generate.storyboard, generate.insert", areas.generateTab, opens.generate),
  "cut.workflow.generated_media": feature("Compare and place generated media", "Generate tab", "Choose registered references, request a labelled variation, compare completed takes, select one, and insert or replace it from verified project history. A cancelled placement can be retried explicitly.", "Requires a configured generation provider and an open project.", "assets.generate, assets.generated_list, edit.insert, edit.replace", areas.generateTab, selectedOpen(opens.generate, "History / compare")),
  "cut.workflow.motion_roundtrip": feature("Edit a linked clip in Motion", "Timeline, Inspector, and Canvas", "Select a Motion-linked clip, choose Edit in Motion, make rich source changes in Canvas, render the copy-on-write revision, return to Cut, and refresh the same clip. Cut rechecks package identity, authored revision, receipt identity, and media hash before replacing the linked render.", "Requires ShellX Canvas and a current linked package. Refresh is explicit; stale or mismatched handbacks never replace the last good render.", "motion.link.edit, motion.link.refresh, motion.link.relink, project.undo", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.workflow.sequence_index": feature("Search across sequences", "Find tab: Sequence Index", "Search clips and markers project-wide, narrow by kind, sequence, or track, and open a result to switch sequence and seek its time.", "Requires an open project with one or more sequences.", "project.sequence_index, project.sequence_switch, ui.playhead", areas.findTab, selectedOpen(opens.find, "Sequence Index")),
  "cut.workflow.agent_review": feature("Review an agent turn", "Chat tab and Review Diff", "Attach registered project assets, choose or edit a prompt, run the turn, inspect its composed Preview and exact Diff, then Accept, guarded Revert, or Try again.", "Requires a configured CLI agent; whole-turn revert is available only when no concurrent operation crossed the turn boundary.", "agent.chat, project.diff, project.revert", areas.chatTab, selectedOpen(opens.chat, "Turn review")),
  "cut.workflow.recording": feature("Record, polish, and export", "Record workspace", "Open Record, confirm FFmpeg/readiness, choose the source, audio and background, start capture, stop it, then let auto-edit polish the result or save raw streams for manual work. Windows system audio uses endpoint-independent process loopback on supported builds; live camera capture is unavailable in this release.", "Requires desktop capture permission and FFmpeg; system audio is optional. Security software may ask to allow audio capture for a new Windows build.", "screen_record.doctor, screen_record.start, screen_record.studio_event, screen_record.stop, screen_record.autoedit, screen_record.polish, screen_record.export", areas.recordMode, opens.record),
  "cut.workflow.review": feature("Review receipts", "Review panel", "Check operation history, receipts, QC, scopes, and diffs before accepting a change or rendering final output.", "Requires project operations or completed checks.", "project.ops, project.diff, verify.checks, verify.scopes", areas.opsTab),
  "cut.workflow.export": feature("Render and export", "Top bar", "Render a preview, final file, range, or delivery bundle from the active project.", "Requires FFmpeg and a valid output folder.", "render.preview, render.final, export.publish", areas.render),

  "cut.api.debug": feature("Debug API overview", "cutd local API", "Use the Debug API when you need repeatable automation, integration tests, external inspection, or MCP access to Cut.", "Requires a running cutd server bound to loopback.", "POST /api/verb/{name}, GET /api/verbs, cutd mcp", areas.cutdStatus),
  "cut.api.rest": feature("REST verbs", "cutd local API", "Dispatch Cut actions through the local REST endpoint when scripting, testing, or integrating with another tool.", "Requires a running cutd server bound to loopback.", "POST /api/verb/{name}", areas.cutdStatus),
  "cut.api.state": feature("Project state", "cutd local API", "Read the current project, assets, tracks, clips, markers, and settings.", "Requires an open project for timeline data.", "GET /api/state, project.state", areas.versionStatus),
  "cut.api.events": feature("WebSocket events", "cutd local API", "Subscribe to operation, job, and UI-relevant state changes while Cut is running.", "Requires a running cutd server.", "GET /api/events", areas.envStatus),
  "cut.api.mcp": feature("MCP server", "cutd command line", "Expose Cut verbs to MCP-capable clients through the cutd MCP command.", "Requires cutd installed and a running Cut server for proxy mode.", "cutd mcp", areas.cutdStatus),
  "cut.api.motion_jobs": feature("Observe a Motion render", "Debug API or MCP", "Choose a job_id before starting a blocking Motion-backed render, then query that same id from another request without exposing another project's jobs or Motion runtime paths.", "Requires an open Cut project and a current ShellX Motion CLI. Poll no faster than pollAfterMs and stop when it disappears.", "motion.template_to_cut, motion.script_to_cut, motion.link.refresh, motion.job.get, motion.job.list", areas.cutdStatus),
  "cut.api.catalog": feature("Verb catalog", "cutd local API", "Read the machine-readable verb contract used by tools and docs.", "Requires a running cutd server or local schema file.", "GET /api/verbs, schema/verbs.json", areas.versionStatus),
};

Object.assign(features, {
  "cut.header.title.add": menuChoice("Add title", "Title menu", "Add a text title at the current playhead.", "title.add", areas.titleTool, opens.title, "Add title"),
  "cut.header.title.templates": menuChoice("Title templates", "Title menu", "Choose a saved or built-in title style before inserting text.", "title.templates", areas.titleTool, opens.title, "Templates"),
  "cut.header.title.lower_third": menuChoice("Lower third", "Title menu", "Add a lower-third title treatment for names, speakers, or short labels.", "title.add, title.update", areas.titleTool, opens.title, "Lower third"),

  "cut.header.shape.rectangle": menuChoice("Rectangle shape", "Shape menu", "Insert a rectangle overlay for cards, callouts, masks, or emphasis.", "edit.add_shape, shape.update", areas.shapeTool, opens.shape, "Rectangle"),
  "cut.header.shape.circle": menuChoice("Circle shape", "Shape menu", "Insert a circular overlay or callout shape.", "edit.add_shape, shape.update", areas.shapeTool, opens.shape, "Circle"),
  "cut.header.shape.arrow": menuChoice("Arrow shape", "Shape menu", "Insert an arrow overlay to point at something in the frame.", "edit.add_shape, shape.update", areas.shapeTool, opens.shape, "Arrow"),

  "cut.header.region_mask.face": menuChoice("Blur face", "Mask / privacy drawer", "Choose Blur face, draw an oval over the face in the preview, and apply a soft privacy blur.", "edit.add_mask, edit.redact", areas.regionMaskTool, opens.regionMask, "Blur face", "Requires a base-track video clip."),
  "cut.header.region_mask.rectangle": menuChoice("Blur rectangle", "Mask / privacy drawer", "Choose Blur rectangle for screen areas, labels, or objects that need a rectangular blur.", "edit.add_mask, edit.redact", areas.regionMaskTool, opens.regionMask, "Blur rectangle", "Requires a base-track video clip."),
  "cut.header.region_mask.plate": menuChoice("Hide plate/text", "Mask / privacy drawer", "Choose Hide plate/text to black out a license plate, address, password, or other visible private text.", "edit.add_mask, edit.redact", areas.regionMaskTool, opens.regionMask, "Hide plate/text", "Requires a base-track video clip."),
  "cut.header.region_mask.duration": menuChoice("Timed privacy from playhead", "Mask / privacy drawer", "Switch Duration to From playhead and set the number of seconds when the private detail is visible.", "edit.redact {range_ms}", areas.regionMaskTool, opens.regionMask, "From playhead", "Requires the playhead to be inside the selected clip."),
  "cut.header.region_mask.custom": menuChoice("Custom mask shape", "Mask / privacy drawer", "Use rectangle, ellipse, or polygon with blur, pixelate, or black box effects for a custom region.", "edit.add_mask", areas.regionMaskTool, opens.regionMask, "Custom shape", "Requires a drawn preview region."),
  "cut.header.region_mask.apply": menuChoice("Apply mask", "Mask / privacy drawer", "Apply the current mask and switch preview to the composed frame so the result is visible.", "edit.add_mask, edit.redact", areas.regionMaskTool, opens.regionMask, "Apply mask", "Requires a configured mask region."),

  "cut.header.music.add": menuChoice("Add music", "Music bed menu", "Add a background music layer to the edit.", "audio.add_music", areas.musicTool, opens.music, "Add music", "Requires a local audio file or available library music."),
  "cut.header.music.duck": menuChoice("Duck voice", "Music bed menu", "Lower the music bed under speech so narration remains clear.", "edit.duck", areas.musicTool, opens.music, "Duck voice", "Requires speech or narration audio plus a music layer."),

  "cut.header.mixer.gain": menuChoice("Track gain", "Audio mixer menu", "Adjust gain on the selected track or clip.", "edit.gain", areas.mixerTool, opens.mixer, "Track gain", "Requires an audio track or linked audio."),
  "cut.header.mixer.pan": menuChoice("Pan", "Audio mixer menu", "Set stereo balance for an audio-bearing track without changing gain.", "edit.pan", areas.mixerTool, opens.mixer, "Pan", "Requires an audio track or linked audio."),
  "cut.header.mixer.mute_solo": menuChoice("Mute or solo", "Audio mixer menu", "Mute a track or solo it while checking a mix.", "edit.mute, edit.solo", areas.mixerTool, opens.mixer, "Mute / solo", "Requires an audio track."),
  "cut.header.mixer.eq": menuChoice("EQ", "Audio mixer menu", "Apply equalizer adjustments to improve voice, music, or source audio.", "edit.eq", areas.mixerTool, opens.mixer, "EQ", "Requires an audio clip or track."),
  "cut.header.mixer.cleanup": menuChoice("Cleanup voice", "Audio mixer menu", "Run voice cleanup on speech material before final delivery.", "audio.cleanup_voice", areas.mixerTool, opens.mixer, "Cleanup voice", "Requires speech audio."),

  "cut.header.repurpose.highlights": menuChoice("Find highlights", "Repurpose menu", "Find likely short-form highlights from the current edit or source media.", "clip.candidates, score.clip", areas.repurposeTool, opens.repurpose, "Find highlights", "Works best with indexed or transcribed media."),
  "cut.header.repurpose.vertical": menuChoice("Vertical crop", "Repurpose menu", "Create vertical framing for shorts-style output.", "render.reframe, edit.crop", areas.repurposeTool, opens.repurpose, "Vertical crop", "Requires a visual clip."),
  "cut.header.repurpose.variants": menuChoice("Shorts variants", "Repurpose menu", "Generate multiple short-form variants from one source sequence.", "render.bundle, render.final", areas.repurposeTool, opens.repurpose, "Shorts variants", "Works best with a selected source range."),

  "cut.header.autopilot.plan": menuChoice("Plan edit", "Autopilot menu", "Ask the configured agent to propose an edit plan.", "autopilot.run {policy:\"preview\"}", areas.autopilotTool, opens.autopilot, "Plan edit", "Requires a configured CLI agent."),
  "cut.header.autopilot.apply": menuChoice("Apply safe ops", "Autopilot menu", "Apply reversible operations from an agent plan and leave them in review.", "autopilot.run {policy:\"auto_low_risk\"}", areas.autopilotTool, opens.autopilot, "Apply safe ops", "Requires a reviewed agent plan."),

  "cut.header.recipes.browse": menuChoice("Browse recipes", "Recipes menu", "Open available repeatable edit recipes.", "recipe.list", areas.recipesTool, opens.recipes, "Browse recipes"),
  "cut.header.recipes.edit_for_clarity": menuChoice("Edit for clarity", "Recipes menu", "Preview and run a conservative speech cleanup pass that removes retakes and fillers, then tightens pauses at the chosen intensity without forcing a delivery render.", "recipe.describe {name:\"edit-for-clarity\"}, recipe.run", areas.recipesTool, opens.recipes, "Edit for clarity", "Requires an open project with speech media and local speech-to-text readiness."),
  "cut.header.recipes.describe": menuChoice("Describe recipe", "Recipes menu", "Read what a recipe will do before running it.", "recipe.describe", areas.recipesTool, opens.recipes, "Describe recipe"),
  "cut.header.recipes.run": menuChoice("Run recipe", "Recipes menu", "Run a repeatable edit recipe on the current project.", "recipe.run", areas.recipesTool, opens.recipes, "Run recipe", "Requires a compatible recipe and project state."),
  "cut.header.recipes.phone_cleanup": menuChoice("Phone clip cleanup", "Recipes menu", "Clean a phone or camera clip: transcribe, tighten pauses, remove fillers, clean voice audio, add captions, and render at online loudness.", "recipe.run {name:\"phone-clip-cleanup\"}", areas.recipesTool, opens.recipes, "Phone cleanup", "Requires the asset on the timeline and FFmpeg plus caption tooling."),
  "cut.header.recipes.social_bundle": menuChoice("Social short bundle", "Recipes menu", "Render the current short or timeline window as 9:16, 1:1, and 16:9 social versions.", "recipe.run {name:\"social-short-bundle\"}, render.bundle", areas.recipesTool, opens.recipes, "Social bundle", "Requires a renderable timeline and FFmpeg."),
  "cut.header.recipes.privacy_mask": menuChoice("Blur or mask an area", "Recipes menu", "Add a privacy mask to hide a face, label, password, or screen area on a clip.", "recipe.run {name:\"area-privacy-mask\"}, edit.add_mask", areas.recipesTool, opens.recipes, "Privacy mask", "Requires a base-track video clip id; use the Mask drawer for direct visual adjustment."),
  "cut.header.recipes.captions": menuChoice("Add captions", "Recipes menu", "Transcribe an asset and add timeline captions without changing clip timing.", "recipe.run {name:\"add-captions\"}, media.transcribe, captions.generate", areas.recipesTool, opens.recipes, "Add captions", "Requires an asset with speech and installed speech-to-text tooling."),
  "cut.header.recipes.youtube_export": menuChoice("Export for YouTube", "Recipes menu", "Render the current timeline with YouTube-ready geometry, bitrate, and container settings.", "recipe.run {name:\"youtube-export\"}, export.publish", areas.recipesTool, opens.recipes, "YouTube export", "Requires a renderable timeline and FFmpeg."),
  "cut.header.recipes.tiktok_export": menuChoice("Export for TikTok", "Recipes menu", "Render the current timeline with TikTok-ready vertical geometry, bitrate, and container settings.", "recipe.run {name:\"tiktok-export\"}, export.publish", areas.recipesTool, opens.recipes, "TikTok export", "Requires a renderable timeline and FFmpeg."),

  "cut.header.assemble.prompt": menuChoice("Assemble prompt", "Assemble AI menu", "Describe the rough cut you want the agent to assemble.", "assemble.from_script, assemble.repurpose", areas.assembleTool, opens.assemble, "Prompt", "Requires a configured CLI agent."),
  "cut.header.assemble.sources": menuChoice("Source clips", "Assemble AI menu", "Choose source clips for an assembled draft.", "assemble.broll, media.search", areas.assembleTool, opens.assemble, "Source clips", "Requires imported media."),
  "cut.header.assemble.draft": menuChoice("Build draft", "Assemble AI menu", "Build a rough timeline draft from the selected sources and prompt.", "assemble.repurpose, assemble.shorts", areas.assembleTool, opens.assemble, "Build draft", "Requires source clips and a configured CLI agent."),

  "cut.header.storyboard.generate": menuChoice("Generate storyboard plan", "Storyboard menu", "Generate a storyboard plan before inserting clips or generated assets.", "generate.storyboard", areas.storyboardTool, opens.storyboard, "Generate plan", "Requires a configured CLI agent."),
  "cut.header.storyboard.review": menuChoice("Review shots", "Storyboard menu", "Inspect planned shots before committing them to the timeline.", "generate.preview", areas.storyboardTool, opens.storyboard, "Review shots"),
  "cut.header.storyboard.preview": menuChoice("Preview sequence", "Storyboard menu", "Preview the storyboard sequence before insertion.", "generate.preview", areas.storyboardTool, opens.storyboard, "Preview sequence"),
  "cut.header.storyboard.insert": menuChoice("Insert storyboard", "Storyboard menu", "Insert the storyboard plan into the timeline.", "generate.insert", areas.storyboardTool, opens.storyboard, "Insert storyboard", "Requires a generated storyboard plan."),

  "cut.header.comments.add": menuChoice("Add comment", "Review comments menu", "Add a review note to the current project or selected edit.", "comment.add", areas.commentsTool, opens.comments, "Add comment"),
  "cut.header.comments.apply": menuChoice("Apply note", "Review comments menu", "Apply an actionable comment as a reversible edit operation.", "comment.apply", areas.commentsTool, opens.comments, "Apply note", "Requires an actionable comment."),
  "cut.header.comments.resolve": menuChoice("Resolve comment", "Review comments menu", "Mark a review comment as resolved after the issue is handled.", "comment.resolve", areas.commentsTool, opens.comments, "Resolve", "Requires an existing comment."),

  "cut.top.projects.open": menuChoice("Open project", "Projects menu", "Open an existing .cutproj project directory.", "project.open", areas.projects, opens.projects, "Open project"),
  "cut.top.projects.create": menuChoice("Create project", "Projects menu", "Create a new Cut project.", "project.create", areas.projects, opens.projects, "Create project"),
  "cut.top.library.saved": menuChoice("Saved assets", "Library menu", "Browse saved assets available for reuse.", "library.list", areas.libraryTop, opens.library, "Saved assets"),
  "cut.top.library.add": menuChoice("Add to project", "Library menu", "Add a saved library asset into the current project.", "library.add_to_project", areas.libraryTop, opens.library, "Add to project"),
  "cut.top.find.media": menuChoice("Find media", "Find tab", "Search reusable media providers or a local folder, then import a result into the open project.", "assets.search, assets.fetch", areas.findMediaTab, opens.find, "Find media"),
  "cut.top.find.captions": menuChoice("Find moment", "Find tab", "Search indexed frames inside the project to jump to matching visual moments.", "media.index, media.search", areas.findMomentTab, opens.find, "Find moment", "Requires imported video; index the clip before content search."),
  "cut.top.render.preview": menuChoice("Draft-quality check pass", "Render menu", "Set the quality preset to draft for a fast look at the whole timeline before committing to a delivery render.", "render.preview, render.final", areas.render, opens.render, "Quality preset", "Requires FFmpeg."),
  "cut.top.render.full": menuChoice("Render the timeline", "Render menu", "Render the whole timeline using the quality preset, delivery aspect and loudness target chosen in this panel. The deterministic checks run themselves and leave a receipt.", "render.final, verify.checks", areas.render, opens.render, "Quality preset", "Requires FFmpeg and an output folder."),
  "cut.top.render.range": menuChoice("Render selected range", "Preview monitor", "Mark a range on the timeline ruler, then use Render selection under the Preview monitor to render only that window.", "render.preview", areas.renderSelectionButton, opens.render, "Quality preset", "Requires a marked range on the ruler."),
  "cut.top.export.video": menuChoice("Final video", "Export menu", "Export the project as a final video file.", "render.final", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Requires FFmpeg."),
  "cut.top.export.bundle": menuChoice("Publish presets", "Export menu", "Publish straight to a platform's geometry and bitrate: YouTube 16:9, TikTok/Shorts 9:16, Instagram Reels 9:16, or X 16:9. A multi-platform pack with captions and a thumbnail comes from the Social short bundle recipe instead.", "export.publish, render.bundle", areas.exportMenu, opens.exportMenu, "Publish presets", "Requires a renderable timeline and FFmpeg."),
  "cut.top.export.archive": menuChoice("Keeping a copy of a project", "Export menu", "There is no project-archive export in this release. A project is an ordinary .cutproj folder holding the operation log and cached artifacts, so copying or zipping that folder keeps everything; use Interchange (OTIO, EDL, XML) to hand the edit to another tool.", "export.otio, export.xml, export.edl", areas.exportMenu, opens.exportMenu, "Interchange XML / OTIO / EDL", "Requires an open project."),
  "cut.export.preflight": menuChoice("Preflight warnings", "Export menu", "Cut checks for likely export problems before starting a video render. High-risk issues block export; advisory warnings can continue.", "verify.pregate", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Requires a renderable timeline."),
  "cut.export.preflight.black_tail": menuChoice("Black ending", "Preflight warnings", "The timeline continues after the last video clip, so the export would end on black frames.", "verify.pregate empty_tail", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Trim the tail, shorten audio, or add picture before exporting."),
  "cut.export.preflight.dead_frames": menuChoice("Black or frozen footage", "Preflight warnings", "A clip contains black or frozen frames in the edited range.", "verify.pregate black_or_frozen", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Trim around the dead frames or replace the shot."),
  "cut.export.preflight.pacing": menuChoice("Long holds", "Preflight warnings", "The base story has very few cuts over a long timeline.", "verify.pregate slideshow_risk", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Add cuts, motion, or shorten holds if the static pacing is not intentional."),
  "cut.export.preflight.silent_audio": menuChoice("Silent export", "Preflight warnings", "The timeline appears to export without audible audio.", "verify.pregate silent_output", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Add or unmute audio, or continue if the video is intentionally silent."),
  "cut.export.preflight.tiny_clips": menuChoice("Tiny clips", "Preflight warnings", "One or more clips are shorter than a video frame and may not visibly render.", "verify.pregate tiny_or_zero_clips", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Delete the stray clip or extend it."),
  "cut.export.preflight.borders": menuChoice("Black border", "Preflight warnings", "Source media appears letterboxed or pillarboxed.", "verify.pregate uniform_border", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Crop to visible content if you do not want black bands in the export."),
});

function setActiveFeature(id, selectedNode) {
  const feature = features[id] || features["cut.left.assets"];
  const highlight = document.querySelector("[data-manual-highlight]");
  const detailTitle = document.querySelector("[data-detail-title]");
  const detailDescription = document.querySelector("[data-detail-description]");
  const detailWhere = document.querySelector("[data-detail-where]");
  const detailRequirement = document.querySelector("[data-detail-requirement]");
  const detailApi = document.querySelector("[data-detail-api]");
  const popover = document.querySelector("[data-manual-popover]");

  if (highlight) {
    highlight.style.left = `${feature.highlight.left * 100}%`;
    highlight.style.top = `${feature.highlight.top * 100}%`;
    highlight.style.width = `${feature.highlight.width * 100}%`;
    highlight.style.height = `${feature.highlight.height * 100}%`;
    highlight.dataset.label = feature.highlight.label;
  }

  renderOpenedSurface(popover, feature.open);

  if (detailTitle) detailTitle.textContent = feature.title;
  if (detailDescription) detailDescription.textContent = feature.description;
  if (detailWhere) detailWhere.textContent = feature.where;
  if (detailRequirement) detailRequirement.textContent = feature.requirement;
  if (detailApi) detailApi.textContent = feature.api;

  const nodes = Array.from(document.querySelectorAll("[data-feature-id]"));
  const activeNode = selectedNode || nodes.find((node) => node.dataset.featureId === id);
  nodes.forEach((node) => {
    node.classList.toggle("active", node === activeNode);
  });

  if (window.location.hash !== `#${id}`) {
    history.replaceState(null, "", `#${id}`);
  }
}

function normalizeSearchText(value) {
  return String(value || "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
}

function searchTokens(query) {
  return normalizeSearchText(query).split(/\s+/).filter(Boolean);
}

function fieldTokens(values) {
  return searchTokens(values.filter(Boolean).join(" "));
}

function matchesEveryToken(haystackTokens, queryTokens) {
  return queryTokens.every((queryToken) =>
    haystackTokens.some((hayToken) => hayToken.startsWith(queryToken)),
  );
}

function featureSearchFields(node) {
  const feature = features[node.dataset.featureId];
  const primary = [
    node.textContent,
    feature?.title,
    feature?.where,
    feature?.open?.title,
    feature?.open?.title ? `${feature.open.title} menu` : "",
    feature?.open?.selected,
  ];
  const api = [
    feature?.api,
  ];
  const narrative = [
    feature?.description,
    feature?.requirement,
  ];
  return {
    primary: fieldTokens(primary),
    api: fieldTokens(api),
    narrative: fieldTokens(narrative),
  };
}

function featureMatchesSearch(node, tokens) {
  if (!tokens.length) return true;
  const fields = featureSearchFields(node);
  if (matchesEveryToken(fields.primary, tokens)) return true;
  if (matchesEveryToken(fields.api, tokens)) return true;

  const hasShortToken = tokens.some((token) => token.length < 3);
  if (hasShortToken) return false;

  return tokens.length === 1 && matchesEveryToken(fields.narrative, tokens);
}

function updateSearchGroups(normalized) {
  document.querySelectorAll(".manual-tree-subgroup").forEach((group) => {
    let hasVisibleNode = false;
    let node = group.nextElementSibling;
    while (node && !node.classList.contains("manual-tree-subgroup")) {
      if (node.matches?.("[data-feature-id]") && !node.hidden) {
        hasVisibleNode = true;
        break;
      }
      node = node.nextElementSibling;
    }
    group.hidden = normalized.length > 0 && !hasVisibleNode;
  });

  document.querySelectorAll(".manual-folder").forEach((folder) => {
    const hasVisibleNode = Array.from(folder.querySelectorAll("[data-feature-id]")).some((node) => !node.hidden);
    folder.hidden = normalized.length > 0 && !hasVisibleNode;
    if (normalized.length > 0 && hasVisibleNode) folder.open = true;
  });
}

function filterTree(query) {
  const tokens = searchTokens(query);
  document.querySelectorAll("[data-feature-id]").forEach((node) => {
    node.hidden = tokens.length > 0 && !featureMatchesSearch(node, tokens);
  });
  updateSearchGroups(tokens.join(" "));
}

function initManual() {
  document.querySelectorAll("[data-feature-id]").forEach((node) => {
    node.addEventListener("click", () => setActiveFeature(node.dataset.featureId, node));
  });

  const search = document.querySelector("[data-manual-search]");
  if (search) {
    search.addEventListener("input", (event) => filterTree(event.target.value));
  }

  const queryId = new URLSearchParams(window.location.search).get("feature") || "";
  const initialId = queryId || window.location.hash.slice(1);
  setActiveFeature(features[initialId] ? initialId : "cut.left.assets");
}

document.addEventListener("DOMContentLoaded", initManual);
