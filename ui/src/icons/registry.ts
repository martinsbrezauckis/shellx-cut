// registry.ts — the ONE source of truth for every glyph in ShellX Cut.
//
// Why a curated registry (not `import { icons } from 'lucide-react'`): named
// imports tree-shake to only the glyphs Cut actually uses, and a fixed semantic
// map means surfaces reference an INTENT ("split", "record") not a vendor name —
// so we could swap icon libraries without touching a single surface. Native
// Lucide + the 6 custom NLE glyphs are unified under one `IconName` union; callers
// can't tell which are native.
//
// Semantic naming rule: name by EDITOR INTENT, not by shape. `split` not
// `scissors`; `record` not `disc`. One name per concept (aliases are fine where a
// concept legitimately appears under two words). To add a glyph: pick its Lucide
// equivalent from https://lucide.dev/icons, add a line here — never inline an SVG.
import type { LucideIcon } from "lucide-react";
import {
  // ── transport ──
  Play, Pause, Square, SkipBack, SkipForward, Rewind, FastForward, Repeat,
  // ── timeline tools ──
  Scissors, Magnet, ZoomIn, ZoomOut, Flag, Bookmark, Maximize2, Minimize2, SquareDashed,
  // ── media / clips ──
  Film, AudioLines, AudioWaveform, Image, Type, Shapes, Sticker, Layers,
  // ── edit ops ──
  Crop, Move, RotateCw, RotateCcw, Gauge, Diamond, Palette, Contrast, Sparkles,
  Wand2, Blend, EyeOff, Droplet, FlipHorizontal2,
  // ── audio ──
  SlidersVertical, SlidersHorizontal, Volume2, VolumeX, Music, Mic,
  // ── captions / text ──
  Captions, FileText, AlignLeft, CornerDownLeft,
  // ── record / screen ──
  Disc, Monitor, MonitorPlay, PictureInPicture2, Video, Webcam,
  // ── render / export ──
  Upload, Download, Share2, Send, Clapperboard as Render, ImageDown,
  // ── project / files ──
  FolderOpen, Folder, FolderInput, File, Save, LibraryBig, Search, Database,
  Star, Clock, BookOpen,
  // ── review / qc / agent ──
  MessageSquare, ListChecks, BadgeCheck, GitCompare, ReceiptText, Bot,
  // ── status / feedback ──
  Check, X, CircleCheck, CircleAlert, TriangleAlert, Info, Loader,
  // ── ui chrome / nav ──
  Plus, Minus, ChevronLeft, ChevronRight, ChevronDown, ChevronUp,
  ChevronsLeft, ChevronsRight, MoreHorizontal, MoreVertical, Settings,
  GripVertical, Eye, Lock, Link2, Copy, Trash2, Pencil, Undo2, Redo2,
  LayoutGrid, List, Grid3x3, PanelRight, PanelLeft, Cpu, Zap, Maximize,
  ClipboardPaste, Paperclip, Sun, Moon,
} from "lucide-react";
import { PlayheadScrub, RippleDelete, Crossfade, Mask, Matte, Stabilize } from "./custom";

/**
 * The semantic glyph registry. Key = editor intent; value = the Lucide (or custom)
 * component. Every `<Icon name=…>` in the app resolves through this map.
 */
export const REGISTRY = {
  // transport
  play: Play, pause: Pause, stop: Square, skipBack: SkipBack, skipForward: SkipForward,
  rewind: Rewind, fastForward: FastForward, loop: Repeat,
  // timeline tools
  split: Scissors, snap: Magnet, zoomIn: ZoomIn, zoomOut: ZoomOut,
  marker: Flag, bookmark: Bookmark, playhead: PlayheadScrub, rippleDelete: RippleDelete,
  liftDelete: SquareDashed, fitTimeline: Maximize2, collapseTimeline: Minimize2,
  // media / clips
  videoClip: Film, audioClip: AudioLines, waveform: AudioWaveform, image: Image,
  text: Type, shape: Shapes, sticker: Sticker, layers: Layers, film: Film,
  // edit ops
  crop: Crop, transform: Move, rotateCw: RotateCw, reset: RotateCcw, speed: Gauge,
  keyframe: Diamond, crossfade: Crossfade, grade: Palette, contrast: Contrast,
  effect: Sparkles, autopilot: Wand2, blend: Blend, redact: EyeOff, color: Droplet,
  flip: FlipHorizontal2, mask: Mask, matte: Matte, stabilize: Stabilize,
  // audio
  mixer: SlidersVertical, sliders: SlidersHorizontal, volume: Volume2, mute: VolumeX,
  music: Music, mic: Mic,
  // captions / text
  captions: Captions, transcript: FileText, alignText: AlignLeft,
  // record / screen
  record: Disc, screenCapture: Monitor, screenPlay: MonitorPlay,
  pip: PictureInPicture2, video: Video, webcam: Webcam,
  // render / export
  render: Render, export: Upload, download: Download, share: Share2, publish: Send,
  exportFrame: ImageDown,
  // project / files
  projectOpen: FolderOpen, folder: Folder, import: FolderInput, file: File,
  save: Save, library: LibraryBig, search: Search, assets: Database, manual: BookOpen,
  favorite: Star, pending: Clock,
  // review / qc / agent
  comment: MessageSquare, qc: ListChecks, verified: BadgeCheck, diff: GitCompare,
  receipt: ReceiptText, agent: Bot,
  // status / feedback
  check: Check, close: X, success: CircleCheck, error: CircleAlert,
  warning: TriangleAlert, info: Info, spinner: Loader,
  // ui chrome / nav
  plus: Plus, minus: Minus,
  chevronLeft: ChevronLeft, chevronRight: ChevronRight, chevronDown: ChevronDown,
  chevronUp: ChevronUp, collapseLeft: ChevronsLeft, collapseRight: ChevronsRight,
  moreH: MoreHorizontal, moreV: MoreVertical, settings: Settings, drag: GripVertical,
  eye: Eye, lock: Lock, link: Link2, copy: Copy, trash: Trash2, edit: Pencil,
  // clipboard intents: `cut` reuses the scissors glyph (intent = remove-to-
  // clipboard), `paste` is the clipboard-paste glyph. `copy` above doubles as
  // the copy intent.
  cut: Scissors, paste: ClipboardPaste, attach: Paperclip,
  return: CornerDownLeft,
  undo: Undo2, redo: Redo2, grid: LayoutGrid, list: List, gridDense: Grid3x3,
  panelRight: PanelRight, panelLeft: PanelLeft, gpu: Cpu, bolt: Zap, fullscreen: Maximize,
  fullscreenExit: Minimize2,
  // appearance / theme — sun = light is active, moon = dark is active
  themeLight: Sun, themeDark: Moon,
} satisfies Record<string, LucideIcon>;

/** Every valid icon name — a typo or unknown glyph fails the TypeScript build. */
export type IconName = keyof typeof REGISTRY;
