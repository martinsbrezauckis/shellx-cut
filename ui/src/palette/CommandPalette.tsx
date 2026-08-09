// CommandPalette.tsx — Cmd-K / Ctrl-K command palette.
//
// The agent-first discoverability surface: every editing tool one keystroke away,
// the win the native incumbents lack. Behaviour matches the contract:
//   • INSTANT summon (no enter animation — Raycast's deliberate choice for a
//     surface used dozens of times a day; the panel just appears).
//   • fuzzy rank over label + keywords + group.
//   • keyboard-first: ↑/↓ move, ↵ run, esc close; hover also sets the active row.
//   • right-aligned hints + group tags teach the keyboard in context (Linear).
// Self-contained: owns its open state + the global ⌘K listener; actions fire
// through Cut's existing `cut:*` event bus (see commands.ts), so it drives the real
// tools. Mounted once near the other overlays in App.
import { useEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../icons";
import { isEditableTarget } from "../lib/dom";
import { useBlockingOverlay } from "../components/overlay/useBlockingOverlay";
import { COMMANDS, type Command } from "./commands";
import "./palette.css";

/**
 * Fuzzy score. -1 = no match (excluded); higher = better. Substring matches
 * (label > group > keyword) rank above subsequence matches, and a SPARSE
 * subsequence (the matched chars span far more than the query length) is rejected
 * — so e.g. "audio" doesn't drag in "Caption styles" via a scattered c-A-…-d-…-o.
 */
function score(cmd: Command, q: string): number {
  const needle = q.toLowerCase().trim();
  if (!needle) return 1;
  const label = cmd.label.toLowerCase();
  const kw = (cmd.keywords ?? "").toLowerCase();
  const grp = cmd.group.toLowerCase();
  // 1) strong substring matches, ranked by where the hit lands
  if (label.startsWith(needle)) return 1000 - label.length; // prefix wins, shorter first
  if (label.includes(needle)) return 850;
  if (grp === needle) return 700;          // exact group, e.g. "audio"
  if (kw.includes(needle)) return 600;     // a keyword contains the query
  if (grp.includes(needle)) return 500;
  // 2) subsequence fallback (typo tolerance) over label + keywords only
  const hay = `${label} ${kw}`;
  let at = 0, sc = 0, streak = 0, first = -1, last = -1;
  for (const ch of needle) {
    const idx = hay.indexOf(ch, at);
    if (idx === -1) return -1;
    if (first < 0) first = idx;
    last = idx;
    if (idx === at) { sc += 5 + streak; streak += 1; } else { sc += 1; streak = 0; }
    at = idx + 1;
  }
  // reject very sparse subsequences (avoids unrelated rows surfacing on short queries)
  if (needle.length >= 3 && last - first > needle.length * 4) return -1;
  return sc;
}

function rank(q: string): Command[] {
  if (!q.trim()) return COMMANDS;
  return COMMANDS.map((c) => ({ c, s: score(c, q) }))
    .filter((x) => x.s > 0)
    .sort((a, b) => b.s - a.s)
    .map((x) => x.c);
}

export function CommandPalette() {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const close = () => setOpen(false);
  const overlay = useBlockingOverlay<HTMLDivElement>(close, open);

  // Global ⌘K / Ctrl-K toggles the palette from anywhere (its own listener — the
  // app's main keydown handler bails on meta/ctrl, so this doesn't collide).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isEditableTarget(e.target)) return;
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Reset + focus the moment it opens (instant — no animation frame delay beyond
  // the one needed for the input to mount).
  useEffect(() => {
    if (open) {
      setQuery("");
      setActive(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const results = useMemo(() => rank(query), [query]);

  // Keep the active index in range as results shrink.
  useEffect(() => {
    setActive((a) => Math.min(a, Math.max(0, results.length - 1)));
  }, [results.length]);

  // Keep the active row scrolled into view during keyboard nav.
  useEffect(() => {
    if (!open) return;
    listRef.current?.querySelector(".is-active")?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  if (!open) return null;

  const runAt = (i: number) => {
    const c = results[i];
    if (c) { close(); c.run(); }
  };

  const onInputKey = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") { e.preventDefault(); setActive((a) => Math.min(a + 1, results.length - 1)); }
    else if (e.key === "ArrowUp") { e.preventDefault(); setActive((a) => Math.max(a - 1, 0)); }
    else if (e.key === "Enter") { e.preventDefault(); runAt(active); }
  };

  return (
    <div className="cmdk-scrim" onMouseDown={overlay.onScrimMouseDown}>
      <div
        ref={overlay.dialogRef}
        className="cmdk"
        data-cut-command-palette
        data-cut-blocking-overlay
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="cmdk-input-row">
          <Icon name="search" size={16} className="cmdk-search-icon" />
          <input
            ref={inputRef}
            className="cmdk-input"
            data-cut-command-search
            placeholder="Search commands…"
            value={query}
            spellCheck={false}
            onChange={(e) => { setQuery(e.target.value); setActive(0); }}
            onKeyDown={onInputKey}
          />
          <kbd className="cmdk-kbd cmdk-esc">esc</kbd>
        </div>

        <div className="cmdk-list" ref={listRef}>
          {results.length === 0 && (
            <div className="cmdk-empty">No commands match “{query}”.</div>
          )}
          {results.map((c, i) => (
            <button
              key={c.id}
              type="button"
              className={`cmdk-row${i === active ? " is-active" : ""}`}
              data-cut-command={c.id}
              data-cut-command-manual={c.manualId ?? undefined}
              title={c.description ?? c.label}
              onMouseMove={() => setActive(i)}
              onClick={() => runAt(i)}
            >
              <Icon name={c.icon} size={16} className="cmdk-row-icon" />
              <span className="cmdk-row-main">
                <span className="cmdk-row-label">{c.label}</span>
                {c.description && <span className="cmdk-row-desc">{c.description}</span>}
              </span>
              <span className="cmdk-row-meta">
                {c.hint && <kbd className="cmdk-kbd cmdk-row-hint">{c.hint}</kbd>}
                <span className="cmdk-row-group">{c.group}</span>
              </span>
            </button>
          ))}
        </div>

        <div className="cmdk-foot">
          <span className="cmdk-foot-keys"><kbd className="cmdk-kbd">↑</kbd><kbd className="cmdk-kbd">↓</kbd> navigate</span>
          <span className="cmdk-foot-keys"><kbd className="cmdk-kbd">↵</kbd> run</span>
          <span className="cmdk-foot-brand">ShellX Cut · ⌘K</span>
        </div>
      </div>
    </div>
  );
}
