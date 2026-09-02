const areas = {
  setup: { left: 0.393, top: 0.009, width: 0.054, height: 0.025, label: "Settings" },
  recordMode: { left: 0.236, top: 0.009, width: 0.043, height: 0.025, label: "Record" },
  projects: { left: 0.288, top: 0.009, width: 0.05, height: 0.025, label: "Projects" },
  libraryTop: { left: 0.343, top: 0.009, width: 0.044, height: 0.025, label: "Library" },
  manualTop: { left: 0.452, top: 0.009, width: 0.048, height: 0.025, label: "Manual" },
  render: { left: 0.897, top: 0.009, width: 0.048, height: 0.025, label: "Render" },
  exportMenu: { left: 0.95, top: 0.009, width: 0.042, height: 0.022, label: "Export" },
  titleTool: { left: 0.672, top: 0.009, width: 0.014, height: 0.025, label: "Title" },
  shapeTool: { left: 0.69, top: 0.009, width: 0.014, height: 0.025, label: "Shape" },
  regionMaskTool: { left: 0.708, top: 0.009, width: 0.014, height: 0.025, label: "Mask / privacy" },
  musicTool: { left: 0.724, top: 0.009, width: 0.014, height: 0.025, label: "Music" },
  mixerTool: { left: 0.74, top: 0.009, width: 0.014, height: 0.025, label: "Mixer" },
  repurposeTool: { left: 0.756, top: 0.009, width: 0.014, height: 0.025, label: "Repurpose" },
  autopilotTool: { left: 0.772, top: 0.009, width: 0.014, height: 0.025, label: "Autopilot" },
  recipesTool: { left: 0.788, top: 0.009, width: 0.014, height: 0.025, label: "Recipes" },
  assembleTool: { left: 0.804, top: 0.009, width: 0.014, height: 0.025, label: "Assemble" },
  storyboardTool: { left: 0.82, top: 0.009, width: 0.014, height: 0.025, label: "Storyboard" },
  commentsTool: { left: 0.836, top: 0.009, width: 0.014, height: 0.025, label: "Comments" },
  gpuToggle: { left: 0.861, top: 0.009, width: 0.033, height: 0.025, label: "GPU" },

  transcriptTab: { left: 0.074, top: 0.041, width: 0.036, height: 0.023, label: "Transcript" },
  assetsTab: { left: 0.035, top: 0.041, width: 0.032, height: 0.023, label: "Assets" },
  generateTab: { left: 0.111, top: 0.041, width: 0.034, height: 0.023, label: "Generate" },
  libraryTab: { left: 0.343, top: 0.009, width: 0.044, height: 0.025, label: "Library" },
  projectsTab: { left: 0.288, top: 0.009, width: 0.05, height: 0.025, label: "Projects" },
  findTab: { left: 0.145, top: 0.041, width: 0.022, height: 0.023, label: "Find tab" },
  assetSearch: { left: 0.005, top: 0.098, width: 0.18, height: 0.019, label: "Filter field" },
  assetFilters: { left: 0.005, top: 0.098, width: 0.39, height: 0.019, label: "Kind filters" },
  assetNeedsAction: { left: 0.36, top: 0.098, width: 0.035, height: 0.019, label: "Needs action" },
  mediaHealth: { left: 0.005, top: 0.152, width: 0.39, height: 0.061, label: "Readiness summary" },
  proxyToggle: { left: 0.266, top: 0.072, width: 0.041, height: 0.018, label: "Proxies" },
  importButton: { left: 0.31, top: 0.071, width: 0.036, height: 0.02, label: "Import" },
  generateButton: { left: 0.35, top: 0.071, width: 0.044, height: 0.02, label: "Generate asset" },
  assetCard: { left: 0.004, top: 0.242, width: 0.392, height: 0.042, label: "Asset card" },
  addAtPlayhead: { left: 0.328, top: 0.253, width: 0.047, height: 0.019, label: "Add at playhead" },

  preview: { left: 0.4, top: 0.064, width: 0.443, height: 0.68, label: "Preview monitor" },
  playbackButtons: { left: 0.618, top: 0.739, width: 0.07, height: 0.021, label: "Playback controls" },
  frameButton: { left: 0.785, top: 0.739, width: 0.024, height: 0.021, label: "Frame" },
  renderSelectionButton: { left: 0.81, top: 0.739, width: 0.033, height: 0.021, label: "Render selection" },

  timelineTracks: { left: 0, top: 0.762, width: 0.843, height: 0.198, label: "Timeline" },
  timelineToolbar: { left: 0.004, top: 0.762, width: 0.839, height: 0.022, label: "Timeline toolbar" },
  trackControls: { left: 0.004, top: 0.812, width: 0.078, height: 0.071, label: "Track controls" },
  trackVisibility: { left: 0.06, top: 0.818, width: 0.011, height: 0.022, label: "Show / hide" },
  trackOrder: { left: 0.004, top: 0.812, width: 0.078, height: 0.032, label: "Video track header" },
  trackLock: { left: 0.07, top: 0.818, width: 0.01, height: 0.022, label: "Lock" },
  trackMuteSolo: { left: 0.004, top: 0.85, width: 0.03, height: 0.03, label: "Mute / solo" },
  trackListen: { left: 0.025, top: 0.85, width: 0.016, height: 0.03, label: "Listen" },
  trackGain: { left: 0.045, top: 0.85, width: 0.03, height: 0.03, label: "Gain" },
  trackPan: { left: 0.004, top: 0.85, width: 0.078, height: 0.03, label: "Audio track header" },
  timecode: { left: 0.004, top: 0.762, width: 0.07, height: 0.022, label: "Timecode" },
  razorButton: { left: 0.113, top: 0.762, width: 0.028, height: 0.022, label: "Razor" },
  trimButton: { left: 0.146, top: 0.762, width: 0.028, height: 0.022, label: "Trim" },
  snapButton: { left: 0.173, top: 0.762, width: 0.03, height: 0.022, label: "Snap" },
  rippleButton: { left: 0.398, top: 0.762, width: 0.039, height: 0.022, label: "Ripple del" },
  liftButton: { left: 0.444, top: 0.762, width: 0.035, height: 0.022, label: "Lift del" },
  speedControls: { left: 0.488, top: 0.762, width: 0.065, height: 0.022, label: "Speed" },
  gradeLayerMatte: { left: 0.566, top: 0.762, width: 0.07, height: 0.022, label: "Grade/Layer/Matte" },
  saveAssetsGif: { left: 0.64, top: 0.762, width: 0.075, height: 0.022, label: "Save/GIF" },
  timelineClip: { left: 0.082, top: 0.817, width: 0.125, height: 0.028, label: "Timeline clip" },
  timelineSurface: { left: 0.23, top: 0.85, width: 0.4, height: 0.08, label: "Timeline empty lane" },
  timelineMarker: { left: 0.09, top: 0.796, width: 0.02, height: 0.018, label: "Timeline marker" },

  propertiesTab: { left: 0.846, top: 0.041, width: 0.043, height: 0.023, label: "Properties" },
  colorTab: { left: 0.894, top: 0.041, width: 0.029, height: 0.023, label: "Color" },
  audioTab: { left: 0.927, top: 0.041, width: 0.029, height: 0.023, label: "Audio inspector tab" },
  chatTab: { left: 0.958, top: 0.041, width: 0.028, height: 0.023, label: "Chat" },
  toolsStrip: { left: 0.843, top: 0.041, width: 0.157, height: 0.91, label: "Tools strip" },
  railPin: { left: 0.977, top: 0.041, width: 0.011, height: 0.023, label: "Pin tools" },
  inspectorHeader: { left: 0.848, top: 0.072, width: 0.14, height: 0.045, label: "Inspector" },
  receiptStatus: { left: 0.806, top: 0.966, width: 0.08, height: 0.02, label: "Receipt status" },

  cutdStatus: { left: 0.005, top: 0.966, width: 0.04, height: 0.02, label: "cutd status" },
  envStatus: { left: 0.05, top: 0.966, width: 0.09, height: 0.02, label: "Environment" },
  versionStatus: { left: 0.956, top: 0.966, width: 0.043, height: 0.02, label: "Version" },
};

const recordAreas = {
  workspace: { surface: "recording", left: 0.015, top: 0.075, width: 0.68, height: 0.86, label: "Recording workspace" },
  studioPreview: { surface: "recording", left: 0.015, top: 0.681, width: 0.485, height: 0.318, label: "Studio preview" },
  scenes: { surface: "recording", left: 0.015, top: 0.681, width: 0.485, height: 0.318, label: "Recording scenes" },
  sceneTimer: { surface: "recording", left: 0.015, top: 0.681, width: 0.485, height: 0.318, label: "Recording timer" },
  cameraControls: { surface: "recording", left: 0.52, top: 0.697, width: 0.155, height: 0.083, label: "Camera availability" },
  background: { surface: "recording", left: 0.52, top: 0.81, width: 0.155, height: 0.04, label: "Background selector" },
  rawStreams: { surface: "recording", left: 0.52, top: 0.878, width: 0.14, height: 0.058, label: "Raw streams" },
  hotkeys: { surface: "recording", left: 0.547, top: 0.362, width: 0.126, height: 0.043, label: "Start recording (F9)" },
  rawMode: { surface: "recording", left: 0.405, top: 0.363, width: 0.072, height: 0.034, label: "Raw capture" },
  autoedit: { surface: "recording", left: 0.349, top: 0.363, width: 0.057, height: 0.034, label: "Auto-edit" },
  studioEvents: { surface: "recording", left: 0.52, top: 0.907, width: 0.052, height: 0.031, label: "Studio events" },
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
  assets: opened("Assets", ["Media Health", "Readiness badges", "Needs action filter", "Proxy imports", "Source Monitor / All uses / Reveal", "Relink missing files"], 0.01, 0.087, 0.28),
  generate: opened("Generate", ["Templates", "Prompt plan", "Storyboard plan", "Reference media", "History / compare", "Insert / replace"], 0.01, 0.087, 0.28),
  color: opened("Color", ["Basic grade", "Looks", "Exposure", "Contrast"], 0.825, 0.087, 0.16),
  audio: opened("Audio", ["Gain", "Waveform", "Linked audio", "Ducking"], 0.825, 0.087, 0.16),
  chat: opened("Chat", ["Project context", "Attach assets", "Prompt library", "Ask agent", "Turn review"], 0.825, 0.087, 0.16),
  selectedTools: opened("Selected-clip tools", ["Properties", "Color", "Audio", "Chat", "Pin tools"], 0.79, 0.087, 0.2),
  receipts: opened("Review tabs", ["Receipts", "QC", "Scopes", "Diff", "Accept / reject"], 0.825, 0.665, 0.16),
  record: opened("Record", ["Studio preview", "Camera capture", "Background", "Raw streams", "Hotkeys", "Start / Stop"], 0.19, 0.047, 0.18),
  contextAsset: opened("Asset menu", ["Open in Source Monitor", "Add at playhead", "Relink source…", "Remove from project…"], 0.018, 0.29, 0.2),
  contextLibrary: opened("Library item menu", ["Add to project", "Insert at playhead", "Add / remove favorite", "Edit tags…", "Move to…", "Relink source…", "Make portable copy…", "Remove from Library…"], 0.06, 0.12, 0.22),
  contextProject: opened("Recent project menu", ["Reopen", "Forget from list", "Delete project…"], 0.16, 0.1, 0.18),
  contextPreview: opened("Preview menu", ["Open base source", "Seek to base clip start", "Add marker here"], 0.53, 0.31, 0.2),
  contextTimelineClip: opened("Clip menu", ["Split here", "Trim (slip / slide / roll)…", "Copy", "Cut", "Paste attributes…", "Match Frame", "Reveal in Project / Library", "Reveal Source File", "Replace with…", "Fit to fill gap…", "Audio…", "Speed & time…", "Remove clip", "Remove, keep gap"], 0.37, 0.72, 0.22),
  contextTimelineSurface: opened("Timeline menu", ["Seek here", "Paste copied clip here", "Add marker here", "Set export in / out", "Add video track", "Add audio track"], 0.42, 0.76, 0.22),
  contextTimelineTrack: opened("Track menu", ["Match Frame", "Lock / unlock track", "Show / hide track", "Mute / solo track", "Remove track…"], 0.08, 0.74, 0.2),
  contextTimelineMarker: opened("Marker menu", ["Rename marker", "Add a marker note", "Set marker color", "Seek to marker", "Delete marker"], 0.28, 0.69, 0.2),
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

  "cut.top.projects": feature("Projects", "Top bar", "Open, switch, create, or return to recent Cut projects. With a project open, Make a copy prepares a portable .cutproj with only the media it uses.", "Project files use .cutproj directories. A portable copy needs an open project and the desktop app to choose its destination folder.", "project.list, project.open, project.create, project.package_plan, project.package_create", areas.projects, opens.projects),
  "cut.record.open": feature("Record workspace", "Top bar", "Switch from Edit into the focused Recording Studio workspace for an exact native source preview, capture-input meters, disposable rehearsal, scenes, live controls, raw streams, and auto-polished recording clips.", "Requires a supported desktop capture backend; FFmpeg is required for recording and output. Preview and meter availability is reported by the current native build.", "ui.open {panel:\"record\"}, screen_record.doctor, screen_record.preview_capability", areas.recordMode, opens.record),
  "cut.top.library": feature("Library", "Top bar", "Open saved media, reusable assets, and library-backed material.", "Requires media already saved to the library.", "library.list, library.add_to_project", areas.libraryTop, opens.library),
  "cut.top.settings": feature("Settings and editing cache", "Top bar", "Configure tools, environment readiness, agents, paths, and editor preferences. Health & Recovery can estimate and rebuild missing Cut-owned proxies and filmstrips, then separately preview aged unreferenced cache before an explicit cancellable cleanup with before/removed/after reconciliation. The Motion card can verify a discovered runtime and connector descriptor read-only; managed lifecycle controls remain unavailable until the verified MOTION-DIST-01 distribution exists.", "Some checks require FFmpeg or CLI tools installed locally. Rebuild and cleanup require an open project; cleanup also requires a complete ownership-ledger preview. Partial, changed, foreign, or symlinked inventories are refused. Motion discovery never installs, repairs, updates, removes, logs in, or executes a connector.", "system.doctor, system.mcp_test, system.motion_status, project.cache_rebuild, project.cache_preview, project.cache_purge, jobs.cancel", areas.setup, opens.settings),
  "cut.top.manual": feature("Manual", "Top bar", "Open Cut's bundled, indexed manual. Selecting an entry reveals its real editor surface without running the highlighted action.", "Available in the desktop editor. The separately published online manual is a read-only reference.", "Manual panel, read-only reveal", areas.manualTop),

  "cut.header.title": feature("Title", "Header tools", "Add title cards, lower thirds, or styled text overlays at the current playhead.", "Requires an open project; title templates are optional.", "title.add, title.templates, title.update", areas.titleTool, opens.title),
  "cut.header.shape": feature("Shape", "Header tools", "Add basic shape overlays such as rectangles, circles, arrows, and callout blocks.", "Requires an open project.", "edit.add_shape, shape.update", areas.shapeTool, opens.shape),
  "cut.header.region_mask": feature("Mask / privacy", "Header tools", "Draw a region for blur, pixelation, black-box privacy, or timed redaction from the playhead.", "Requires a base-track video clip to target.", "edit.add_mask, edit.redact", areas.regionMaskTool, opens.regionMask),
  "cut.header.music": feature("Music bed", "Header tools", "Add or manage a background music layer for the current edit.", "Requires an audio file or available library music.", "audio.add_music, edit.duck", areas.musicTool, opens.music),
  "cut.header.mixer": feature("Audio mixer", "Header tools", "Open audio controls for track gain, pan, mute, solo, cleanup, and mix balance.", "Requires audio tracks for most controls.", "edit.gain, edit.pan, edit.mute, edit.solo, audio.cleanup_voice", areas.mixerTool, opens.mixer),
  "cut.header.repurpose": feature("Repurpose into shorts", "Header tools", "Create short-form variants from highlights, framing, captions, and export presets.", "Works best with indexed media and transcript data.", "clip.candidates, score.clip, render.reframe, render.bundle", areas.repurposeTool, opens.repurpose),
  "cut.header.autopilot": feature("Autopilot", "Header tools", "Let an agent plan safe edits, apply reversible operations, and leave reviewable receipts.", "Requires a configured CLI agent.", "autopilot.run", areas.autopilotTool, opens.autopilot),
  "cut.header.recipes": feature("Recipes", "Header tools", "Browse and run repeatable edit recipes for common production tasks: phone cleanup, social bundle, privacy mask, captions, YouTube export, and TikTok export. Timeline-changing recipes ask you to preview the exact plan before Run is enabled.", "Recipe availability depends on the current project state.", "recipe.list, recipe.describe, recipe.run", areas.recipesTool, opens.recipes),
  "cut.header.assemble": feature("Assemble AI", "Header tools", "Ask an agent to assemble a rough cut from prompts, selected media, or storyboard intent. When a b-roll folder is needed, choose it with the desktop folder dialog; Cut shows only its final folder name rather than an editable local path.", "Requires a configured CLI agent and source media. Folder selection requires the desktop app.", "assemble.repurpose, assemble.shorts, assemble.from_script, assemble.broll", areas.assembleTool, opens.assemble),
  "cut.header.storyboard": feature("Storyboard", "Header tools", "Generate or inspect storyboard plans before inserting a sequence into the timeline.", "Prompt/storyboard generation needs a configured CLI agent.", "generate.storyboard, generate.preview, generate.insert", areas.storyboardTool, opens.storyboard),
  "cut.header.comments": feature("Review comments", "Header tools", "Open review notes, add comments, apply comment-driven changes, or resolve feedback.", "Requires project review context for existing comments.", "comment.add, comment.list, comment.apply, comment.resolve", areas.commentsTool, opens.comments),
  "cut.header.gpu": feature("Faster exports (hardware encoding)", "Header tools", "The Faster ON / Faster OFF chip picks the encoder for renders and exports. On uses available video hardware for speed; off forces repeatable software encoding, which is what you want when two renders must match byte for byte.", "Requires compatible video hardware and driver support for the ON state.", "system.doctor, render.final {hardware}", areas.gpuToggle),

  "cut.top.render": feature("Render", "Top bar", "Render the current project or selected range using the active project settings.", "Requires FFmpeg and a valid output folder.", "render.preview, render.final", areas.render, opens.render),
  "cut.top.export": feature("Export menu", "Top bar", "Choose delivery targets, output presets, and platform-oriented export paths.", "Requires a renderable timeline.", "render.final, export.publish, export.xml, render.queue", areas.exportMenu, opens.exportMenu),

  "cut.left.transcript": feature("Transcript tab", "Left sidebar", "Browse a timeline-linked phrase and chapter list with exact start-end ranges. Clicking a range seeks the matching timeline occurrence; reused media asks you to choose the occurrence, and unavailable media explains why it cannot open.", "Requires speech-to-text tooling for generated transcripts. Offline transcripts remain readable even when their media occurrence cannot be opened.", "media.transcribe, transcript.timeline", areas.transcriptTab, opens.transcript),
  "cut.left.assets": feature("Assets tab", "Left sidebar", "Use Assets to confirm imported media, read per-clip readiness, filter by type or action needed, and add clips to the base timeline at the playhead.", "Requires a project and at least one imported media file.", "media.import, media.check, library.add_to_project", areas.assetsTab, opens.assets),
  "cut.left.asset_filters": feature("Asset filters", "Assets tab", "Filter imported media by kind, unused state, 4K, missing files, recent changes, or clips that need action.", "Requires imported media.", "media.search, media.bin_list, media.check", areas.assetFilters),
  "cut.left.asset_needs_action": feature("Needs action filter", "Assets tab", "Show only media that needs a user decision, such as relinking a missing file or handling a large source-only clip.", "Requires imported media. The filter is view-only and does not change project media.", "media.check, proxy preference", areas.assetNeedsAction, opens.assets),
  "cut.left.media_health": feature("Media Health and recover missing media", "Assets tab", "Shows whether source files are missing, which large clips are using source playback, how many proxies are ready, and how many items need action. When assets are offline, Recover missing media reviews a chosen folder and offers only uniquely exact full-file hash matches before it rechecks accepted selections as one grouped metadata operation. A unique strong metadata resemblance is labeled “Possible replacement — review individually” and sends you to normal one-file Relink; it is never selected for grouped apply.", "Requires imported media. Recover missing media appears only for offline assets; ambiguous, sampled, changed, symlinked, or merely similar files are not selectable. The review hint needs matching basename, kind, byte size, and duration or still dimensions; available format/codec/geometry conflicts refuse it.", "media.check, media.relink, media.relink_preview, media.relink_apply, proxy preference", areas.mediaHealth, opens.assets),
  "cut.left.proxies": feature("Proxy imports", "Assets tab", "Turn proxy generation on for future imports when editing 4K, phone, or camera files that preview roughly from source media.", "Requires FFmpeg and newly imported video files.", "media.import {proxy:true}, media.filmstrip", areas.proxyToggle, opens.assets),
  "cut.left.import": feature("Import media", "Assets tab", "Bring video, audio, and image files into the current project. The first import can become the starting timeline.", "Requires readable local media and FFmpeg for probe/proxy work.", "media.import, media.probe", areas.importButton),
  "cut.left.add_at_playhead": feature("Add at playhead", "Asset card", "Insert an asset at the current playhead on the base story timeline. Linked audio is placed with the video so preview and export stay audible.", "Requires an imported asset and an open project.", "edit.insert", areas.addAtPlayhead),
  "cut.left.generate": feature("Generate tab", "Left sidebar", "Create prompt plans, storyboard material, generated inserts, and reusable generated assets.", "Prompt and storyboard planning need a configured CLI agent.", "generate.list, generate.preview, generate.insert, generate.from_prompt, generate.storyboard", areas.generateTab, opens.generate),
  "cut.left.generated_history": feature("Generated history and compare", "Generate tab", "Review integrity-checked project generation history, compare takes, choose one, then insert it or replace the selected clip without regenerating.", "Requires at least one completed generated image or video in the open project.", "assets.generated_list, edit.insert, edit.replace", areas.generateTab, selectedOpen(opens.generate, "History / compare")),
  "cut.left.generated_references": feature("References and variations", "Generate tab", "Attach up to four registered project image or video assets as generation references and label the requested variation. Choose Codex images, Grok Imagine images/video, or Antigravity (`agy`) images; retry reuses saved provenance and never exposes arbitrary source paths.", "Provider-backed generation requires the selected local generation CLI and may spend provider quota only after confirmation. Antigravity uses its native sandboxed non-interactive contract and is image-only.", "assets.generate, assets.generated_list", areas.generateTab, selectedOpen(opens.generate, "Reference media")),
  "cut.left.motion_edit": feature("Edit in Motion", "Timeline and Inspector", "Open the selected linked Motion package in ShellX Motion without exposing the package or return-request paths. ShellX Motion publishes a new immutable ready revision only after its render is verified.", "Requires a selected Motion-linked clip and an installed or configured ShellX Motion editor.", "motion.link.edit", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.left.motion_refresh": feature("Refresh linked Motion render", "Timeline and Inspector", "Adopt the newest verified Canvas return for the same package, motion identity, and authored source revision, replacing pixels in the existing Cut clip without changing its editorial identity.", "Run Edit in Motion and complete a verified Canvas render first. A mismatch or failed render leaves the last good Cut clip untouched.", "motion.link.refresh, project.undo", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.left.motion_tracking": feature("Track and stabilize linked footage", "Inspector", "Choose manifest-declared footage and a visual target, analyze a point or planar region, apply ordinary Motion transform keyframes, verify the attachment, or detach back to the exact prior keyframes.", "Requires a Motion-linked package with footage and a target visual layer. Refresh remains explicit after applying or detaching.", "motion.link.tracking.inventory, motion.link.tracking.request, motion.link.tracking.apply, motion.link.tracking.verify, motion.link.tracking.detach", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.left.find": feature("Find tab", "Left sidebar", "Keep search available at all times in the sidebar. Use it for reusable media, local folders, source-backed stock, cited project moments, and the cross-sequence index. A local-folder search uses the desktop folder picker and displays only the chosen folder name; there is no typed filesystem-path field.", "Find media searches external or local sources; Find moment needs an open project with imported media. Local-folder selection requires the desktop app.", "assets.providers, assets.search, assets.fetch, media.intelligence_status, media.intelligence_search, ui.open", areas.findTab, opens.find),
  "cut.left.find.media": menuChoice("Find media", "Find tab", "Choose a source from the matching Cut server, search its supported kinds, review license and credit, then import into the open project. Built-in stickers work offline and can be browsed without a query; network sources are contacted when you search or import a result.", "assets.providers, assets.search, assets.fetch", areas.findTab, opens.find, "Find media"),
  "cut.left.find.moment": menuChoice("Find moment", "Find tab", "Search cited spoken words, visuals, scenes, beats, markers, and media metadata; preview the registered source, jump to a real current-sequence use, or select citations to discuss with Agent Chat.", "media.intelligence_status, media.intelligence_rebuild, media.intelligence_search", areas.findTab, opens.find, "Find moment", "Requires imported media. Prepare search derives citations only from analysis already stored in the project; it does not silently run missing analysis."),
  "cut.left.sequence_index": menuChoice("Sequence Index", "Find tab", "Search clips and markers across every project sequence, filter by result kind, sequence, or track, then switch sequences and seek the exact result time.", "project.sequence_index, project.sequence_switch, ui.playhead", areas.findTab, opens.find, "Sequence Index", "Requires an open project; results remain path-light and project-scoped."),

  "cut.preview.monitor": feature("Video preview", "Center editor surface", "Preview the current frame, selected edit, and rendered composition while you work.", "Requires imported media for real playback.", "ui.screenshot, render.preview", areas.preview),
  "cut.preview.ffmpeg_setup": feature("Preview setup notice", "Preview monitor", "When FFmpeg is missing, Preview shows a clear setup notice with an Install action that opens the Video processing card.", "Requires the system doctor to confirm FFmpeg is missing.", "system.doctor, system.fetch_tool, ui.highlight", areas.preview),
  "cut.preview.transport": feature("Play controls", "Preview monitor", "Jump to start or end, shuttle backward or forward, and play or pause the timeline.", "Requires an open timeline.", "ui.playhead (play/pause is a preview transport, not a verb)", areas.playbackButtons),
  "cut.preview.frame": feature("Frame button", "Preview controls", "Capture or save the current preview frame for review or reuse.", "Requires a visible preview frame.", "render.frame, media.import", areas.frameButton),
  "cut.preview.render_selection": feature("Render selection", "Preview controls", "Render only the selected range when a timeline range is active.", "Requires an export range on the ruler.", "render.preview", areas.renderSelectionButton),
  "cut.preview.audio": feature("Audio monitor", "Preview controls", "Toggle and monitor preview audio while checking cuts, sync, and narration.", "Requires audio in the timeline for level movement.", "media.waveform, edit.gain", areas.preview),
  "cut.preview.composed": feature("Composed toggle", "Preview controls", "Switch composed preview on or off when checking generated frames, overlays, or render previews.", "Depends on the active render or preview mode.", "render.preview", areas.preview),
  "cut.preview.compare": feature("Compare edits", "Preview monitor", "Pause and place the exact composed frame before the latest timeline edit beside the current durable frame. Before is on the left and Current is on the right; closing it never changes the timeline.", "Requires a saved timeline edit with an earlier compatible state and visible media at the current playhead. Later project metadata stays part of Current but does not become a deceptive identical Before. If Cut cannot prove that pair, Compare explains why instead of guessing.", "render.compare {at_ms, revision} — read-only historical replay", areas.preview),
  "cut.preview.guides": feature("Guides", "Preview controls", "Cycle visual guides for safe areas, thirds, or both when framing titles and overlays.", "No setup required.", "no verb — a preview-only control", areas.preview),
  "cut.preview.fullscreen": feature("Full-screen preview", "Preview controls", "Expand the monitor when checking focus, titles, caption placement, or visual defects.", "No setup required.", "no verb — a preview-only control", areas.preview),

  "cut.timeline.timecode": feature("Timecode", "Timeline", "Read or jump the current playhead time while trimming and reviewing edits.", "Requires an open timeline.", "ui.playhead", areas.timecode),
  "cut.timeline.razor": feature("Razor", "Timeline toolbar", "Split a clip at the playhead so each side can be moved, trimmed, graded, or deleted independently.", "Requires a clip under the playhead.", "edit.split", areas.razorButton),
  "cut.timeline.trim": feature("Trim", "Timeline toolbar", "Adjust clip in and out points while keeping the edit on the timeline.", "Requires a selected clip or trim handle.", "edit.trim", areas.trimButton),
  "cut.timeline.snap": feature("Snap", "Timeline toolbar", "Toggle magnetic alignment to clip edges, markers, and playhead positions.", "No setup required.", "edit.move, edit.trim", areas.snapButton),
  "cut.timeline.ripple": feature("Ripple delete", "Timeline toolbar", "Delete a selection and close the gap so following clips move left.", "Requires a selected clip or range.", "edit.ripple_delete", areas.rippleButton),
  "cut.timeline.lift": feature("Lift delete", "Timeline toolbar", "Delete a selection while leaving a gap in its original time span.", "Requires a selected clip or range.", "edit.ripple_delete {ripple:false}", areas.liftButton),
  "cut.timeline.speed": feature("Speed controls", "Timeline toolbar", "Retiming controls change clip playback speed while preserving the edit span rules.", "Requires a selected clip.", "edit.speed", areas.speedControls),
  "cut.timeline.sync": feature("Sync by audio", "Timeline toolbar", "Align clips using their audio waveforms when matching camera or recorder sources.", "Requires clips with usable audio.", "edit.multicam_sync", areas.timelineToolbar),
  "cut.timeline.multicam": feature("Auto multicam", "Timeline toolbar", "Build a multicam-style alignment from multiple sources when their audio can be matched.", "Requires multiple compatible clips.", "edit.multicam_switch", areas.timelineToolbar),
  "cut.timeline.beat": feature("Cut to beat", "Timeline toolbar", "Place cuts or timing choices against detected beats in the audio.", "Requires analyzed audio.", "edit.cut_to_beat", areas.timelineToolbar),
  "cut.timeline.cleanup_tools": feature("Cleanup tools", "Timeline toolbar", "Run cleanup and scene-detection actions directly beside editing tools instead of opening a top-bar menu.", "Requires an open project. Scene actions need at least one imported video asset.", "edit.trim_edges, edit.split_at_scenes, edit.mark_scenes", areas.timelineToolbar),
  "cut.timeline.trim_dead_air": feature("Trim dead air", "Timeline toolbar", "Trim silence from the beginning and end of the current timeline while keeping the operation reversible.", "Requires an open timeline with audio analysis available.", "edit.trim_edges", areas.timelineToolbar),
  "cut.timeline.split_scenes": feature("Split scenes", "Timeline toolbar", "Split the first imported video asset at detected scene cuts so each scene can be moved, trimmed, or deleted.", "Requires an imported video asset with scene detection data.", "edit.split_at_scenes", areas.timelineToolbar),
  "cut.timeline.mark_scenes": feature("Mark scenes", "Timeline toolbar", "Add timeline markers at detected scene cuts without changing clip timing.", "Requires an imported video asset with scene detection data.", "edit.mark_scenes", areas.timelineToolbar),
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
  "cut.timeline.voiceover": feature("Timeline voiceover", "Audio track header", "Record into the selected audio track after a visible three-second count-in. Start at the playhead or use the current In-Out range; Stop places one sealed take as one Undoable edit, while Cancel adds nothing.", "Requires an unlocked audio track, an admitted native microphone, and a supported host. Direct monitoring stays off.", "voiceover.start, voiceover.tick, voiceover.stop, voiceover.cancel, voiceover.observe_playhead", areas.trackControls),
  "cut.timeline.base_overlay": feature("Base track and overlays", "Timeline tracks", "Normal Insert and normal drops build the base story timeline and ripple later clips. Extra video tracks are overlays: use Alt-drag, a new overlay lane, or an existing overlay lane when the clip should appear on top.", "Requires an open project; overlay placement requires an overlay lane or Alt-drag.", "edit.insert, edit.add_track", areas.timelineTracks),
  "cut.timeline.save_assets": feature("Save to Assets and GIF", "Timeline toolbar", "Save selected output back into project assets or create a GIF from the current selection.", "Requires a selected range or renderable clip.", "media.import, render.preview", areas.saveAssetsGif),

  "cut.context.menu": feature("Using context menus", "Assets, Library, Projects, Preview, and Timeline", "Right-click the exact item you want to work on. Cut keeps that target through the menu action instead of applying it to a similarly named or merely selected item. Commands that need an open project, compatible media, an unlocked track, or copied clip stay visible but explain why they are unavailable.", "For a focused Library item or folder, timeline clip, marker, or track header, press the Context Menu key or Shift+F10. Once open, use Arrow keys, Home, or End to move between enabled commands; Escape or an outside click closes the menu.", "UI menu; its enabled actions dispatch the corresponding Cut operation", areas.timelineTracks),
  "cut.context.assets": feature("Asset item menu", "Assets card", "Right-click an asset card or use its More actions button. Video, audio, and still-image assets can open in Source Monitor; audio-bearing sources show a seekable waveform with amber In/Out marks and a white current-position line, while All uses remains UI-only navigation over exact per-asset Sequence Index rows. A still shows an image preview with a bounded duration and an unlocked video target or Off; it has no pretend transport, source marks, range insert, or audio destination. Source Monitor can reveal that registered asset in Project Assets or Library, or ask the installed desktop shell to reveal its local source file; browser and unavailable-source cases explain why they refuse. An online asset can be added at the playhead; an offline asset gets Relink source. Remove from project is disabled while the asset still has timeline clips, and does not delete the original source file.", "Requires an imported asset. Waveform seeking requires audio. The exact actions vary by media kind, offline state, and whether Cut is already updating that asset.", "UI menu; exact asset target", areas.assetCard, opens.contextAsset),
  "cut.context.library": feature("Library item and folder menus", "Library", "A Library item menu can add or insert its exact item into the open project, toggle favorite, edit tags, move it to a folder, relink a missing source, make a portable copy, or remove it from the Library. A folder menu is intentionally shorter: rename or delete the folder.", "Add and Insert require an open project and an available source. Focus a Library item or folder and use the Context Menu key or Shift+F10 when you do not want to right-click.", "UI menu; exact Library item or folder target", areas.libraryTab, opens.contextLibrary),
  "cut.context.projects": feature("Recent project menu", "Projects", "Use the menu on a recent-project row to reopen that exact project, forget only its recent-list entry, or delete that project after confirmation. Forget leaves files on disk; deleting leaves original media files untouched.", "The current project cannot be reopened or deleted here. A missing project must be restored or forgotten first.", "UI menu; exact recent-project target", areas.projectsTab, opens.contextProject),
  "cut.context.preview": feature("Preview menu", "Preview monitor", "Right-click the Preview monitor or choose More Preview actions. When there is an unambiguous base clip under the playhead, open that source in Source Monitor or seek to its start; with an open project, add a marker at the exact preview time.", "Open base source and Seek stay disabled over a black frame, gap, or composited view with no unambiguous base video.", "edit.add_marker; exact base clip and preview time", areas.preview, opens.contextPreview),
  "cut.context.timeline_clip": feature("Timeline clip menu", "Timeline clip", "Right-click inside a timeline clip for target-specific edits. Footage exposes Match Frame, which opens Source Monitor on the exact normal/reverse/freeze source frame under the playhead; speed ramps and unavailable sources remain disabled instead of guessed. Its Reveal in Project / Library entries focus the exact registered asset in an existing surface. Reveal Source File is desktop-only and re-resolves that asset in the native shell; browser, missing, offline, and non-file cases tell you why they refuse. Media clips also expose split, trim, copy/cut/paste attributes, source replacement or fitting, transitions and fades, Audio, Speed & time, picture controls, and the two remove choices. Caption clips keep only caption edit, seek, and remove; generated titles and shapes keep Inspector edit, Transform, split, and remove so rendered overlays cannot be treated as ordinary footage.", "Focus a clip and press the Context Menu key or Shift+F10 for the keyboard route. Some entries depend on clip kind, a valid seam or click point, selected clips, an adjacent gap, imported compatible media, or timeline audio.", "UI-only source navigation plus edit operations retain the exact clip target", areas.timelineClip, opens.contextTimelineClip),
  "cut.context.timeline_surface": feature("Timeline empty-lane and gap menu", "Timeline lane or gap", "Right-click an empty lane to seek, paste the copied clip on its compatible unlocked track, add a marker, set the export in or out point, or add a video or audio track. Right-click a visible gap to seek its start, paste at the gap, select the gap as the export range, or fit a compatible copied clip to fill it.", "Empty-lane editing needs an open project. Paste requires a copied compatible clip and an unlocked target track; fit also requires the source clip to remain on the timeline and to meet the shown speed range.", "edit.add_marker, edit.fit_to_fill; exact timeline position", areas.timelineSurface, opens.contextTimelineSurface),
  "cut.context.timeline_track": feature("Timeline track menu", "Timeline track header", "Right-click a video track header to Match Frame the clip under the playhead in Source Monitor; normal, reverse, and freeze map exactly, while ramps and unavailable sources stay disabled. At a crossfade or other overlap, select one clip directly before Match Frame can identify its exact source. Track headers also lock or unlock; video and caption tracks can show or hide; audio tracks can mute or solo. Non-base tracks can be removed after confirmation. A right-clicked clip on a locked track instead offers inspection and unlock, never an edit.", "Focus a track header and press the Context Menu key or Shift+F10 for the keyboard route. Base video and audio tracks cannot be removed.", "UI-only source navigation; edit.track_lock, edit.track_visible, edit.mute, edit.solo; exact track target", areas.trackControls, opens.contextTimelineTrack),
  "cut.context.timeline_marker": feature("Timeline marker menu", "Timeline ruler marker", "Right-click a plain timeline marker to rename it, add a note, choose its color, seek to its time, or delete it. Enter commits a rename; Ctrl/Cmd+Enter saves a note; Escape leaves the menu without saving.", "Focus a plain marker and press the Context Menu key or Shift+F10 for the keyboard route. Non-editable marker classes do not open this menu.", "edit.update_marker, edit.remove_marker, edit.seek_marker; exact marker target", areas.timelineMarker, opens.contextTimelineMarker),

  "cut.inspector.properties": feature("Properties tab", "Inspector", "Show selected clip details, engagement scoring, fades, clip actions, and transform controls.", "Requires a selected clip for clip-specific controls.", "ui.select, edit.fade, edit.transform", areas.propertiesTab),
  "cut.inspector.color": feature("Color tab", "Inspector", "Adjust grade and color controls for the selected visual clip. Browse for a LUT with the native desktop file dialog; Cut shows the selected filename rather than exposing an editable absolute path.", "Requires a selected visual clip. LUT browsing requires the desktop app.", "edit.grade", areas.colorTab, opens.color),
  "cut.inspector.audio": feature("Audio tab", "Inspector and timeline", "Select an audio clip to shape Clip volume over its waveform: Add point at playhead, Ctrl/Cmd-click the curve, or drag a named point. Use Inspector for exact time, level, interpolation, or Clear automation; static Gain resumes unchanged after Clear.", "Requires a selected audio clip on an unlocked track with a current project revision; speed-ramped clips explain why automation is unavailable.", "edit.keyframe {param:\"volume\"}; edit.gain", areas.audioTab, opens.audio),
  "cut.inspector.chat": feature("Chat tab", "Inspector", "Use the connected agent to discuss the current project or selected edit context.", "Requires a configured CLI agent.", "agent.chat", areas.chatTab, opens.chat),
  "cut.inspector.chat_assets": feature("Attach project assets", "Chat tab", "Attach up to eight registered project assets to a turn. Cut validates the IDs, shows them on the request, and never accepts an arbitrary source path through Chat.", "Requires an open project with imported assets and a configured CLI agent.", "agent.chat {attachments:[asset_ids]}", areas.chatTab, selectedOpen(opens.chat, "Attach assets")),
  "cut.inspector.chat_review": feature("Review each agent turn", "Chat tab and Review Diff", "Every editing turn records its plan, baseline, tip, exact diff, and safe-revert verdict. Preview or inspect Diff, then Accept, Revert, or Try again. Concurrent human or agent operations disable whole-turn revert.", "Requires an agent turn that applied at least one operation.", "agent.chat, project.diff, project.revert", areas.chatTab, selectedOpen(opens.chat, "Turn review")),
  "cut.inspector.tools_overlay": feature("Tools overlay", "Right edge", "Open selected-clip tools without permanently narrowing the timeline. The overlay closes with the close button, Escape, or an outside click.", "No setup required.", "ui.open, ui.highlight", areas.toolsStrip, opens.selectedTools),
  "cut.inspector.pin": feature("Pin tools", "Tools header", "Pin the selected-clip tools beside the editor when you want a persistent inspector; unpin to return to the full-width timeline.", "Open the Tools overlay first.", "ui.state", areas.railPin, opens.selectedTools),
  "cut.inspector.fades": feature("Fades", "Inspector Properties", "Set fade-in and fade-out values for the selected clip.", "Requires a selected clip.", "edit.fade", areas.inspectorHeader),
  "cut.inspector.transform": feature("Transform", "Inspector Properties", "Adjust position, scale, crop, and related visual transform settings.", "Requires a selected visual clip.", "edit.transform, edit.effect", areas.inspectorHeader),
  "cut.review.ops": feature("Review Ops", "Review panel", "Read every applied operation so edits remain auditable and reversible.", "Requires project operations in the current session or file.", "project.ops, project.undo, project.redo", areas.receiptStatus),
  "cut.review.receipts": feature("Receipts, QC, Scopes, Diff", "Review panel", "Switch review tabs to inspect receipts, quality checks, video scopes, and project diffs before delivery.", "Some tabs depend on completed review, render, or scope-check jobs.", "project.diff, verify.checks, verify.scopes, ui.open", areas.receiptStatus, opens.receipts),
  "cut.review.scopes": feature("Video scopes", "Review panel", "Run an objective frame check for luma, saturation, white balance, broadcast range, and clipping. Turn on images when you want vectorscope, waveform, or histogram evidence. Agents and tests can open this exact tab with ui.open {panel:\"scopes\"}.", "Requires an open project with a renderable frame and FFmpeg.", "verify.scopes, ui.open {panel:\"scopes\"}", areas.receiptStatus, selectedOpen(opens.receipts, "Scopes")),

  "cut.record.studio": feature("Studio preview", "Focused Record workspace", "Record replaces editor chrome with a capture-focused layout. Choose screen and sound on the left, inspect the dominant composition in the centre, adjust scenes and style on the right, and keep Start/Stop visible. Capture setup provides 24/25/30/50/60 FPS presets with custom 1–240 FPS validation and remembers one selected microphone as an app-local preference. Start native preview explicitly to see real bounded pixels from the exact selected display/window, or a fresh Linux Portal choice; pause, hide, stop, permission, source-loss, recursion, and unavailable states are shown truthfully. Microphone and system-audio meters come from the admitted capture streams without monitoring playback or a second stream. A video-only 3–5 second rehearsal can be played immediately and discarded without creating project media. During a normal take, compact live controls expose the real Stop, Pause/Resume, marker, scene, timer, and meter state.", "Requires the Record workspace and a supported native capture adapter. Linux preview always uses the system picker. Rehearsal is disposable and video-only; use the explicit audio checks and meters for sound. Unsupported preview or meter inputs stay labeled unavailable, and real capture still depends on OS permissions.", "screen_record.preview_capability, screen_record.preview_start, screen_record.preview_status, screen_record.preview_frame, screen_record.preview_pause, screen_record.preview_resume, screen_record.preview_hide, screen_record.preview_stop, screen_record.rehearsal_start, screen_record.rehearsal_discard, screen_record.start, screen_record.stop", recordAreas.studioPreview),
  "cut.record.scenes": feature("Recording scenes", "Record workspace", "Choose a named Screen or Presenter PiP scene before recording, then switch the live composition from the compact scene strip. Cut saves the exact scene revision and shared-clock time before showing the switch as saved.", "Presenter PiP requires an admitted camera. A Screen-first scene catalog can start without opening a camera.", "screen_record.start {scenes}, screen_record.scene_activate", recordAreas.scenes),
  "cut.record.scene_timer": feature("Recording timer", "Record workspace", "Choose one timer for the whole recording: Off, Elapsed, or Countdown. During capture you can pause, resume, reset, restart, or end it; each acknowledged change is durable recording metadata rather than baked pixels.", "Requires a recording started with a saved Scenes catalog.", "screen_record.start {scenes.timer}, screen_record.scene_timer", recordAreas.sceneTimer),
  "cut.record.pause_resume": feature("Pause and resume", "Record workspace", "On a supported macOS build, explicitly enable Pause & resume before Start. During the live recording the same button becomes Pause or Resume, and Cut changes its state only after the native owner durably acknowledges the transition.", "Requires one exact Display at a whole-number frame rate. Scenes, Camera, keystrokes, Window capture, and Quality remain unavailable for that take; other platforms explain that Pause is unavailable.", "screen_record.doctor pause, screen_record.start {pause}, screen_record.pause, screen_record.resume", recordAreas.workspace),
  "cut.record.camera_enable": feature("Camera take", "Record workspace", "Turn Camera on in Auto-edit mode and choose one current device. Cut records it as a separate editable take instead of baking it into the screen source.", "Available on a supported Windows or macOS build after the OS admits the selected camera and a real first frame arrives. Raw mode keeps Camera off.", "screen_record.doctor camera, screen_record.start {camera_id}", recordAreas.cameraControls),
  "cut.record.camera_visible": feature("Show or hide camera", "Record workspace", "The Studio preview shows whether the separate camera take will appear in the polished composition. Visibility changes remain editable, timed Studio metadata.", "Requires Camera to be enabled for this Auto-edit recording.", "screen_record.studio_event {source:\"camera\", kind:\"visibility\"}", recordAreas.cameraControls),
  "cut.record.camera_position": feature("Camera position", "Record workspace", "Choose the corner for the camera overlay before recording; live scene and Studio changes remain replayable rather than baked into the screen pixels.", "Requires Camera to be enabled.", "screen_record.studio_event {source:\"camera\", kind:\"transform\", x, y}", recordAreas.cameraControls),
  "cut.record.camera_size": feature("Camera size", "Record workspace", "Adjust the camera overlay size while keeping the captured camera video as its own editable source.", "Requires Camera to be enabled.", "screen_record.studio_event {source:\"camera\", kind:\"transform\", size}", recordAreas.cameraControls),
  "cut.record.camera_shape": feature("Camera shape", "Record workspace", "Choose Circle or Rounded rectangle for the camera overlay. Shape is composition metadata and does not crop the original camera take destructively.", "Requires Camera to be enabled.", "screen_record.studio_event {source:\"camera\", kind:\"transform\", shape}", recordAreas.cameraControls),
  "cut.record.background": feature("Background", "Record workspace", "Choose the recording background style for polished screen-demo output while raw streams remain untouched.", "Works with auto-edit/polish recording output.", "screen_record.studio_event {source:\"background\", kind:\"style\"}", recordAreas.background),
  "cut.record.raw_streams": feature("Raw streams", "Record workspace", "After a capture, Cut reports the raw screen, optional camera, microphone, system audio, and Studio metadata so you can diagnose or reuse exactly what was finalized. The camera remains a separate editable take. On Windows 10 build 20348 or newer, system audio uses endpoint-independent process loopback; if security software blocks it, screen, camera, and microphone capture can still report their own terminal outcomes.", "Requires a completed capture. A new Windows build may need one audio-capture approval from security software.", "screen_record.stop raw_streams, camera_artifact", recordAreas.rawStreams),
  "cut.record.hotkeys": feature("Recording hotkeys", "Record workspace", "F9 starts or stops recording and F12 drops a marker. Camera visibility and placement shortcuts operate only when this Auto-edit take has an admitted camera.", "Global F9 is available in the desktop app; focused-window fallback works while Cut has focus.", "screen_record.start, screen_record.stop, screen_record.studio_event", recordAreas.hotkeys),
  "cut.record.raw_mode": feature("Raw capture mode", "Record workspace", "Raw capture saves the recording as captured without auto-edit or polish, then lets you add the saved file to the timeline manually.", "Requires FFmpeg and a writable output location.", "screen_record.stop {mux_raw:true}, media.import", recordAreas.rawMode),
  "cut.record.autoedit": feature("Auto-edit recording", "Record workspace", "Auto-edit stops the capture, builds a recorder plan, replays Studio camera/background metadata, polishes the clip, and places it on the timeline. A finalized native camera remains a separate editable take in that plan.", "Requires FFmpeg and a finalized capture. Camera composition requires an integrity-checked camera artifact or an explicit project-local camera file.", "screen_record.stop {autoedit:true}, screen_record.autoedit, screen_record.polish", recordAreas.autoedit),
  "cut.record.studio_event_api": feature("Studio event API", "Debug API", "Agents and the UI append timed Studio events for background style, recording markers, and camera visibility or placement. Camera events replay against the finalized separate camera take when one exists.", "Requires an open project and a valid capture_id returned by screen_record.start.", "screen_record.studio_event", recordAreas.studioEvents),

  "cut.workflow.import": feature("Import and organize media", "Assets tab and timeline", "Import a file, verify it in Assets, then add it at the playhead or drag it into the base timeline.", "Requires readable media and FFmpeg.", "media.import, edit.insert", areas.importButton),
  "cut.workflow.base_overlay": feature("Build the base timeline, then overlays", "Assets tab and timeline", "Drop ordinary clips onto the base timeline first. Use Alt-drag or drop on an overlay lane for B-roll, picture-in-picture, titles, masks, or any video that should composite above the main story.", "Requires imported media; overlays need a video overlay lane.", "edit.insert, edit.add_track, edit.transform", areas.timelineTracks),
  "cut.workflow.split_trim": feature("Split and trim clips", "Timeline", "Use Razor, Trim, Snap, Ripple delete, and Lift delete to shape the sequence.", "Requires clips on the timeline.", "edit.split, edit.trim, edit.ripple_delete {ripple:false}", areas.razorButton),
  "cut.workflow.captions": feature("Captions and transcript", "Transcript tab and Inspector", "Generate or import captions, style them, translate caption or transcript text, then use Inspector Find & Replace to preview exact track/range changes before one Undoable Replace.", "Requires speech-to-text setup for generated transcript content; timing refresh is offered only with exact transcript-word evidence.", "media.transcribe, captions.import, captions.bulk_preview, captions.bulk_apply, transcript.timeline", areas.transcriptTab, opens.transcript),
  "cut.workflow.edit_for_clarity": feature("Edit for Clarity", "Recipes", "Preview and run the conservative clarity pass: transcribe, analyze pauses, remove retakes and fillers, then tighten pauses at the chosen intensity without forcing a delivery render.", "Requires an open project with speech media and local speech-to-text readiness.", "recipe.describe {name:\"edit-for-clarity\"}, recipe.run", areas.recipesTool, selectedOpen(opens.recipes, "Edit for clarity")),
  "cut.workflow.generate": feature("Prompt and storyboard Generate", "Generate tab", "Use prompt and storyboard flows to create planned inserts, generated assets, and template-backed material.", "Requires a configured CLI agent for planning flows.", "generate.from_prompt, generate.storyboard, generate.insert", areas.generateTab, opens.generate),
  "cut.workflow.generated_media": feature("Compare and place generated media", "Generate tab", "Choose registered references, request a labelled variation, compare completed takes, select one, and insert or replace it from verified project history. Codex and Antigravity generate images; Grok Imagine also supports video. A cancelled placement can be retried explicitly.", "Requires a configured generation provider and an open project; a provider call may spend quota only after the second confirmation click.", "assets.generate, assets.generated_list, edit.insert, edit.replace", areas.generateTab, selectedOpen(opens.generate, "History / compare")),
  "cut.workflow.motion_roundtrip": feature("Edit a linked clip in Motion", "Timeline, Inspector, and ShellX Motion", "Select a Motion-linked clip, choose Edit in Motion, make rich source changes in ShellX Motion, render the copy-on-write revision, return to Cut, and refresh the same clip. Cut rechecks package identity, authored revision, receipt identity, and media hash before replacing the linked render.", "Requires ShellX Motion and a current linked package. Refresh is explicit; stale or mismatched handbacks never replace the last good render.", "motion.link.edit, motion.link.refresh, motion.link.relink, project.undo", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.workflow.sequence_index": feature("Search across sequences", "Find tab: Sequence Index", "Search clips and markers project-wide, narrow by kind, sequence, or track, and open a result to switch sequence and seek its time.", "Requires an open project with one or more sequences.", "project.sequence_index, project.sequence_switch, ui.playhead", areas.findTab, selectedOpen(opens.find, "Sequence Index")),
  "cut.workflow.agent_review": feature("Review an agent turn", "Chat tab and Review Diff", "Attach registered project assets, choose or edit a prompt, run the turn, inspect its composed Preview and exact Diff, then Accept, guarded Revert, or Try again.", "Requires a configured CLI agent; whole-turn revert is available only when no concurrent operation crossed the turn boundary.", "agent.chat, project.diff, project.revert", areas.chatTab, selectedOpen(opens.chat, "Turn review")),
  "cut.workflow.recording": feature("Record, polish, export, and retry", "Record workspace and status bar", "Open Record, verify the exact source with native preview, watch admitted input meters, optionally make and discard a short rehearsal, then start capture. Lost source, microphone, camera, or system audio is reported honestly; only finalized normal takes are retained. A qualifying failed default-output export shows one Retry after Cut rechecks durable lineage, project revision, source, EditPlan, capture-audio bytes, and output lease; the queued renderer receives private staged input copies instead of reopening mutable paths.", "Requires desktop capture permission and FFmpeg; camera and system audio are optional. Rehearsal is video-only and never becomes project media. Retry is available only for one engine-declared eligible failed Recording Studio export; cancellation, stale inputs, non-durable records, or consumed attempts are refused.", "screen_record.preview_start, screen_record.rehearsal_start, screen_record.rehearsal_discard, screen_record.start, screen_record.studio_event, screen_record.stop, screen_record.autoedit, screen_record.polish, screen_record.export, jobs.retry, jobs.status", recordAreas.workspace),
  "cut.workflow.review": feature("Review receipts", "Review panel", "Check operation history, receipts, QC, scopes, and diffs before accepting a change or rendering final output.", "Requires project operations or completed checks.", "project.ops, project.diff, verify.checks, verify.scopes", areas.receiptStatus),
  "cut.workflow.export": feature("Render and export", "Top bar", "Render a preview, final file, range, or delivery bundle from the active project. Render Queue destinations use one native Save dialog per output and show path-light filenames instead of editable absolute paths.", "Requires FFmpeg and a valid output folder. Render Queue destination selection requires the desktop app.", "render.preview, render.final, export.publish", areas.render),

  "cut.api.debug": feature("Debug API overview", "cutd local API", "Use the Debug API when you need repeatable automation, integration tests, external inspection, or MCP access to Cut.", "Requires a running cutd server bound to loopback.", "POST /api/verb/{name}, GET /api/verbs, cutd mcp", areas.cutdStatus),
  "cut.api.rest": feature("REST verbs", "cutd local API", "Dispatch Cut actions through the local REST endpoint when scripting, testing, or integrating with another tool.", "Requires a running cutd server bound to loopback.", "POST /api/verb/{name}", areas.cutdStatus),
  "cut.api.state": feature("Project state", "cutd local API", "Read the current project, assets, tracks, clips, markers, and settings.", "Requires an open project for timeline data.", "GET /api/state, project.state", areas.versionStatus),
  "cut.api.events": feature("WebSocket events", "cutd local API", "Subscribe to operation, job, and UI-relevant state changes while Cut is running.", "Requires a running cutd server.", "GET /api/events", areas.envStatus),
  "cut.api.mcp": feature("MCP server", "cutd command line", "Expose Cut verbs to MCP-capable clients through the cutd MCP command.", "Requires cutd installed and a running Cut server for proxy mode.", "cutd mcp", areas.cutdStatus),
  "cut.api.motion_jobs": feature("Observe a Motion render", "Debug API or MCP", "Choose a job_id before starting a blocking Motion-backed render, then query that same id from another request without exposing another project's jobs or Motion runtime paths.", "Requires an open Cut project and a current ShellX Motion CLI. Poll no faster than pollAfterMs and stop when it disappears.", "motion.template_to_cut, motion.script_to_cut, motion.link.refresh, motion.job.get, motion.job.list", areas.cutdStatus),
  "cut.api.catalog": feature("Verb catalog", "cutd local API", "Read the machine-readable verb contract used by tools and docs.", "Requires a running cutd server or local schema file.", "GET /api/verbs, schema/verbs.json", areas.versionStatus),
};

let activeFeatureId = "";

const legacyFeatureAliases = {
  "cut.top.find": "cut.left.find",
  "cut.top.find.media": "cut.left.find.media",
  "cut.top.find.captions": "cut.left.find.moment",
};

function resolveFeatureId(id) {
  return features[id] ? id : legacyFeatureAliases[id] || "cut.left.assets";
}

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
  "cut.top.projects.portable_copy": menuChoice("Take a copy with you", "Projects", "Open Make a copy, choose a destination folder, and preview the exact used-media count, dedupe, cache exclusion, and current collision state before confirming. When the current revision is a grouped bulk relink, Cut carries that durable receipt into the package plan and refuses missing, stale, or mismatched evidence. The copy contains only used media; the current project and original files remain unchanged.", "project.package_plan, project.package_create, jobs.status", areas.projects, opens.projects, "Make a copy…", "Requires an open project and the desktop app for the destination-folder picker. Switching project or revision clears older relink/package results before another action can run."),
  "cut.top.library.saved": menuChoice("Saved assets", "Library menu", "Browse saved assets available for reuse.", "library.list", areas.libraryTop, opens.library, "Saved assets"),
  "cut.top.library.add": menuChoice("Add to project", "Library menu", "Add a saved library asset into the current project.", "library.add_to_project", areas.libraryTop, opens.library, "Add to project"),
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

function updateHighlight(highlight, area) {
  const surface = highlight.closest(".manual-surface");
  const width = surface?.clientWidth || 0;
  const height = surface?.clientHeight || 0;
  // Keep icon-scale targets legible after desktop or mobile layout rounding.
  const minimum = 14;
  const visualWidth = width ? minimum / width : 0;
  const visualHeight = height ? minimum / height : 0;
  const highlightWidth = Math.min(1, Math.max(area.width, visualWidth));
  const highlightHeight = Math.min(1, Math.max(area.height, visualHeight));
  const left = Math.max(0, Math.min(1 - highlightWidth, area.left + area.width / 2 - highlightWidth / 2));
  const top = Math.max(0, Math.min(1 - highlightHeight, area.top + area.height / 2 - highlightHeight / 2));

  highlight.style.left = `${left * 100}%`;
  highlight.style.top = `${top * 100}%`;
  highlight.style.width = `${highlightWidth * 100}%`;
  highlight.style.height = `${highlightHeight * 100}%`;
  highlight.dataset.label = area.label;
}

function setActiveHighlight(feature) {
  const surfaceName = feature.highlight.surface || "editor";
  let activeHighlight = null;
  document.querySelectorAll("[data-manual-highlight]").forEach((highlight) => {
    const active = highlight.dataset.manualHighlight === surfaceName;
    highlight.hidden = !active;
    if (active) {
      updateHighlight(highlight, feature.highlight);
      activeHighlight = highlight;
    }
  });
  return activeHighlight;
}

function setActiveFeature(id, selectedNode, revealTarget = Boolean(selectedNode)) {
  const resolvedId = resolveFeatureId(id);
  const feature = features[resolvedId];
  activeFeatureId = resolvedId;
  const highlight = setActiveHighlight(feature);
  const detailTitle = document.querySelector("[data-detail-title]");
  const detailDescription = document.querySelector("[data-detail-description]");
  const detailWhere = document.querySelector("[data-detail-where]");
  const detailRequirement = document.querySelector("[data-detail-requirement]");
  const detailApi = document.querySelector("[data-detail-api]");
  const popover = document.querySelector("[data-manual-popover]");

  renderOpenedSurface(popover, feature.highlight.surface ? null : feature.open);

  if (detailTitle) detailTitle.textContent = feature.title;
  if (detailDescription) detailDescription.textContent = feature.description;
  if (detailWhere) detailWhere.textContent = feature.where;
  if (detailRequirement) detailRequirement.textContent = feature.requirement;
  if (detailApi) detailApi.textContent = feature.api;

  const nodes = Array.from(document.querySelectorAll("[data-feature-id]"));
  const activeNode = selectedNode || nodes.find((node) => node.dataset.featureId === resolvedId);
  nodes.forEach((node) => {
    node.classList.toggle("active", node === activeNode);
  });

  if (window.location.hash !== `#${id}`) {
    history.replaceState(null, "", `#${id}`);
  }

  if (revealTarget) {
    highlight?.closest(".manual-surface")?.scrollIntoView({ block: "center", inline: "nearest" });
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
  const initialIsResolvable = Boolean(features[initialId] || legacyFeatureAliases[initialId]);
  setActiveFeature(initialIsResolvable ? initialId : "cut.left.assets", null, initialIsResolvable);
}

document.addEventListener("DOMContentLoaded", initManual);

window.addEventListener("resize", () => {
  const feature = features[activeFeatureId];
  if (feature) {
    setActiveHighlight(feature);
    renderOpenedSurface(document.querySelector("[data-manual-popover]"), feature.highlight.surface ? null : feature.open);
  }
});
