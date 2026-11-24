# Reads the pinned Windows FFmpeg download (base URL, file, byte size, SHA-256, bin folder) from
# app/media/src/tool_setup/download.rs, one value per line, so CI downloads exactly what the app pins.
# Used by .github/workflows/ci.yml (Install Windows test deps): awk -f scripts/release/windows-ffmpeg-pin.awk <download.rs>
function unquote(line) {
  if (match(line, /"[^"]*"/)) {
    return substr(line, RSTART + 1, RLENGTH - 2)
  }
  return ""
}
{ gsub(/\r/, "") }
/^[[:space:]]*\/\// { next }
pending_const != "" {
  val = unquote($0)
  if (val != "") {
    consts[pending_const] = val
    pending_const = ""
  }
  next
}
/^[[:space:]]*(pub[[:space:]]+)?const[[:space:]]+[A-Z0-9_]+:[[:space:]]*&str[[:space:]]*=/ {
  name = $0
  sub(/^[[:space:]]*(pub[[:space:]]+)?const[[:space:]]+/, "", name)
  sub(/:.*/, "", name)
  val = unquote($0)
  if (val != "") consts[name] = val
  else pending_const = name
  next
}
/^pub const WINDOWS_X64_FFMPEG:/ { in_win = 1; in_members = 0; next }
in_win && /^};/ { in_win = 0; next }
in_win && /^[[:space:]]*members:[[:space:]]*&\[/ { in_members = 1; next }
in_win && !in_members && /^[[:space:]]*base:[[:space:]]*/ {
  val = unquote($0)
  if (val != "") base = val
  else {
    base_ident = $0
    sub(/^[[:space:]]*base:[[:space:]]*/, "", base_ident)
    sub(/,.*/, "", base_ident)
    gsub(/[[:space:]]/, "", base_ident)
  }
}
in_win && !in_members && /^[[:space:]]*file:[[:space:]]*"/ { file = unquote($0) }
in_win && !in_members && /^[[:space:]]*size:[[:space:]]*/ {
  s = $0
  sub(/^[[:space:]]*size:[[:space:]]*/, "", s)
  sub(/,.*/, "", s)
  gsub(/[_[:space:]]/, "", s)
  size = s
}
in_win && !in_members && /^[[:space:]]*sha256:[[:space:]]*"/ { sha256 = tolower(unquote($0)) }
in_win && in_members && /^[[:space:]]*path:[[:space:]]*"/ {
  p = unquote($0)
  if (p ~ /\/ffmpeg\.exe$/) ffmpeg_path = p
  else if (first_member_path == "") first_member_path = p
}
END {
  if (base == "" && base_ident in consts) base = consts[base_ident]
  member = (ffmpeg_path != "" ? ffmpeg_path : first_member_path)
  bin_rel = member
  sub(/\/[^\/]*$/, "", bin_rel)
  if (base == "" || file == "" || size == "" || sha256 == "" || bin_rel == "" || bin_rel == member) {
    exit 1
  }
  printf "%s\n%s\n%s\n%s\n%s\n", base, file, size, sha256, bin_rel
}
