This changelog records release notes for ShellX Cut. For older versions, see the [GitHub releases page](https://github.com/martinsbrezauckis/shellx-cut/releases).

## 0.6.122 - 2026-11-24
- HDR videos can be exported as HDR: choose Keep HDR in the export options to get a 10-bit H.265 or AV1 file with the clip's own HDR type (HLG or PQ). The normal export is unchanged.
- Markers you flag while recording appear on the recording clip in Edit and move, trim and split with it. A short note confirms each marker while you record.
- Recordings made with 0.6.116 to 0.6.118 can be made editable: select the recording and choose Make editable.
- A conversation started by an agent appears in the Chat tab, marked "Started by an agent", and you can continue it. Conversations are saved in the project.
- Windows recordings: the camera recording is saved when you stop, short sounds such as clicks stay in sync with the picture, and a recording starts with movement instead of a frozen first picture.
- Playback keeps time on timelines with many short clips.
- Tracking and face blur on long clips export quickly.
- A rectangle camera overlay keeps the shape of its camera, and the export matches the preview.
- Before an export and on the finished file, every check is listed as passed, found something, not checked or could not run, with the reason.
- An empty timeline shows its hint in the middle of the panel.
- A take with a pause shows its length, and the length limit counts recorded time only.
- Error messages say what to do next.
- Many smaller fixes.

## 0.6.121 - 2026-11-24
- A picture in a project is no longer shown as offline, with editing blocked, when Cut could not check it at that moment; it is checked again.

## 0.6.120 - 2026-11-23
- Controls that do not apply are no longer shown. The timeline toolbar follows the selected clip: no Speed for captions; no Color, Transform, Remove background or GIF for sound. The source monitor shows only the targets a source has. Pause is shown only on macOS, where it exists.
- Dragging a clip from Assets onto the timeline overwrites at the drop position; hold Alt to insert it and push the rest to the right. A label shows which will happen.
- Record page: messages say in plain words what to do; a check that has not run says "Not checked"; markers and the timer follow the recorded time after a pause; a Start that is refused keeps you on the Record page and says why, and one refused by the checks leaves no empty project; the camera size slider is reachable in a small window.
- Export: the warning before an export says in plain words what was found or was not checked, and it also opens when the checks could not run. You always choose whether to export.
- A damaged picture file is refused when you add it instead of stalling the preview; in an existing project it shows as a file that cannot be read.
- Premium background removal uses updated tools and shows the real download size before the install. It needs a current NVIDIA driver, and cards of the 2017 Volta generation (Titan V, Quadro GV100) are no longer supported by it; existing installs keep working.
- In a wide window the preview stays visible beside Properties.
- Windows: saving the recent projects list, the Library and the search index no longer fails while another program reads the file.
- For agents: every refusal names the next step, including pause on Windows and Linux; the system report says what this system can do.
- Many smaller fixes.

## 0.6.119 - 2026-11-22
- Screen recordings: the zooms can be edited after recording. Select the recording in Edit and open Zoom in the Inspector: change a zoom's strength, target and timing, add or delete zooms, or redo the automatic ones, then press Update video.
- Screen recordings with a camera: the camera is on its own Camera track. Move it, resize it, change its shape, cut it or hide it for a stretch, or put it back as it was recorded. A switch on the Record page keeps the camera drawn into the video as before.
- Overlay clips can be a circle or a rounded square, with an optional ring.
- "Make a copy" of a project carries the original recordings, so their zooms stay editable in the copy.
- Record page: a paused recording says so in plain words and its timer stands still; the list of technical checks shows one check per line and closes when you click outside it.
- Rebuilding missing previews: one number says how many items need attention, and each skipped item says why.
- For agents: commands to list and edit a recording's zooms and to reset its camera.
- Many smaller fixes.

## 0.6.118 - 2026-11-21
- Clearer wording across the app and the manual, following the terms of the leading video editors.
- Light theme: every label is readable, and keyboard focus is visible.
- On Linux, recording says when no microphone is available before trying to use it.
- Reversing a video clip also reverses its linked sound.
- A 1080p recording at 60 fps is labelled High frame rate instead of 4K source.
- The setup status always shows the newest check and says when FFmpeg is older than the tested versions.
- Fixed: exports could lose a picture where clips that are not whole frames meet.
- Fixed: a new project lost the size you chose after a 4K import.
- Many smaller fixes.

## 0.6.117 - 2026-11-21
- Fixed: screen recordings on macOS and Windows keep moving as they are saved in parts.
- On Windows, Stop responds at once while the recording finishes saving in the background.
- The Record screen shows whether the current recording is paused.
- On Linux Wayland, screen recording no longer requires keyboard input access.
- On Windows and macOS, recording says when no microphone is available before trying to use it.
- On Windows, recording checks video support before starting and says why when a PC cannot record.
- Fixed: Add to timeline messages follow the right recording and project.
- Fixed: recordings already added to the timeline no longer keep offering recovery.
- Fixed: the chosen export folder is kept when updating from 0.6.115.

## 0.6.116 - 2026-11-21
- The preview shows the finished picture when you pause, with titles, effects and fades.
- Exports match the timeline exactly: picture and sound stay in sync on timelines with many cuts or mixed frame rates.
- Closing or quitting during a recording saves the recording first; during an export, ShellX Cut asks before stopping it.
- A recording or export cut short by a crash or power loss is reported at the next start; recordings come back with their sound and say how much was recovered.
- Stopping a screen recording is quicker, and a recording made while an export runs is kept.
- One export folder for all projects, remembered between sessions and shown in the status bar.
- FFmpeg can be downloaded from inside the app on Apple-silicon Macs too, and every download shows its publisher, version, size and licence first.
- When a feature needs a tool that is not installed, ShellX Cut says what is missing and offers the download in Settings, including Microsoft's Visual C++ runtime for captions on Windows.
- In the timeline, dragging a clip overwrites and Alt-dragging inserts, and the timeline names which one you are doing.
- An export that stops making progress ends with a clear message, and a render queue tells you when some of its exports failed.
- ShellX Cut opens in one window that fits your screen.
- Fixed: playback looping or jumping back to 00:00 when the middle part of a split clip is on a second video track (#2).
- Many smaller fixes.
