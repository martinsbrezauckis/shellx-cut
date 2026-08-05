// icons/ — the normalized icon system for ShellX Cut.
// Surfaces import ONLY from here: `import { Icon } from "@/icons"` (or a relative
// path). Direct `lucide-react` imports and inline <svg> outside this folder are
// banned by the icon tripwire (scripts/icon-svg-tripwire.sh).
export { Icon, type IconProps, type IconSize, type IconName } from "./Icon";
export { REGISTRY } from "./registry";
export { BrandMark } from "./BrandMark";
