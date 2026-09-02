export function nativeRuntimeEnvPrefix({ arg, env, remotePath }) {
  const value = (name, fallback = '') => arg(name, fallback)
  return [
    `WDIO_PYTHON=${JSON.stringify(remotePath(value('--python', env.SHELLX_CUT_PYTHON || '')))}`,
    `WDIO_PYTHONPATH=${JSON.stringify(value('--pythonpath', env.PYTHONPATH || ''))}`,
    `WDIO_SIDECAR_DIR=${JSON.stringify(remotePath(value('--sidecar-dir', env.SHELLX_CUT_SIDECAR_DIR || '')))}`,
    `WDIO_MATTE_MODEL=${JSON.stringify(remotePath(value('--matte-model', env.MATTE_MODEL || '')))}`,
    `WDIO_FFMPEG=${JSON.stringify(remotePath(value('--ffmpeg', env.SHELLX_CUT_FFMPEG || env.FFMPEG_BIN || '')))}`,
    `WDIO_DIARIZE_ENDPOINT=${JSON.stringify(value('--diarize-endpoint', env.CUT_DIARIZE_ENDPOINT || ''))}`,
    `WDIO_DUB_ENDPOINT=${JSON.stringify(value('--dub-endpoint', env.CUT_DUB_ENDPOINT || ''))}`,
  ]
}

export const NATIVE_RUNTIME_ENV_SHELL = String.raw`
WDIO_PYTHON_RESOLVED="$(resolve_optional_path "$WDIO_PYTHON")"
WDIO_SIDECAR_DIR_RESOLVED="$(resolve_optional_path "$WDIO_SIDECAR_DIR")"
WDIO_MATTE_MODEL_RESOLVED="$(resolve_optional_path "$WDIO_MATTE_MODEL")"
WDIO_FFMPEG_RESOLVED="$(resolve_optional_path "$WDIO_FFMPEG")"
[ -n "$WDIO_SIDECAR_DIR_RESOLVED" ] || WDIO_SIDECAR_DIR_RESOLVED="$REMOTE_DIR_RESOLVED/app/perception/py"
[ -n "$WDIO_FFMPEG_RESOLVED" ] || WDIO_FFMPEG_RESOLVED=ffmpeg
export SHELLX_CUT_PYTHON="$WDIO_PYTHON_RESOLVED"
export PYTHONPATH="$WDIO_PYTHONPATH"
export SHELLX_CUT_SIDECAR_DIR="$WDIO_SIDECAR_DIR_RESOLVED"
export MATTE_MODEL="$WDIO_MATTE_MODEL_RESOLVED"
export MATTE_RUNNER_PY="$WDIO_PYTHON_RESOLVED"
export SHELLX_CUT_FFMPEG="$WDIO_FFMPEG_RESOLVED"
export FFMPEG_BIN="$WDIO_FFMPEG_RESOLVED"
export CUT_DIARIZE_ENDPOINT="$WDIO_DIARIZE_ENDPOINT"
export CUT_DUB_ENDPOINT="$WDIO_DUB_ENDPOINT"`
