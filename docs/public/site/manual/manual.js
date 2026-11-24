const areas = {
  setup: { left: 0.42708, top: 0.01166, width: 0.0625, height: 0.02915, label: "Settings" },
  recordMode: { left: 0.25521, top: 0.01166, width: 0.04167, height: 0.02915, label: "Record" },
  projects: { left: 0.31042, top: 0.01166, width: 0.05417, height: 0.02915, label: "Projects" },
  libraryTop: { left: 0.37083, top: 0.01166, width: 0.04948, height: 0.02915, label: "Library" },
  manualTop: { left: 0.49635, top: 0.01166, width: 0.05208, height: 0.02915, label: "Manual" },
  render: { left: 0.88281, top: 0.01166, width: 0.05469, height: 0.02915, label: "Render" },
  exportMenu: { left: 0.94427, top: 0.01166, width: 0.04688, height: 0.02915, label: "Export" },
  titleTool: { left: 0.6276, top: 0.01166, width: 0.01562, height: 0.02915, label: "Title" },
  shapeTool: { left: 0.64479, top: 0.01166, width: 0.01562, height: 0.02915, label: "Shape" },
  regionMaskTool: { left: 0.66146, top: 0.01166, width: 0.01562, height: 0.02915, label: "Mask / privacy" },
  musicTool: { left: 0.67865, top: 0.01166, width: 0.01562, height: 0.02915, label: "Music" },
  mixerTool: { left: 0.69583, top: 0.01166, width: 0.01562, height: 0.02915, label: "Mixer" },
  repurposeTool: { left: 0.71302, top: 0.01166, width: 0.01562, height: 0.02915, label: "Repurpose" },
  autopilotTool: { left: 0.73021, top: 0.01166, width: 0.01562, height: 0.02915, label: "Autopilot" },
  recipesTool: { left: 0.7474, top: 0.01166, width: 0.01562, height: 0.02915, label: "Recipes" },
  assembleTool: { left: 0.76458, top: 0.01166, width: 0.01562, height: 0.02915, label: "Assemble" },
  storyboardTool: { left: 0.78177, top: 0.01166, width: 0.01562, height: 0.02915, label: "Timeline contact sheet" },
  commentsTool: { left: 0.80208, top: 0.01166, width: 0.01823, height: 0.02915, label: "Comments" },
  gpuToggle: { left: 0.82656, top: 0.01166, width: 0.0526, height: 0.02915, label: "GPU" },

  transcriptTab: { left: 0.07969, top: 0.05345, width: 0.0401, height: 0.02915, label: "Transcript" },
  assetsTab: { left: 0.03906, top: 0.05345, width: 0.03906, height: 0.02915, label: "Assets" },
  generateTab: { left: 0.12083, top: 0.05345, width: 0.0349, height: 0.02915, label: "Generate" },
  libraryTab: { left: 0.37083, top: 0.01166, width: 0.04948, height: 0.02915, label: "Library" },
  projectsTab: { left: 0.31042, top: 0.01166, width: 0.05417, height: 0.02915, label: "Projects" },
  findTab: { left: 0.15729, top: 0.05345, width: 0.01875, height: 0.02915, label: "Find tab" },
  assetSearch: { left: 0.00417, top: 0.13022, width: 0.07396, height: 0.02332, label: "Filter field" },
  assetFilters: { left: 0.00417, top: 0.13022, width: 0.31979, height: 0.02332, label: "Kind filters" },
  assetNeedsAction: { left: 0.28177, top: 0.13022, width: 0.04115, height: 0.02332, label: "Needs action" },
  mediaHealth: { left: 0.00417, top: 0.19145, width: 0.31979, height: 0.15355, label: "Readiness summary" },
  proxyToggle: { left: 0.18125, top: 0.09329, width: 0.04427, height: 0.0243, label: "Proxies" },
  importButton: { left: 0.23021, top: 0.09329, width: 0.04115, height: 0.0243, label: "Import" },
  generateButton: { left: 0.27448, top: 0.09329, width: 0.04792, height: 0.0243, label: "Generate asset" },
  assetCard: { left: 0.00417, top: 0.35569, width: 0.31562, height: 0.07289, label: "Asset card" },
  addAtPlayhead: { left: 0.22396, top: 0.37998, width: 0.05417, height: 0.0243, label: "Add at playhead" },

  preview: { left: 0.32917, top: 0.09232, width: 0.49323, height: 0.55782, label: "Preview monitor" },
  playbackButtons: { left: 0.48333, top: 0.655, width: 0.0776, height: 0.02721, label: "Playback controls" },
  frameButton: { left: 0.58229, top: 0.65695, width: 0.03177, height: 0.02527, label: "Frame" },
  renderSelectionButton: { left: 0.61719, top: 0.65695, width: 0.06563, height: 0.02527, label: "Render selection" },

  timelineTracks: { left: 0.0, top: 0.68902, width: 0.8224, height: 0.27211, label: "Timeline" },
  timelineToolbar: { left: 0.0, top: 0.68902, width: 0.8224, height: 0.03304, label: "Timeline toolbar" },
  trackControls: { left: 0.0, top: 0.74927, width: 0.11198, height: 0.14966, label: "Track controls" },
  trackVisibility: { left: 0.05781, top: 0.76676, width: 0.0099, height: 0.01944, label: "Show / hide" },
  trackOrder: { left: 0.0, top: 0.74927, width: 0.11198, height: 0.05442, label: "Video track header" },
  trackLock: { left: 0.08646, top: 0.76676, width: 0.0099, height: 0.01944, label: "Lock" },
  trackMuteSolo: { left: 0.0125, top: 0.87561, width: 0.02552, height: 0.02138, label: "Mute / solo" },
  trackListen: { left: 0.0401, top: 0.87561, width: 0.01042, height: 0.02138, label: "Listen" },
  trackGain: { left: 0.08646, top: 0.87561, width: 0.01042, height: 0.02138, label: "Gain" },
  trackPan: { left: 0.0, top: 0.86006, width: 0.11198, height: 0.03887, label: "Audio track header" },
  timecode: { left: 0.00417, top: 0.69096, width: 0.07865, height: 0.02624, label: "Timecode" },
  razorButton: { left: 0.0901, top: 0.69096, width: 0.03125, height: 0.02624, label: "Razor" },
  trimButton: { left: 0.1276, top: 0.69096, width: 0.02708, height: 0.02624, label: "Trim" },
  snapButton: { left: 0.15885, top: 0.69096, width: 0.03229, height: 0.02624, label: "Snap" },
  rippleButton: { left: 0.40156, top: 0.69096, width: 0.04479, height: 0.02624, label: "Ripple del" },
  liftButton: { left: 0.44896, top: 0.69096, width: 0.03802, height: 0.02624, label: "Lift del" },
  speedControls: { left: 0.48958, top: 0.69096, width: 0.10625, height: 0.02624, label: "Speed" },
  gradeLayerMatte: { left: 0.59896, top: 0.69096, width: 0.10833, height: 0.02624, label: "Color/Transform/Remove background" },
  saveAssetsGif: { left: 0.59896, top: 0.69096, width: 0.10833, height: 0.02624, label: "Save/GIF" },
  timelineClip: { left: 0.1125, top: 0.75219, width: 0.07865, height: 0.04762, label: "Timeline clip" },
  timelineSurface: { left: 0.1125, top: 0.90087, width: 0.70937, height: 0.05831, label: "Timeline empty lane" },
  timelineMarker: { left: 0.1125, top: 0.72303, width: 0.07865, height: 0.02527, label: "Timeline marker" },

  propertiesTab: { left: 0.82708, top: 0.05345, width: 0.04531, height: 0.02915, label: "Properties" },
  colorTab: { left: 0.87396, top: 0.05345, width: 0.02656, height: 0.02915, label: "Color" },
  audioTab: { left: 0.9026, top: 0.05345, width: 0.02917, height: 0.02915, label: "Audio inspector tab" },
  chatTab: { left: 0.93542, top: 0.05345, width: 0.025, height: 0.02915, label: "Chat" },
  toolsStrip: { left: 0.8224, top: 0.05248, width: 0.1776, height: 0.90865, label: "Tools strip" },
  railPin: { left: 0.96927, top: 0.05442, width: 0.01458, height: 0.02721, label: "Pin tools" },
  inspectorHeader: { left: 0.82969, top: 0.09135, width: 0.15729, height: 0.04568, label: "Inspector" },
  receiptStatus: { left: 0.82656, top: 0.96599, width: 0.04271, height: 0.02624, label: "Receipt status" },

  cutdStatus: { left: 0.00417, top: 0.96599, width: 0.04427, height: 0.02624, label: "cutd status" },
  envStatus: { left: 0.05365, top: 0.96599, width: 0.05833, height: 0.02624, label: "Environment" },
  versionStatus: { left: 0.93437, top: 0.96599, width: 0.06042, height: 0.02624, label: "Version" },
};

const recordAreas = {
  workspace: { surface: "recording", left: 0.0125, top: 0.13994, width: 0.975, height: 0.74052, label: "Recording workspace" },
  studioPreview: { surface: "recording", left: 0.19062, top: 0.18756, width: 0.61875, height: 0.33042, label: "Selected source preview" },
  scenes: { surface: "recording", left: 0.82135, top: 0.31876, width: 0.1599, height: 0.11662, label: "Camera layout" },
  sceneTimer: { surface: "recording", left: 0.82135, top: 0.23129, width: 0.07865, height: 0.03304, label: "Video timer tab" },
  cameraControls: { surface: "recording", left: 0.82135, top: 0.31876, width: 0.1599, height: 0.11662, label: "Camera controls" },
  background: { surface: "recording", left: 0.90312, top: 0.19436, width: 0.07812, height: 0.03304, label: "Background tab" },
  rawStreams: { surface: "recording", left: 0.19062, top: 0.18756, width: 0.61875, height: 0.55685, label: "Recording details after Stop" },
  hotkeys: { surface: "recording", left: 0.0125, top: 0.90379, width: 0.11458, height: 0.04179, label: "Start recording (F9)" },
  rawMode: { surface: "recording", left: 0.09948, top: 0.55199, width: 0.07865, height: 0.02721, label: "Raw mode" },
  autoedit: { surface: "recording", left: 0.01979, top: 0.55199, width: 0.07969, height: 0.02721, label: "Polished mode" },
  studioEvents: { surface: "recording", left: 0.82135, top: 0.19436, width: 0.1599, height: 0.1069, label: "Studio controls" },
};

function opened(title, items, left, top, width = 0.16) {
  return { title, items, left, top, width };
}

const opens = {
  settings: opened("Settings", ["Check setup", "FFmpeg path", "Caption tools", "CLI agents"], 0.392, 0.04, 0.18),
  projects: opened("Projects", ["Open project", "Recent projects", "Create project", "Project folder"], 0.293, 0.04, 0.16),
  library: opened("Library", ["Saved assets", "Reusable bins", "Add to project", "Manage library"], 0.344, 0.04, 0.16),
  find: opened("Find", ["Find media", "Find moment", "Sequence Index", "Search field", "Import result"], 0.138, 0.065, 0.28),
  render: opened("Render", ["Quality preset", "Delivery aspect", "Loudness target", "Subject reframe", "Advanced · timeline format"], 0.9, 0.04, 0.17),
  exportMenu: opened("Export", ["Video (.mp4)", "Audio, GIF, still frame", "Platform exports", "Interchange XML / OTIO / EDL", "Captions and transcript", "Render queue / batch", "Default export folder"], 0.905, 0.04, 0.19),
  title: opened("Title", ["Add title", "Templates", "Lower third", "Edit style"], 0.676, 0.04, 0.16),
  shape: opened("Shape", ["Rectangle", "Circle", "Arrow", "Update shape"], 0.692, 0.04, 0.16),
  regionMask: opened("Mask / privacy", ["Blur face", "Blur rectangle", "Hide plate/text", "From playhead", "Custom shape", "Apply mask"], 0.707, 0.04, 0.18),
  music: opened("Music bed", ["Add music", "Pick source", "Loop to fit", "Duck voice"], 0.722, 0.04, 0.16),
  mixer: opened("Audio mixer", ["Track gain", "Pan", "Mute / solo", "EQ", "Cleanup voice"], 0.738, 0.04, 0.16),
  repurpose: opened("Repurpose", ["Find highlights", "Vertical crop", "Shorts variants", "Export set"], 0.753, 0.04, 0.17),
  autopilot: opened("Autopilot", ["Preview fixes", "Apply fixes", "Run autopilot"], 0.768, 0.04, 0.17),
  recipes: opened("Recipes", ["Browse recipes", "Edit for clarity", "Phone cleanup", "Social bundle", "Privacy mask", "Add captions", "YouTube export", "TikTok export"], 0.783, 0.04, 0.2),
  assemble: opened("Assemble", ["Prompt", "Source clips", "Build draft", "Insert timeline"], 0.799, 0.04, 0.16),
  storyboard: opened("Timeline contact sheet", ["12 timeline frames", "Review the whole edit"], 0.814, 0.04, 0.17),
  comments: opened("Comments", ["Add comment", "Make changes", "Resolve"], 0.835, 0.04, 0.17),
  transcript: opened("Transcript", ["Phrase search", "Clip / Program / Source", "Generate captions", "Transcript tools"], 0.07, 0.065, 0.28),
  assets: opened("Assets", ["Media Health", "Readiness badges", "Needs action filter", "Proxy imports", "Source Monitor / All uses / Reveal", "Relink missing files"], 0.034, 0.065, 0.28),
  generate: opened("Generate", ["Templates", "Prompt plan", "Storyboard plan", "Reference media", "History / compare", "Insert / replace"], 0.105, 0.065, 0.28),
  color: opened("Color", ["Basic grade", "Looks", "Exposure", "Contrast"], 0.886, 0.065, 0.16),
  audio: opened("Audio", ["Gain", "Waveform", "Linked audio", "Ducking"], 0.913, 0.065, 0.16),
  chat: opened("Chat", ["Project context", "Attach assets", "Ask agent", "Applied change", "Step back", "Ask replacement"], 0.941, 0.065, 0.16),
  selectedTools: opened("Selected-clip tools", ["Properties", "Color", "Audio", "Chat", "Pin tools"], 0.85, 0.065, 0.2),
  receipts: opened("Review tabs", ["Edit history", "Render history & checks", "Quality checks", "Scopes", "Compare edits", "Undo / restore"], 0.843, 0.63, 0.16),
  record: opened("Record", ["Studio preview", "Camera capture", "Background", "Raw streams", "Hotkeys", "Start / Stop"], 0.241, 0.04, 0.18),
  contextAsset: opened("Asset menu", ["Open in Source Monitor", "Add at playhead", "Relink source…", "Remove from project…"], 0.19, 0.31, 0.2),
  contextLibrary: opened("Library item menu", ["Add to project", "Insert at playhead", "Add / remove favorite", "Edit tags…", "Move to…", "Relink source…", "Make portable copy…", "Remove from Library…"], 0.06, 0.1, 0.22),
  contextProject: opened("Recent project menu", ["Reopen", "Forget from list", "Delete project…"], 0.1, 0.1, 0.18),
  contextPreview: opened("Preview menu", ["Open base source", "Seek to base clip start", "Add marker here"], 0.43, 0.27, 0.2),
  contextTimelineClip: opened("Clip menu", ["Split here", "Trim (slip / slide / roll)…", "Copy", "Cut", "Paste attributes…", "Match Frame", "Reveal in Project / Library", "Reveal Source File", "Replace with…", "Fit to fill gap…", "Audio…", "Speed & time…", "Remove clip", "Remove, keep gap"], 0.17, 0.8, 0.22),
  contextTimelineSurface: opened("Timeline menu", ["Seek here", "Paste copied clip here", "Add marker here", "Set export in / out", "Add video track", "Add audio track"], 0.3, 0.82, 0.22),
  contextTimelineTrack: opened("Track menu", ["Match Frame", "Lock / unlock track", "Show / hide track", "Mute / solo track", "Remove track…"], 0.08, 0.82, 0.2),
  contextTimelineMarker: opened("Marker menu", ["Rename marker", "Add a marker note", "Set marker color", "Seek to marker", "Delete marker"], 0.13, 0.78, 0.2),
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
    "FFmpeg powers probing, preview media, proxies, screen recording, and final renders. If it is missing, the first start opens the Welcome setup window and Preview shows a setup notice that opens the Video processing card.",
    "On Windows/Linux x86_64 and Apple silicon Macs, choose Download in the Video processing card; the publisher, version, size, licence and source are shown beside it. The Apple silicon build cannot stabilize video: for Stabilize, and on Intel Macs, the card shows the Homebrew steps (install Homebrew, then run brew install ffmpeg-full, with Copy buttons). Check again finds a new install without a restart; you can also choose an ffmpeg that has ffprobe beside it. With several FFmpeg builds installed, Cut uses one that can encode with the graphics hardware; the Video processing card's Why this ffmpeg line says which build is in use and why, and Choose ffmpeg overrides it.",
    "system.doctor, system.set_ffmpeg, system.fetch_tool",
    areas.setup,
    opens.settings,
  ),
  "cut.setup.doctor": feature(
    "Check setup",
    "Settings",
    "Check setup when Cut cannot import, preview, transcribe, record, or render. It separates missing tools from project problems.",
    "Open Settings and choose Re-scan.",
    "system.doctor",
    areas.setup,
    opens.settings,
  ),
  "cut.setup.ffmpeg_path": feature(
    "Set FFmpeg path",
    "Settings",
    "Point Cut at a specific FFmpeg binary when it is installed outside the normal system PATH.",
    "Requires a local ffmpeg executable and an ffprobe Cut can find, normally beside it; otherwise nothing is saved.",
    "system.set_ffmpeg",
    areas.setup,
    opens.settings,
  ),
  "cut.setup.captions": feature(
    "Captions and transcription",
    "Transcript tab and Settings",
    "Captions need speech-to-text tooling in addition to FFmpeg. Install this when transcript actions show a setup message. If you run Cut inside a container, enable container nesting before installing captions. Without it, setup can fail with Permission denied.",
    "Install captions sets up Cut's own captions environment and speech model; no Python of your own is needed, and one you installed is not used. Until then, transcript and caption actions say that captions and transcription are not installed and offer Install captions. Captions on Windows need the Microsoft Visual C++ Redistributable; ShellX Cut tells you when it is missing and gets it from Microsoft for you: Settings > AI & transcription > Microsoft Visual C++ runtime > Install downloads Microsoft's own installer, checks Microsoft's signature, and Windows asks for administrator permission. Uninstalling ShellX Cut keeps it.",
    "system.setup_perception, system.fetch_tool, system.set_stt_model, media.transcribe",
    areas.transcriptTab,
    opens.transcript,
  ),
  "cut.setup.cli_install": feature(
    "Install a CLI agent",
    "Settings",
    "Use your installed Claude Code, Codex, Grok, or Antigravity CLI for AI planning, prompt Generate, storyboard Generate, or review assistance. Keep the selected CLI current; Cut checks the capabilities required by each route rather than pinning a provider version.",
    "Requires the selected CLI tool installed and signed in outside Cut.",
    "system.doctor, system.mcp_test",
    areas.setup,
    opens.settings,
  ),
  "cut.setup.cli_connect": feature(
    "Connect agent in Settings",
    "Settings",
    "After installing and signing in to the CLI outside Cut, connect it in Settings so Generate, chat, and review workflows can call it. Leave the optional Model field empty to use that CLI's configured default; a supplied model name goes only to the selected provider.",
    "Requires a working CLI agent command.",
    "system.doctor, system.mcp_test, agent.chat",
    areas.setup,
    opens.settings,
  ),

  "cut.top.projects": feature("Projects", "Top bar", "Open, switch, create, or return to recent Cut projects. With a project open, Make a copy prepares a portable .cutproj with only the media it uses.", "Project files use .cutproj directories. A portable copy needs an open project and the desktop app to choose its destination folder. If Cut refuses an older project because ops.jsonl exceeds 128 MiB or one operation exceeds 8 MiB, keep the project folder unchanged. Restore a smaller known-good copy, or arrange repair or migration from a full copy; do not truncate ops.jsonl because it contains the editing history.", "project.list, project.open, project.create, project.package_plan, project.package_create", areas.projects, opens.projects),
  "cut.record.open": feature("Record workspace", "Top bar", "Open Record to choose a display or, on Windows and macOS, an application window; confirm readiness, preview actual source pixels, choose Raw MP4 or Polished clip, and start capture. On Linux, Start opens the system picker for a display. On Wayland, screen and sound recording work without input-device access; Show keystrokes, click highlights and click-based automatic zoom need it: run sudo usermod -aG input $USER, then log out and back in to enable them. F9 can also start from Edit using your current Record choices, or the saved setup when Record has not been opened, after fresh source, device, permission, and project checks.", "Requires screen recording support and FFmpeg. Opening Record shows the last readiness check without starting a capture; on macOS the screen-recording prompt can appear only when you check again or start. Record shows whether F9 works outside Cut or only while Cut has focus.", "ui.open {panel:\"record\"}, screen_record.doctor, screen_record.preview_capability", areas.recordMode, opens.record),
  "cut.top.library": feature("Library", "Top bar", "Open saved media, reusable assets, and library-backed material.", "Requires media already saved to the library.", "library.list, library.add_to_project", areas.libraryTop, opens.library),
  "cut.top.settings": feature("Settings and editing cache", "Top bar", "Configure tools, environment readiness, agents, paths, and editor preferences. Settings > About shows the third-party notices: every library in the app with its licence. Health & recovery can estimate and rebuild missing Cut-owned proxies and filmstrips, then separately preview aged unreferenced cache before an explicit cancellable cleanup with before/removed/after reconciliation. The Motion card can find an existing ShellX Motion installation and show its status. Connecting, installing, repairing, or updating Motion from Cut is not ready yet.", "Some checks require FFmpeg or CLI tools installed locally. Rebuild and cleanup require an open project; review the complete cache preview before cleanup. Cut refuses files it cannot safely identify as its own. Motion status only checks an existing installation; it does not change it or sign you in. On Linux, removing the DEB or RPM package keeps your projects, settings and downloaded tools; after quitting Cut, you can optionally erase settings and downloaded tools at ~/.shellx-cut, ~/.config/lv.shellx.cut, ~/.local/share/lv.shellx.cut and ~/.local/share/shellx-cut (use your custom XDG_CONFIG_HOME and XDG_DATA_HOME locations if set). Delete project folders (normally ~/ShellX Cut Projects) only if you also want to erase those projects. On macOS, moving ShellX Cut.app to the Trash keeps your projects, settings and downloaded tools; after quitting Cut, you can optionally erase settings and downloaded tools at ~/.shellx-cut, ~/Library/Application Support/lv.shellx.cut and ~/Library/Application Support/ShellX Cut. Delete project folders (normally ~/ShellX Cut Projects) only if you also want to erase those projects.", "system.doctor, system.mcp_test, system.motion_status, project.cache_rebuild, project.cache_preview, project.cache_purge, jobs.cancel", areas.setup, opens.settings),
  "cut.top.manual": feature("Manual", "Top bar", "Open Cut's bundled, indexed manual. Select an entry to read it, then choose Show in Cut to reveal its editor surface without running the highlighted action. Reference-only entries cannot be revealed.", "Available in the desktop editor. The separately published online manual is a read-only reference.", "Manual panel, read-only reveal", areas.manualTop),

  "cut.header.title": feature("Title", "Header tools", "Add title cards, lower thirds, or styled text overlays at the current playhead.", "Requires an open project; title templates are optional.", "title.add, title.templates, title.update", areas.titleTool, opens.title),
  "cut.header.shape": feature("Shape", "Header tools", "Add basic shape overlays such as rectangles, circles, arrows, and callout blocks.", "Requires an open project.", "edit.add_shape, shape.update", areas.shapeTool, opens.shape),
  "cut.header.region_mask": feature("Mask / privacy", "Header tools", "Draw a region for blur, pixelation, black-box privacy, or timed redaction from the playhead.", "Requires a base-track video clip to target.", "edit.add_mask, edit.redact", areas.regionMaskTool, opens.regionMask),
  "cut.header.music": feature("Music bed", "Header tools", "Add or manage a background music layer for the current edit.", "Requires an audio file or available library music.", "audio.add_music, edit.duck", areas.musicTool, opens.music),
  "cut.header.mixer": feature("Audio mixer", "Header tools", "Open audio controls for track gain, pan, mute, solo, cleanup, and mix balance.", "Requires audio tracks for most controls.", "edit.gain, edit.pan, edit.mute, edit.solo, audio.cleanup_voice", areas.mixerTool, opens.mixer),
  "cut.header.repurpose": feature("Repurpose into shorts", "Header tools", "Create short-form variants from highlights, framing, captions, and export presets.", "Works best with indexed media and transcript data.", "clip.candidates, score.clip, render.reframe, render.bundle", areas.repurposeTool, opens.repurpose),
  "cut.header.autopilot": feature("Autopilot", "Header tools", "Preview a render-quality check, then apply its low-risk fixes and inspect the receipts.", "Requires an open project with a renderable timeline and FFmpeg.", "autopilot.run", areas.autopilotTool, opens.autopilot),
  "cut.header.recipes": feature("Recipes", "Header tools", "Browse and run repeatable edit recipes for common production tasks: phone cleanup, social bundle, privacy mask, captions, YouTube export, and TikTok export. Timeline-changing recipes ask you to preview the exact plan before Run is enabled.", "Recipe availability depends on the current project state.", "recipe.list, recipe.describe, recipe.run", areas.recipesTool, opens.recipes),
  "cut.header.assemble": feature("Assemble (AI)", "Header tools", "Choose Repurpose, Shorts, or From script to review source ranges first, then choose Add reviewed plan to timeline to commit one editable, normal-Undo action. Shorts also applies its reviewed crop and transcript captions when the project aspect matches. B-roll finds and places footage in a slot; when a folder is needed, use the desktop folder dialog, which shows only its final folder name.", "Requires a configured CLI agent and source media. A changed timeline or transcript requires a fresh reviewed plan. Shorts requires the requested project aspect before applying; change the format and plan again if it differs. Folder selection requires the desktop app.", "assemble.repurpose, assemble.shorts, assemble.from_script, assemble.broll", areas.assembleTool, opens.assemble),
  "cut.header.storyboard": feature("Timeline contact sheet", "Header tools", "View 12 evenly spaced frames from the current edit in one image. For shot planning, use Generate > Storyboard.", "Requires a non-empty timeline and video processing tools.", "render.storyboard", areas.storyboardTool, opens.storyboard),
  "cut.header.comments": feature("Comments", "Header tools", "Open timeline notes, add or resolve feedback, or route a note into Agent Chat with its exact target.", "Requires an open project for existing comments.", "comment.add, comment.list, comment.resolve, agent.chat", areas.commentsTool, opens.comments),
  "cut.header.gpu": feature("Faster exports (hardware encoding)", "Header tools", "The Faster ON / Faster OFF chip picks the encoder for renders and exports. On uses available video hardware for speed; off forces repeatable software encoding, which is what you want when two renders must match byte for byte.", "Requires compatible video hardware and driver support for the ON state.", "system.doctor, render.final {hardware}", areas.gpuToggle),

  "cut.top.render": feature("Render", "Top bar", "Render the current project or selected range using the active project settings.", "Requires FFmpeg and a valid output folder.", "render.preview, render.final", areas.render, opens.render),
  "cut.top.export": feature("Export menu", "Top bar", "Choose delivery targets, output presets, and platform-oriented export paths. After a successful interchange export, read the warning list for edits the format could not carry.", "Requires a renderable timeline.", "render.final, export.publish, export.xml, render.queue", areas.exportMenu, opens.exportMenu),

  "cut.left.transcript": feature("Transcript tab", "Left sidebar", "Browse a timeline-linked phrase and chapter list with exact start-end ranges. Clicking a range seeks the matching timeline occurrence; reused media asks you to choose the occurrence, and unavailable media explains why it cannot open.", "Requires speech-to-text tooling for generated transcripts. Without the captions tools the tab offers Install captions; while Cut checks what is installed it says so; with captions installed and no transcript yet it says one is made after a clip is imported. Offline transcripts remain readable even when their media occurrence cannot be opened.", "media.transcribe, transcript.timeline", areas.transcriptTab, opens.transcript),
  "cut.left.assets": feature("Assets tab", "Left sidebar", "Use Assets to confirm imported media, read per-clip readiness, filter by type or action needed, and add clips to the base timeline at the playhead.", "Requires a project and at least one imported media file.", "media.import, media.check, library.add_to_project", areas.assetsTab, opens.assets),
  "cut.left.asset_filters": feature("Asset filters", "Assets tab", "Filter imported media by kind, unused state, 4K, missing files, recent changes, or clips that need action.", "Requires imported media.", "media.search, media.bin_list, media.check", areas.assetFilters),
  "cut.left.asset_needs_action": feature("Needs action filter", "Assets tab", "Show only media that needs a user decision, such as relinking a missing file or handling a large source-only clip.", "Requires imported media. The filter is view-only and does not change project media.", "media.check, proxy preference", areas.assetNeedsAction, opens.assets),
  "cut.left.media_health": feature("Media Health and recover missing media", "Assets tab", "Shows whether source files are missing, which large clips are using source playback, how many proxies are ready, and how many items need action. When assets are offline, Recover missing media reviews a chosen folder and offers only uniquely exact full-file hash matches before it rechecks accepted selections as one grouped metadata operation. A unique strong metadata resemblance is labeled “Possible replacement — review individually” and sends you to normal one-file Relink; it is never selected for grouped apply.", "Requires imported media. Recover missing media appears only for offline assets; ambiguous, sampled, changed, symlinked, or merely similar files are not selectable. The review hint needs matching basename, kind, byte size, and duration or still dimensions; available format/codec/geometry conflicts refuse it.", "media.check, media.relink, media.relink_preview, media.relink_apply, proxy preference", areas.mediaHealth, opens.assets),
  "cut.left.proxies": feature("Proxy imports", "Assets tab", "Turn proxy generation on for future imports when editing 4K, phone, or camera files that preview roughly from source media. Opening a project also checks its proxies and makes the missing ones in the background.", "Requires FFmpeg and newly imported video files.", "media.import {proxy:true}, media.filmstrip", areas.proxyToggle, opens.assets),
  "cut.left.import": feature("Import media", "Assets tab", "Bring video, audio, and image files into the current project. The first import requested on an empty timeline is placed on it; later ones wait in Assets. Only files are imported: dropped folders and unsupported files are named in a notice, and a file Cut cannot read is not kept. Use self-contained local media; playlist files are unsupported.", "Requires readable local media and FFmpeg for probe/proxy work.", "media.import, media.probe", areas.importButton),
  "cut.left.add_at_playhead": feature("Add at playhead", "Asset card", "Insert an asset at the current playhead on the base story timeline. A probed video with audio is placed as one aligned picture-and-sound action, so it cannot leave only one half behind.", "Requires an imported asset and an open project. Video with sound needs available video and audio tracks.", "edit.insert, edit.insert_linked", areas.addAtPlayhead),
  "cut.left.generate": feature("Generate tab", "Left sidebar", "Create prompt plans, storyboard material, generated inserts, and reusable generated assets.", "Prompt and storyboard planning need a configured CLI agent.", "generate.list, generate.preview, generate.insert, generate.from_prompt, generate.storyboard", areas.generateTab, opens.generate),
  "cut.left.generated_history": feature("Generated history and compare", "Generate tab", "Review integrity-checked project generation history, compare takes, choose one, then insert it or replace the selected clip without regenerating.", "Requires at least one completed generated image or video in the open project.", "assets.generated_list, edit.insert, edit.replace", areas.generateTab, selectedOpen(opens.generate, "History / compare")),
  "cut.left.generated_references": feature("References and variations", "Generate tab", "Attach up to four registered project image or video assets as generation references and label the requested variation. Choose Codex images, Grok Imagine images/video, or Antigravity (`agy`) images; retry reuses saved provenance and never exposes arbitrary source paths.", "Provider-backed generation requires the selected local generation CLI and may spend provider quota only after confirmation. Antigravity uses its native sandboxed non-interactive contract and is image-only.", "assets.generate, assets.generated_list", areas.generateTab, selectedOpen(opens.generate, "Reference media")),
  "cut.left.motion_edit": feature("Edit in Motion", "Timeline and Inspector", "This Motion editing connection is still in development. It is designed to open a linked clip in ShellX Motion and return a verified new render to Cut.", "Not ready for normal use. Settings can find an existing Motion installation without changing it.", "motion.link.edit", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.left.motion_refresh": feature("Refresh linked Motion render", "Timeline and Inspector", "This Motion refresh connection is still in development. It is designed to replace the linked clip’s picture only after Cut verifies a new render from the same Motion project.", "Not ready for normal use. A mismatched or failed render must leave the last good Cut clip untouched.", "motion.link.refresh, project.undo", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.left.motion_tracking": feature("Track and stabilize linked footage", "Inspector", "This Motion tracking connection is still in development. It is designed to track a point or region in linked footage, apply the motion to a visual layer, and restore the prior motion when detached.", "Not ready for normal use. The planned workflow needs linked footage and a visual layer; refreshing the Cut clip remains a separate step.", "motion.link.tracking.inventory, motion.link.tracking.request, motion.link.tracking.apply, motion.link.tracking.verify, motion.link.tracking.detach", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.left.find": feature("Find tab", "Left sidebar", "Keep search available at all times in the sidebar. Use it for reusable media, local folders, source-backed stock, cited project moments, and the cross-sequence index. A local-folder search uses the desktop folder picker and displays only the chosen folder name; there is no typed filesystem-path field.", "Find media searches external or local sources; Find moment needs an open project with imported media. Local-folder selection requires the desktop app.", "assets.providers, assets.search, assets.fetch, media.intelligence_status, media.intelligence_search, ui.open", areas.findTab, opens.find),
  "cut.left.find.media": menuChoice("Find media", "Find tab", "Choose a source from the matching Cut server, search its supported kinds, review license and credit, then import into the open project. Built-in stickers work offline and can be browsed without a query; network sources are contacted when you search or import a result.", "assets.providers, assets.search, assets.fetch", areas.findTab, opens.find, "Find media"),
  "cut.left.find.moment": menuChoice("Find moment", "Find tab", "Search cited spoken words, visuals, scenes, beats, markers, and media metadata; preview the registered source, jump to a real current-sequence use, or select citations to discuss with Agent Chat.", "media.intelligence_status, media.intelligence_rebuild, media.intelligence_search", areas.findTab, opens.find, "Find moment", "Requires imported media. Prepare search derives citations only from analysis already stored in the project; it does not silently run missing analysis."),
  "cut.left.sequence_index": menuChoice("Sequence Index", "Find tab", "Search clips and markers across every project sequence, filter by result kind, sequence, or track, then switch sequences and seek the exact result time.", "project.sequence_index, project.sequence_switch, ui.playhead", areas.findTab, opens.find, "Sequence Index", "Requires an open project; results remain path-light and project-scoped."),

  "cut.preview.monitor": feature("Video preview", "Center editor surface", "Preview the current frame, selected edit, and rendered composition while you work.", "Requires imported media for real playback. On Linux, on a heavily loaded machine the picture can hold briefly at a cut while playing; the paused frame and exports are exact.", "ui.screenshot, render.preview", areas.preview),
  "cut.preview.ffmpeg_setup": feature("Preview setup notice", "Preview monitor", "When FFmpeg is missing, Preview shows a clear setup notice with an Install action that opens the Video processing card.", "Requires the system doctor to confirm FFmpeg is missing.", "system.doctor, system.fetch_tool, ui.highlight", areas.preview),
  "cut.preview.transport": feature("Play controls", "Preview monitor", "Jump to start or end, shuttle backward or forward, and play or pause the timeline.", "Requires an open timeline.", "ui.playhead (play/pause is a preview transport, not a verb)", areas.playbackButtons),
  "cut.preview.frame": feature("Frame button", "Preview controls", "Capture or save the current preview frame for review or reuse.", "Requires a visible preview frame.", "export.frame", areas.frameButton),
  "cut.preview.render_selection": feature("Render selection", "Preview controls", "Render only the selected ruler span or clip range to inspect the exact composite with full audio.", "Requires an export range on the ruler or a selected clip.", "export.range, media.import", areas.renderSelectionButton),
  "cut.preview.audio": feature("Audio monitor", "Preview controls", "Toggle and monitor preview audio while checking cuts, sync, and narration.", "Requires audio in the timeline for level movement.", "media.waveform, edit.gain", areas.preview),
  "cut.preview.composed": feature("Composed toggle", "Preview controls", "COMPOSED is on by default: when you pause, the preview shows the finished frame with titles, effects and fades. While playing, a note on the picture names anything that appears only when paused. Turn it off to see the live preview only; the choice is remembered.", "The exact frame appears shortly after the playhead stops; playing, seeking or editing removes it at once.", "ui.preview, ui.state, render.frame", areas.preview),
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
  "cut.timeline.trim_dead_air": feature("Trim silent start and end", "Timeline toolbar", "Trim silence from the beginning and end of the current timeline while keeping the operation reversible.", "Requires an open timeline with audio analysis available.", "edit.trim_edges", areas.timelineToolbar),
  "cut.timeline.split_scenes": feature("Split scenes", "Timeline toolbar", "Split the first imported video asset at detected scene cuts so each scene can be moved, trimmed, or deleted.", "Requires an imported video asset with scene detection data.", "edit.split_at_scenes", areas.timelineToolbar),
  "cut.timeline.mark_scenes": feature("Mark scenes", "Timeline toolbar", "Add timeline markers at detected scene cuts without changing clip timing.", "Requires an imported video asset with scene detection data.", "edit.mark_scenes", areas.timelineToolbar),
  "cut.timeline.grade": feature("Color, Transform, Remove background", "Timeline toolbar", "Open the Color, Transform and Remove background controls for the selected clip.", "Requires a selected visual clip.", "edit.grade, edit.effect", areas.gradeLayerMatte),
  "cut.timeline.ask_agent": feature("Ask agent", "Timeline toolbar", "Open Agent Chat with the selected range, clips, or playhead attached as an immutable target. Write and send the normal Chat request to continue.", "Requires an open project and a configured CLI agent.", "agent.chat {target}", areas.timelineToolbar),
  "cut.timeline.track_controls": feature("Track controls", "Timeline track header", "Use lane headers for show/hide, lock, layer order, mute, solo, listen, gain, and pan without opening the mixer first.", "Requires at least one timeline track; controls vary by track kind.", "edit.track_visible, edit.track_lock, edit.reorder_track, edit.mute, edit.solo, edit.gain, edit.pan, export.audio", areas.trackControls),
  "cut.timeline.track_visibility": feature("Show or hide a track", "Timeline track header", "Hide a video or caption lane from preview and export without deleting its clips. Use mute for audio tracks.", "Requires a video or caption track.", "edit.track_visible", areas.trackVisibility),
  "cut.timeline.track_lock": feature("Lock track edits", "Timeline track header", "Lock a lane when it should stay in place. Drag/drop, trim, move, split targets, delete, and context-menu edits skip locked tracks until you unlock them.", "Works on any timeline track.", "edit.track_lock", areas.trackLock),
  "cut.timeline.track_order": feature("Layer order", "Timeline track header", "Send an overlay video track backward or bring it forward in the visual stack.", "Requires more than one video track.", "edit.reorder_track", areas.trackOrder),
  "cut.timeline.track_mute": feature("Mute track", "Timeline track header", "Silence an audio-bearing track without changing its gain value.", "Requires an audio-bearing track.", "edit.mute", areas.trackMuteSolo),
  "cut.timeline.track_solo": feature("Solo track", "Timeline track header", "Hear only soloed tracks while checking a mix. Explicit mute still wins.", "Requires an audio-bearing track.", "edit.solo", areas.trackMuteSolo),
  "cut.timeline.track_listen": feature("Listen to one track", "Timeline track header", "Render or check the selected audio track by itself when diagnosing a mix.", "Requires an audio track.", "export.audio", areas.trackListen),
  "cut.timeline.track_gain": feature("Track gain", "Timeline track header", "Set a compact per-track gain value directly from the lane header.", "Requires an audio-bearing track.", "edit.gain", areas.trackGain),
  "cut.timeline.track_pan": feature("Track pan", "Timeline track header", "Set common pan positions from the lane header: left, center, right, or half-left/half-right.", "Requires an audio-bearing track.", "edit.pan", areas.trackPan),
  "cut.timeline.voiceover": feature("Timeline voiceover", "Audio track header", "Record into the selected audio track after a visible three-second count-in. Start at the playhead or use the current In-Out range; Stop places one sealed take as one Undoable edit, while Cancel adds nothing.", "Requires an unlocked audio track, a microphone and microphone permission; it never checks screen capture. Windows privacy settings and macOS permission blocks name the setting to change. Windows, macOS and Linux say when no microphone is found; Linux allows Start if its sound server or audio route cannot be checked. Direct monitoring stays off.", "voiceover.start, voiceover.tick, voiceover.stop, voiceover.cancel, voiceover.observe_playhead", areas.trackControls),
  "cut.timeline.base_overlay": feature("Base track and overlays", "Timeline tracks", "Add at playhead inserts on the base timeline. Dragging from Assets overwrites; hold Alt to insert and push the rest right. The dropped lane chooses the track; empty space uses the base track. Video appears on top only when dropped on an overlay lane. Picture and sound are placed in one aligned action; destinations are checked or created together.", "Requires an open project; overlay placement requires an overlay lane. Video with sound needs available video and audio tracks.", "edit.insert, edit.insert_linked, edit.overwrite, edit.add_track", areas.timelineTracks),
  "cut.timeline.save_assets": feature("Save to Assets and GIF", "Timeline toolbar", "Save selected output back into project assets or create a GIF from the current selection.", "Requires a selected range or renderable clip.", "export.range, export.gif", areas.saveAssetsGif),

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
  "cut.inspector.color": feature("Color tab", "Inspector", "Adjust grade and color controls for the selected visual clip. Tag its input as Rec.709, Rec.2020, sRGB, or scene-linear so Cut can convert it into the project's working and output spaces before grade/effects. Browse for a LUT with the native desktop file dialog; Cut shows the selected filename rather than exposing an editable absolute path.", "Requires a selected visual clip. LUT browsing requires the desktop app. The four explicit tags are lightweight color management, not camera Log interpretation or an end-to-end HDR mastering or delivery workflow.", "edit.grade, edit.color_space, project.color", areas.colorTab, opens.color),
  "cut.inspector.audio": feature("Audio tab", "Inspector and timeline", "Select an audio clip to shape Clip volume over its waveform: Add point at playhead, Ctrl/Cmd-click the curve, or drag a named point. Use Inspector for exact time, level, interpolation, or Clear automation; static Gain resumes unchanged after Clear.", "Requires a selected audio clip on an unlocked track with a current project revision; speed-ramped clips explain why automation is unavailable.", "edit.keyframe {param:\"volume\"}; edit.gain", areas.audioTab, opens.audio),
  "cut.inspector.chat": feature("Chat tab", "Inspector", "Use the connected agent to discuss the current project, a selected edit, or an attached timeline target.", "Requires a configured CLI agent and an open project.", "agent.chat", areas.chatTab, opens.chat),
  "cut.inspector.chat_assets": feature("Attach project assets", "Chat tab", "Attach up to eight registered project assets to a turn. Cut validates the IDs, shows them on the request, and never accepts an arbitrary source path through Chat.", "Requires an open project with imported assets and a configured CLI agent.", "agent.chat {attachments:[asset_ids]}", areas.chatTab, selectedOpen(opens.chat, "Attach assets")),
  "cut.inspector.chat_change": feature("Applied Chat change", "Chat tab", "Chat applies a validated result directly. The turn keeps its request, baseline, tip, and diff; use Step back only while the guarded whole-turn revert is safe, or Ask replacement to re-resolve the target before a new request.", "Requires an agent turn that applied at least one operation. Concurrent later changes can make Step back unavailable.", "agent.chat, project.diff, project.revert", areas.chatTab, selectedOpen(opens.chat, "Applied change")),
  "cut.inspector.chat_clear_target": feature("Clear Chat target", "Chat tab", "Remove the attached timeline target before sending so the next request uses the context you choose.", "Requires an attached timeline target and an open project.", "agent.chat", areas.chatTab, selectedOpen(opens.chat, "Clear target")),
  "cut.inspector.tools_overlay": feature("Tools overlay", "Right edge", "Open selected-clip tools without permanently narrowing the timeline. The overlay closes with the close button, Escape, or an outside click.", "No setup required.", "ui.open, ui.highlight", areas.toolsStrip, opens.selectedTools),
  "cut.inspector.pin": feature("Pin tools", "Tools header", "Pin the selected-clip tools beside the editor when you want a persistent inspector; unpin to return to the full-width timeline.", "Open the Tools overlay first.", "ui.state", areas.railPin, opens.selectedTools),
  "cut.inspector.fades": feature("Fades", "Inspector Properties", "Set fade-in and fade-out values for the selected clip.", "Requires a selected clip.", "edit.fade", areas.inspectorHeader),
  "cut.inspector.transform": feature("Transform", "Inspector Properties", "Adjust position, scale, crop, and related visual transform settings. For a new recording, select its Camera track clip and open the Camera section: choose a corner or centre, size, Circle, Rounded or Rectangle, and ring. Move it on the preview, or split at both ends and delete a piece to hide it for a stretch. Back to as recorded restores its recorded look and placement. Undo works for camera edits.", "Requires a selected visual clip.", "edit.transform, edit.effect", areas.inspectorHeader),
  "cut.review.ops": feature("Edit history", "Review panel", "Read every applied operation so edits remain auditable and reversible.", "Requires project operations in the current session or file.", "project.ops, project.undo, project.redo", areas.receiptStatus),
  "cut.review.receipts": feature("Render history & checks, Quality checks, Scopes, Compare edits", "Review panel", "Switch review tabs to inspect render history and checks, quality checks, video scopes, and edit comparisons before delivery.", "Some tabs depend on completed review, render, or scope-check jobs.", "project.diff, verify.checks, verify.scopes, ui.open", areas.receiptStatus, opens.receipts),
  "cut.review.scopes": feature("Video scopes", "Review panel", "Run an objective frame check for luma, saturation, white balance, broadcast range, and clipping. Turn on images when you want vectorscope, waveform, or histogram evidence. Agents and tests can open this exact tab with ui.open {panel:\"scopes\"}.", "Requires an open project with a renderable frame and FFmpeg.", "verify.scopes, ui.open {panel:\"scopes\"}", areas.receiptStatus, selectedOpen(opens.receipts, "Scopes")),

  "cut.record.studio": feature("Record workspace", "Focused Record workspace", "Screen and sound, the real selected-source preview, recording settings, and a persistent Start/Stop transport share the Record workspace. Native preview shows source pixels before Start; during capture the exact active capture supplies live frames, or Record shows an unavailable or stale state while keeping Stop usable. A selected whole display includes Cut when Cut is visible. Refresh sources after a source or device change, and use optional video-only rehearsal to check framing. Recording settings contain Camera, Background, Video timer, Start, length & pause (on Windows and Linux, where a recording cannot be paused, the tab reads Start & length), and Video quality. Advanced frame rate accepts whole-number custom 1–240 FPS only after Apply or Enter; an invalid draft leaves the prior rate active. Cut remembers one selected microphone as an app-local preference. If Cut restarts, refresh the source preview before Start. After a Stop error, Cut checks whether that take is still recording before offering Retry Stop.", "Requires a supported screen source, recording permission, FFmpeg, and a writable export folder. Preview, camera, meters, and Pause depend on your system and devices.", "screen_record.preview_capability, screen_record.preview_start, screen_record.preview_frame, screen_record.live_frame, screen_record.rehearsal_start, screen_record.start, screen_record.status, screen_record.stop", recordAreas.studioPreview),
  "cut.record.scenes": feature("Camera layout", "Record workspace", "Choose Camera on or off, then choose one of four corners, Circle or Rounded rectangle, size, and background. Manual placement survives internal scene changes until Reset camera layout.", "Camera requires an available device and Polished mode; unavailable controls show the reason.", "screen_record.start {scenes}, screen_record.studio_event {source:\"camera\", kind:\"transform\"|\"reset\"}", recordAreas.scenes),
  "cut.record.scene_timer": feature("On-video timer", "Record workspace", "Choose Off, Count up, or Count down. Apply a custom countdown duration from 00:00:01 to 23:59:59; 1, 5, and 10 minutes are shortcuts. During capture, timer Pause, Resume, Reset, Restart, and End timer affect the overlay only. Reaching zero does not stop recording.", "Requires a saved timer configuration and native scene support. Capture Pause and capture length are separate controls.", "screen_record.start {scenes.timer}, screen_record.scene_timer", recordAreas.sceneTimer),
  "cut.record.pause_resume": feature("Pause and resume", "Record workspace", "Pause and resume is available on macOS only. Enable Pause & resume before Start; the setting and live Pause/Resume button are not shown on Windows or Linux. During the live recording the same button becomes Pause or Resume, and Cut changes its state only after the recording confirms the transition.", "Requires one selected display at a whole-number frame rate. Scenes, Camera, keystrokes, Window capture, and Quality remain unavailable for that take. Pause and resume requires macOS; Windows and Linux do not show its controls.", "screen_record.doctor pause, screen_record.start {pause}, screen_record.pause, screen_record.resume", recordAreas.workspace),
  "cut.record.camera_enable": feature("Camera take", "Record workspace", "Turn Camera on in Polished mode when Doctor lists a current device. Choose a corner, Circle or Rounded rectangle, and size. New polished recordings use an editable Camera track by default. Select its clip for the Camera section in Inspector, or turn off Edit the camera later (own track) before recording to bake the bubble in instead.", "Camera is optional and available in Polished mode on Windows, macOS and Linux when a compatible device appears in the camera list. Allow camera access in your operating-system permissions. If the selected camera cannot record, Cut stops the take and reports the problem; choose another available device or turn Camera off before starting again. Raw mode keeps Camera off.", "screen_record.doctor camera, screen_record.start {camera_id}", recordAreas.cameraControls),
  "cut.record.camera_visible": feature("Show or hide camera", "Record workspace", "The Studio preview shows whether the camera will appear in the polished composition. Visibility changes are timed Studio metadata used when that composition is baked.", "Requires Camera to be enabled for this Polished recording.", "screen_record.studio_event {source:\"camera\", kind:\"visibility\"}", recordAreas.cameraControls),
  "cut.record.camera_position": feature("Camera position", "Record workspace", "Choose top left, top right, bottom right, or bottom left. The selected corner stays visible and survives internal scene changes until Reset camera layout.", "Requires an available Camera in Polished mode.", "screen_record.studio_event {source:\"camera\", kind:\"transform\", x, y}; camera reset", recordAreas.cameraControls),
  "cut.record.camera_size": feature("Camera size", "Record workspace", "Choose the camera bubble size for this recording. New polished recordings use a separate camera track by default. After Stop, select the Camera track clip and use the Camera section to change its size, position, shape or ring. Back to as recorded restores the recorded camera layout. Turn off Edit the camera later (own track) on the Record page before recording if you want the bubble baked in.", "Requires Camera to be enabled.", "screen_record.studio_event {source:\"camera\", kind:\"transform\", size}", recordAreas.cameraControls),
  "cut.record.camera_shape": feature("Camera shape", "Record workspace", "Choose Circle or Rounded rectangle for the camera overlay. Shape is composition metadata and does not crop the original camera take destructively.", "Requires Camera to be enabled.", "screen_record.studio_event {source:\"camera\", kind:\"transform\", shape}", recordAreas.cameraControls),
  "cut.record.background": feature("Background", "Record workspace", "Choose the recording background style for polished screen-demo output while raw streams remain untouched.", "Works with Polished recording output.", "screen_record.studio_event {source:\"background\", kind:\"style\"}", recordAreas.background),
  "cut.record.raw_streams": feature("Raw streams", "Record workspace", "After a capture, Cut reports the raw screen, optional camera, microphone, system audio, and Studio metadata so you can diagnose or reuse exactly what was finalized. New polished recordings put the camera on a visible Camera track by default. With Edit the camera later (own track) turned off on the Record page, the bubble is baked in instead. On Windows 10 build 20348 or newer, system audio uses endpoint-independent process loopback; if security software blocks it, screen, camera, and microphone capture can still finish separately.", "Requires a completed capture. A new Windows build may need one audio-capture approval from security software.", "screen_record.stop raw_streams, camera_artifact", recordAreas.rawStreams),
  "cut.record.hotkeys": feature("Recording hotkeys", "Record transport", "F9 starts or stops recording in Cut, including Edit. It also works while another app is focused when your operating system allows the global shortcut. Start uses the open project when one exists; otherwise Cut creates a new recording project before capture. It checks your current Record choices, or the saved setup when Record has not been opened; first use without saved settings offers screen-only capture. Invalid setup reports a reason and starts no capture without switching workspaces. F12 adds a marker. Record shows whether F9 works globally or only while Cut has focus, and explains any limitation.", "Windows and macOS try to enable the global shortcut automatically; GNOME Wayland tries on first launch. Availability depends on your operating system and shortcut permissions. You can turn it off in Settings.", "project.create, screen_record.start, screen_record.status, screen_record.stop, screen_record.studio_event", recordAreas.hotkeys),
  "cut.record.raw_mode": feature("Raw MP4", "Record workspace", "Raw saves the unchanged original MP4 to the default export folder after Stop. Add to timeline or Save a copy are result actions; there is no destination picker before recording.", "Requires FFmpeg and a writable default export folder. If Stop reports an error, Cut checks whether the take is still recording before offering Retry Stop.", "screen_record.stop {mux_raw:true}, screen_record.copy_raw, media.import", recordAreas.rawMode),
  "cut.record.autoedit": feature("Polished clip in Edit", "Record workspace", "Polished saves the same unchanged original MP4 and adds an editable clip to the open project; when no project is open, Cut creates a new recording project first. The polished clip follows recorded pointer movement and clicks with blue click rings and smooth zoom up to 2×. Display recording supports smooth cursor styling; Windows/macOS Window recording keeps the native cursor. On Wayland, stationary clicks may lack a fresh pointer position and produce no ring or zoom. Show keystrokes is optional, off by default, and shown directly beneath this mode. For new recordings, select the screen clip in Edit and use the Zoom card in Inspector to select, add or delete zooms, change strength, target, timing and speed. Redo automatic zooms keeps your own zooms. Press Update video to see the changes; playback keeps the previous zooms until it finishes. Earlier 0.6.116–0.6.118 recordings stay unchanged on open and can be turned into editable takes from the Zoom card (Make editable) when their capture files still exist. New camera recordings use a separate camera track by default; select its clip for the Camera section and Back to as recorded. The Record page switch Edit the camera later (own track) can bake the bubble in instead. Export MP4 or GIF chooses a file after Stop.", "Requires FFmpeg and a finalized capture. A camera bubble needs a completed camera take.", "project.create, screen_record.stop {mux_raw:true, autoedit:true}, screen_record.polish, screen_record.export", recordAreas.autoedit),
  "cut.record.studio_event_api": feature("Studio event API", "Debug API", "Agents and the UI append timed Studio events for background style, recording markers, and camera visibility or placement. Camera events replay against the finalized separate camera take when one exists.", "Requires a valid project-bound capture_id returned by screen_record.start.", "screen_record.studio_event", recordAreas.studioEvents),

  "cut.workflow.import": feature("Import and organize media", "Assets tab and timeline", "Import a file, verify it in Assets, then add it at the playhead or drag it into the base timeline. Muxed video enters as one aligned picture-and-sound placement.", "Requires readable media and FFmpeg.", "media.import, edit.insert, edit.insert_linked", areas.importButton),
  "cut.workflow.base_overlay": feature("Build the base timeline, then overlays", "Assets tab and timeline", "Drop ordinary clips onto the base timeline first. Plain drops overwrite; Alt inserts. A muxed video validates its picture and sound destinations together. Drop on an overlay lane for B-roll, picture-in-picture, titles, masks, or any video that should composite above the main story.", "Requires imported media; overlays need a video overlay lane and muxed video needs an audio destination.", "edit.insert, edit.insert_linked, edit.overwrite, edit.add_track, edit.transform", areas.timelineTracks),
  "cut.workflow.split_trim": feature("Split, trim, and move clips", "Timeline", "Use Razor, Trim, Snap, Ripple delete, and Lift delete to shape the sequence. Drag a clip to move it: a plain drag is Overwrite (it replaces what is under it; nothing else moves); hold Alt for Insert (it pushes the rest right). The label by the pointer names the mode while you drag.", "Requires clips on the timeline.", "edit.split, edit.trim, edit.move, edit.ripple_delete {ripple:false}", areas.razorButton),
  "cut.workflow.captions": feature("Captions and transcript", "Transcript tab and Inspector", "Generate or import captions, style them, translate caption or transcript text, then use Inspector Find & Replace to preview exact track/range changes before one Undoable Replace.", "Requires speech-to-text setup for generated transcript content; timing refresh is offered only with exact transcript-word evidence.", "media.transcribe, captions.import, captions.bulk_preview, captions.bulk_apply, transcript.timeline", areas.transcriptTab, opens.transcript),
  "cut.workflow.edit_for_clarity": feature("Edit for Clarity", "Recipes", "Preview and run the conservative clarity pass: transcribe, analyze pauses, remove retakes and fillers, then tighten pauses at the chosen intensity without forcing a delivery render.", "Requires an open project with speech media and local speech-to-text readiness.", "recipe.describe {name:\"edit-for-clarity\"}, recipe.run", areas.recipesTool, selectedOpen(opens.recipes, "Edit for clarity")),
  "cut.workflow.generate": feature("Prompt and storyboard Generate", "Generate tab", "Use prompt and storyboard flows to create planned inserts, generated assets, and template-backed material.", "Requires a configured CLI agent for planning flows.", "generate.from_prompt, generate.storyboard, generate.insert", areas.generateTab, opens.generate),
  "cut.workflow.generated_media": feature("Compare and place generated media", "Generate tab", "Choose registered references, request a labelled variation, compare completed takes, select one, and insert or replace it from verified project history. Codex and Antigravity generate images; Grok Imagine also supports video. A cancelled placement can be retried explicitly.", "Requires a configured generation provider and an open project; a provider call may spend quota only after the second confirmation click.", "assets.generate, assets.generated_list, edit.insert, edit.replace", areas.generateTab, selectedOpen(opens.generate, "History / compare")),
  "cut.workflow.motion_roundtrip": feature("Edit a linked clip in Motion", "Timeline, Inspector, and ShellX Motion", "This Motion editing round trip is still in development. The planned workflow opens a linked clip in ShellX Motion, renders the changes, then explicitly refreshes that clip in Cut after checking the returned media.", "Not ready for normal use. Settings can find an existing Motion installation, but Cut cannot manage it or complete the editing round trip yet. A stale or mismatched return must leave the last good clip untouched.", "motion.link.edit, motion.link.refresh, motion.link.relink, project.undo", areas.generateTab, selectedOpen(opens.generate, "Templates")),
  "cut.workflow.sequence_index": feature("Search across sequences", "Find tab: Sequence Index", "Search clips and markers project-wide, narrow by kind, sequence, or track, and open a result to switch sequence and seek its time.", "Requires an open project with one or more sequences.", "project.sequence_index, project.sequence_switch, ui.playhead", areas.findTab, selectedOpen(opens.find, "Sequence Index")),
  "cut.workflow.agent_change": feature("Applied agent change", "Chat tab", "Attach registered project assets or a timeline target, choose or edit a prompt, and run the turn. A validated result applies directly; inspect Preview and Compare edits, use Step back only while it is safe, or Ask replacement to re-resolve the retained target.", "Requires a configured CLI agent and an open project. Concurrent later operations can make Step back unavailable.", "agent.chat, project.diff, project.revert", areas.chatTab, selectedOpen(opens.chat, "Applied change")),
  "cut.workflow.recording": feature("Record, save, and export", "Record workspace and status bar", "In Record, choose a real source, check readiness and native preview, select Raw MP4 or Polished clip, then Start. Both modes save the unchanged MP4 in the default export folder; Polished adds an editable project clip with cursor, blue click rings, and smooth zoom up to 2× when valid pointer and click positions are available. If no project is open, Start or global F9 creates a new recording project automatically; an open project is used as-is. During capture, the preview shows frames from that take. On Windows and macOS, the picture keeps moving between saved parts. On Windows, Stop responds at once while the file finishes saving in the background; wait for the saved-file confirmation. Known Windows recording limits: about half a second at the start of a take can repeat the first picture. Short, separate system sounds can drift out of sync with the picture. If Stop fails, Cut offers Retry Stop only when the take is still recording; if it cannot tell, it says so. Closing or quitting Cut during a recording saves it first; a recording interrupted by a crash or power loss is offered at the next start with Add to timeline. Export or Save a copy chooses optional destinations after Stop.", "Requires desktop capture permission and FFmpeg; camera and system audio are optional. F9 from another app needs operating-system support; otherwise it works while Cut has focus. Camera and audio options depend on current devices and permissions.", "project.create, screen_record.preview_start, screen_record.live_frame, screen_record.start, screen_record.status, screen_record.stop, screen_record.polish, screen_record.export, screen_record.copy_raw", recordAreas.workspace),
  "cut.workflow.review": feature("Review render history & checks", "Review panel", "Check edit history, render history and checks, quality checks, scopes, and edit comparisons to understand the project state before rendering final output.", "Requires project operations or completed checks.", "project.ops, project.diff, verify.checks, verify.scopes", areas.receiptStatus),
  "cut.workflow.export": feature("Render and export", "Top bar", "Render a preview, final file, range, or delivery bundle from the active project. Render Queue destinations use one native Save dialog per output and show path-light filenames instead of editable absolute paths. Closing or quitting while an export runs asks first (Keep working or Quit and stop it); after a crash the next start says which export was interrupted and that its unfinished files were removed.", "Requires FFmpeg and a valid output folder. Render Queue destination selection requires the desktop app.", "render.preview, render.final, export.publish", areas.render),

  "cut.api.debug": feature("Debug API overview", "cutd local API", "Use the Debug API when you need repeatable automation, integration tests, external inspection, or MCP access to Cut.", "Requires a running cutd server bound to loopback.", "POST /api/verb/{name}, GET /api/verbs, cutd mcp", areas.cutdStatus),
  "cut.api.rest": feature("REST verbs", "cutd local API", "Dispatch Cut actions through the local REST endpoint when scripting, testing, or integrating with another tool.", "Requires a running cutd server bound to loopback.", "POST /api/verb/{name}", areas.cutdStatus),
  "cut.api.state": feature("Project state", "cutd local API", "Read the current project, assets, tracks, clips, markers, and settings.", "Requires an open project for timeline data.", "GET /api/state, project.state", areas.versionStatus),
  "cut.api.events": feature("WebSocket events", "cutd local API", "Subscribe to operation, job, and UI-relevant state changes while Cut is running.", "Requires a running cutd server.", "GET /api/events", areas.envStatus),
  "cut.api.mcp": feature("MCP server", "cutd command line", "Expose Cut verbs to MCP-capable clients through the cutd MCP command.", "Requires cutd installed and a running Cut server for proxy mode.", "cutd mcp", areas.cutdStatus),
  "cut.api.motion_jobs": feature("Observe a Motion render", "Debug API or MCP", "Choose a job_id before starting a blocking Motion-backed render, then query that same id from another request without exposing another project's jobs or Motion runtime paths.", "Motion rendering through this connection is still in development for normal use. For API experiments, use an open Cut project and a current ShellX Motion CLI; poll no faster than pollAfterMs and stop when the job disappears.", "motion.template_to_cut, motion.script_to_cut, motion.link.refresh, motion.job.get, motion.job.list", areas.cutdStatus),
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

  "cut.header.autopilot.plan": menuChoice("Preview fixes", "Autopilot", "Choose Preview fixes to render and check the timeline, then inspect the proposed fixes without applying them.", "autopilot.run {policy:\"preview\"}", areas.autopilotTool, opens.autopilot, "Preview fixes", "Requires an open project with a renderable timeline and FFmpeg."),
  "cut.header.autopilot.apply": menuChoice("Apply fixes", "Autopilot", "Run the quality check again and apply its low-risk fixes, keeping receipts and a restore point.", "autopilot.run {policy:\"auto_low_risk\"}", areas.autopilotTool, opens.autopilot, "Apply fixes", "Available after Preview finds an automatic fix; requires a renderable timeline and FFmpeg."),

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

  "cut.header.assemble.prompt": menuChoice("Assemble prompt", "Assemble (AI) menu", "Choose Repurpose to find highlights, or From script to match script lines to footage. Review the proposed source ranges before adding them to the timeline.", "assemble.from_script, assemble.repurpose", areas.assembleTool, opens.assemble, "Prompt", "Requires a configured CLI agent."),
  "cut.header.assemble.sources": menuChoice("Source clips", "Assemble (AI) menu", "Choose imported source footage for the reviewed plan, or find and place footage in a B-roll slot. Local B-roll folders use the desktop folder picker.", "assemble.broll, media.search", areas.assembleTool, opens.assemble, "Source clips", "Requires imported media."),
  "cut.header.assemble.draft": menuChoice("Build draft", "Assemble (AI) menu", "Review the Repurpose or Shorts plan, then choose Add reviewed plan to timeline to add its ranges as one editable Undo action. Shorts applies its crop and transcript captions only when the project already has the requested aspect.", "assemble.repurpose, assemble.shorts", areas.assembleTool, opens.assemble, "Build draft", "Requires source clips, a configured CLI agent, and a current reviewed plan. Changed timeline or transcript evidence requires a new review."),

  "cut.header.storyboard.generate": menuChoice("Generate storyboard plan", "Generate > Storyboard", "Generate a storyboard plan before inserting clips or generated assets.", "generate.storyboard", areas.storyboardTool, opens.storyboard, "Generate plan", "Requires a configured CLI agent."),
  "cut.header.storyboard.review": menuChoice("Review shots", "Generate > Storyboard", "Inspect planned shots before committing them to the timeline.", "generate.preview", areas.storyboardTool, opens.storyboard, "Review shots"),
  "cut.header.storyboard.preview": menuChoice("Preview sequence", "Generate > Storyboard", "Preview the storyboard sequence before insertion.", "generate.preview", areas.storyboardTool, opens.storyboard, "Preview sequence"),
  "cut.header.storyboard.insert": menuChoice("Insert storyboard", "Generate > Storyboard", "Insert the storyboard plan into the timeline.", "generate.insert", areas.storyboardTool, opens.storyboard, "Insert storyboard", "Requires a generated storyboard plan."),

  "cut.header.comments.add": menuChoice("Add comment", "Review comments menu", "Add a review note to the current project or selected edit.", "comment.add", areas.commentsTool, opens.comments, "Add comment"),
  "cut.header.comments.make_changes": menuChoice("Make changes", "Comments menu", "Route this comment and its exact timeline target to Agent Chat. The target is checked again before Chat runs.", "agent.chat {target}", areas.commentsTool, opens.comments, "Make changes", "Requires an existing comment, an open project, and a configured CLI agent."),
  "cut.header.comments.resolve": menuChoice("Resolve comment", "Review comments menu", "Mark a review comment as resolved after the issue is handled.", "comment.resolve", areas.commentsTool, opens.comments, "Resolve", "Requires an existing comment."),

  "cut.top.projects.open": menuChoice("Open project", "Projects menu", "Open an existing .cutproj project directory.", "project.open", areas.projects, opens.projects, "Open project"),
  "cut.top.projects.create": menuChoice("Create project", "Projects menu", "Create a new Cut project.", "project.create", areas.projects, opens.projects, "Create project"),
  "cut.top.projects.portable_copy": menuChoice("Take a copy with you", "Projects", "Open Make a copy, choose a destination folder, and preview the exact used-media count, dedupe, cache exclusion, and current collision state before confirming. When the current revision is a grouped bulk relink, Cut carries that durable receipt into the package plan and refuses missing, stale, or mismatched evidence. The copy contains only used media; the current project and original files remain unchanged.", "project.package_plan, project.package_create, jobs.status", areas.projects, opens.projects, "Make a copy…", "Requires an open project and the desktop app for the destination-folder picker. Switching project or revision clears older relink/package results before another action can run."),
  "cut.top.library.saved": menuChoice("Saved assets", "Library menu", "Browse saved assets available for reuse.", "library.list", areas.libraryTop, opens.library, "Saved assets"),
  "cut.top.library.add": menuChoice("Add to project", "Library menu", "Add a saved library asset into the current project.", "library.add_to_project", areas.libraryTop, opens.library, "Add to project"),
  "cut.top.render.preview": menuChoice("Draft-quality check pass", "Render menu", "Set the quality preset to draft for a fast look at the whole timeline before committing to a delivery render.", "render.preview, render.final", areas.render, opens.render, "Quality preset", "Requires FFmpeg."),
  "cut.top.render.full": menuChoice("Render the timeline", "Render menu", "Render the whole timeline using the quality preset, delivery aspect and loudness target chosen in this panel. The deterministic checks run themselves and leave a receipt.", "render.final, verify.checks", areas.render, opens.render, "Quality preset", "Requires FFmpeg and an output folder."),
  "cut.top.render.range": menuChoice("Render selected range", "Preview monitor", "Mark a range on the timeline ruler or select clips, then use Render selection under the Preview monitor to render only that window.", "export.range, media.import", areas.renderSelectionButton, opens.render, "Quality preset", "Requires a marked range on the ruler or a selected clip."),
  "cut.top.export.video": menuChoice("Final video", "Export menu", "Export the project as a final video file.", "render.final", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Requires FFmpeg."),
  "cut.top.export.bundle": menuChoice("Platform exports", "Export menu", "Export for a platform's geometry and bitrate. The file is saved locally; Cut does not upload it. Presets: YouTube 16:9, TikTok/Shorts 9:16, Instagram Reels 9:16, or X 16:9. A multi-platform pack with captions and a thumbnail comes from the Social short bundle recipe instead.", "export.publish, render.bundle", areas.exportMenu, opens.exportMenu, "Platform exports", "Requires a renderable timeline and FFmpeg."),
  "cut.top.export.archive": menuChoice("Keeping a copy of a project", "Export menu", "There is no project-archive option in Export. To copy the project with its used media, choose Projects > Make a copy. A project-folder copy alone may still need the original linked media. Use Interchange (OTIO, EDL, XML) to hand the edit to another tool.", "export.otio, export.xml, export.edl", areas.exportMenu, opens.exportMenu, "Interchange XML / OTIO / EDL", "Requires an open project."),
  "cut.export.preflight": menuChoice("Preflight warnings", "Export menu", "Cut checks for likely quality problems before starting a video render. Review the warnings, then choose Export anyway or Queue anyway if the result is intentional. Missing media or video tools still need to be fixed.", "verify.pregate", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Requires a renderable timeline."),
  "cut.export.preflight.black_tail": menuChoice("Black ending", "Preflight warnings", "The timeline continues after the last video clip, so the export would end on black frames.", "verify.pregate empty_tail", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Trim the tail, shorten audio, or add picture, or export anyway if the black ending is intentional."),
  "cut.export.preflight.dead_frames": menuChoice("Black or frozen footage", "Preflight warnings", "A clip contains black or still frames in the edited range. Still footage can be intentional in a screen recording.", "verify.pregate black_or_frozen", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Trim or replace unwanted footage, or choose Export anyway to keep intentional still footage."),
  "cut.export.preflight.pacing": menuChoice("Long holds", "Preflight warnings", "The base story has very few cuts over a long timeline.", "verify.pregate slideshow_risk", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Add cuts, motion, or shorten holds if the static pacing is not intentional."),
  "cut.export.preflight.silent_audio": menuChoice("Silent export", "Preflight warnings", "The timeline appears to export without audible audio.", "verify.pregate silent_output", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Add or unmute audio, or continue if the video is intentionally silent."),
  "cut.export.preflight.tiny_clips": menuChoice("Tiny clips", "Preflight warnings", "One or more clips are shorter than a video frame and may not visibly render. In the timeline such a clip is an amber marker: select it and press Delete. New edits never make a clip shorter than one frame.", "verify.pregate tiny_or_zero_clips", areas.exportMenu, opens.exportMenu, "Video (.mp4)", "Delete the stray clip or extend it."),
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
