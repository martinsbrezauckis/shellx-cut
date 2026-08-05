# Sourced by linux-wdio-full-coverage.mjs after its bounded runtime and output
# paths have been resolved. This owns only processes created by the current run
# and preserves retained evidence when rebuildable state is removed.
before_cutd=" $(pgrep -x cutd 2>/dev/null | tr '\n' ' ' || true)"
before_app=" $(pgrep -x shellx-cut 2>/dev/null | tr '\n' ' ' || true)"
gate_pid=""

cleanup_new_processes() {
  for process_name in cutd shellx-cut; do
    if [ "$process_name" = "cutd" ]; then before="$before_cutd"; else before="$before_app"; fi
    for pid in $(pgrep -x "$process_name" 2>/dev/null || true); do
      case " $before " in
        *" $pid "*) ;;
        *) kill "$pid" >/dev/null 2>&1 || true ;;
      esac
    done
  done
}

cleanup_run() {
  run_status=$?
  cleanup_status=0
  trap - EXIT
  if [ -n "$gate_pid" ]; then
    kill -TERM -- "-$gate_pid" >/dev/null 2>&1 || true
    for _cleanup_wait in 1 2 3 4 5; do kill -0 -- "-$gate_pid" >/dev/null 2>&1 || break; sleep 1; done
    kill -KILL -- "-$gate_pid" >/dev/null 2>&1 || true
  fi
  cleanup_new_processes
  if [ "$CLEAN_AFTER" = "1" ]; then
    rm -rf app/target app/desktop/src-tauri/target ui/node_modules ui/dist \
      "$WDIO_OUT_RESOLVED/package-root" "$WDIO_OUT_RESOLVED/app-home" "$WDIO_OUT_RESOLVED/projects" \
      || cleanup_status=1
    # xdg-document-portal can leave FUSE mounts below the private runtime on a
    # sick/overloaded host. Never recurse into a live mount: that can strand the
    # release wrapper in uninterruptible I/O. Read the kernel mount table
    # directly because stat-based probes can themselves block on wedged FUSE.
    runtime_mounted=0
    for runtime_mount in "$runtime_dir/doc" "$runtime_dir/gvfs"; do
      if awk -v target="$runtime_mount" '$5 == target { found=1 } END { exit !found }' /proc/self/mountinfo; then
        echo "FAIL: cleanup blocked by mounted portal runtime: $runtime_mount" >&2
        runtime_mounted=1
      fi
    done
    if [ "$runtime_mounted" = "0" ]; then
      rm -rf "$runtime_dir" || cleanup_status=1
    else
      cleanup_status=1
    fi
  fi
  if [ "$run_status" != "0" ]; then exit "$run_status"; fi
  if [ "$cleanup_status" != "0" ]; then exit 86; fi
  exit 0
}

trap cleanup_run EXIT
