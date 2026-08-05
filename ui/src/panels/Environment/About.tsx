// About.tsx — compact About + version block for the Environment drawer (the
// updater, version, and update-disclosure surface.
//
// The version comes from the engine doctor's `app_version` (system.doctor): the
// Cut UI is SERVED BY cutd (a remote loopback origin) where Tauri 2 blocks
// plugin/command IPC, so the engine — not a Tauri call — is the version source
// of truth. The auto-updater itself runs in the desktop SHELL (checks the
// signed release feed on launch, prompts via a native dialog before restarting);
// this panel just REPORTS the version + disclosed default. Kept compact
// as concise status rather than prose. Relay-drivable via the data-cut-* hooks.
//
// Callers: EnvironmentPanel (Settings > Environment). Deps: lib/doctor.

import type { DoctorReport } from '../../lib/doctor'

export default function About({ report }: { report: DoctorReport | null }) {
  const version = report?.app_version ?? '—'
  return (
    <section className="env-about" data-cut-about data-cut-app-version={version}>
      <div className="env-about-row">
        <span className="env-about-name">ShellX Cut</span>
        <span className="env-about-version" data-cut-about-version>
          v{version}
        </span>
      </div>
      <p className="env-about-tag">
        Agent-first video editor — every edit is a verb, every render ships a measured receipt.
      </p>
      <p className="env-about-update" data-cut-about-update>
        The installed app checks GitHub for signed releases on launch by default. You can turn this off under Storage &amp; privacy; installing still asks before restart.
      </p>
      <div className="env-about-links">
        <a href="https://theshellx.com" target="_blank" rel="noopener noreferrer">
          theshellx.com
        </a>
        <span className="env-about-dot">·</span>
        <a
          href="https://github.com/martinsbrezauckis/shellx-cut"
          target="_blank"
          rel="noopener noreferrer"
        >
          GitHub
        </a>
        <span className="env-about-dot">·</span>
        <span className="env-about-license">MIT</span>
      </div>
    </section>
  )
}
