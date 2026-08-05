#!/usr/bin/env python3
"""translate_runner.py — the SHIPPABLE local TEXT-TRANSLATION runtime (one-shot CLI).

Role: translate subtitle/transcript SEGMENTS offline, on the user's own machine,
when no subscription CLI agent is available. This is the LOCAL FALLBACK for
`captions.translate` / `transcript.translate`; the PRIMARY path is the user's
own claude/codex/grok CLI (server/translate.rs). cutd spawns this exactly like
the OCR / face / matte runners — one-shot CLI, no port, no 2nd window — feeding
the segments as JSON on STDIN and reading the translations as JSON on STDOUT.

TEXT ONLY: no audio, no TTS, no dubbing.

Model choice (server/translate.rs documents the tradeoff):
- DEFAULT = Opus-MT (Helsinki-NLP, MarianMT). LIGHT (~300 MB per pair), CC-BY-4.0
  (commercial-OK), per-pair. The model id is `Helsinki-NLP/opus-mt-{src}-{tgt}`;
  if that 404s we fall back to the `opus-mt-tc-big-{src}-{tgt}` variant (the
  bigger Tatoeba-Challenge models, e.g. for en-lv).
- UNIVERSAL alternative = MADLAD-400 (Google, Apache-2.0, 419 langs incl.
  Latvian, ~3B). Heavier; selected by passing `--model jbochi/madlad400-3b-mt`
  (or a CTranslate2-int8 build for speed — a documented future optimization).
  MADLAD is a T5 model needing a `<2{tgt}>` target-language token prefix, which
  this runner adds automatically when the model id contains "madlad".
- NLLB-200 is intentionally NOT supported (CC-BY-NC, non-commercial).

The model DOWNLOAD is gated behind FIRST USE: transformers fetches + caches the
model on the first run; an offline run with no cached model FAILS HONESTLY
(non-zero exit + a clear stderr message) — nothing fake is emitted, and no
multi-GB model is bundled with the app.

Contract (matches server/translate.rs):
  python translate_runner.py --src <code> --tgt <code> [--model <hf_id>]
                             [--max-new-tokens N]
  STDIN  = {"segments": ["text one", "text two", ...]}
  STDOUT = ONE JSON line:
    {"translations": ["...","..."], "model": "<hf_id>", "backend": "opus-mt"|"madlad"}
  (translations has EXACTLY the same length + order as segments). Non-zero exit
  + stderr on any failure (missing deps, unknown pair, offline+uncached).

Dependencies: transformers, sentencepiece, torch (CPU is fine for Opus-MT).
Primary caller: server/translate.rs (local-CLI transport).
"""
from __future__ import annotations

import argparse
import json
import sys

try:
    sys.stdin.reconfigure(encoding="utf-8")
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8", errors="backslashreplace")
except AttributeError:
    print(
        "translate_runner: stream encoding reconfigure unavailable; continuing with process defaults",
        file=sys.stderr,
    )


def _eprint(*a):
    print(*a, file=sys.stderr)


def _fail(msg: str, code: int = 1):
    _eprint(f"translate_runner: {msg}")
    sys.exit(code)


def _load(model_id: str):
    """Load tokenizer + seq2seq model, returning (tok, model). Honest, specific
    errors for the two common failures (missing deps / model fetch)."""
    try:
        from transformers import AutoModelForSeq2SeqLM, AutoTokenizer  # type: ignore
    except Exception as e:  # noqa: BLE001
        _fail(
            "transformers is not installed in the perception venv "
            f"(pip install transformers sentencepiece torch): {e}"
        )
    try:
        tok = AutoTokenizer.from_pretrained(model_id)
        model = AutoModelForSeq2SeqLM.from_pretrained(model_id)
    except Exception as e:  # noqa: BLE001
        # Distinguish "no such pair / offline" from other errors as best we can.
        raise RuntimeError(str(e)) from e
    model.eval()
    return tok, model


def _resolve_models(src: str, tgt: str, explicit: str | None) -> list[tuple[str, str]]:
    """Ordered list of (model_id, backend_tag) candidates to try."""
    if explicit:
        tag = "madlad" if "madlad" in explicit.lower() else "custom"
        return [(explicit, tag)]
    # Opus-MT default, then the tc-big variant for pairs the small model lacks.
    return [
        (f"Helsinki-NLP/opus-mt-{src}-{tgt}", "opus-mt"),
        (f"Helsinki-NLP/opus-mt-tc-big-{src}-{tgt}", "opus-mt"),
    ]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", required=True, help="source language code, e.g. en")
    ap.add_argument("--tgt", required=True, help="target language code, e.g. es / lv")
    ap.add_argument("--model", default=None, help="explicit HF model id (e.g. a MADLAD-400 build)")
    ap.add_argument("--max-new-tokens", type=int, default=512)
    args = ap.parse_args()

    src = args.src.strip().lower()
    tgt = args.tgt.strip().lower()

    raw = sys.stdin.read()
    try:
        payload = json.loads(raw) if raw.strip() else {}
        segments = list(payload.get("segments", []))
    except Exception as e:  # noqa: BLE001
        _fail(f"could not parse STDIN JSON ({e}); expected {{\"segments\":[...]}}")
        return
    if not segments:
        # Nothing to translate is not an error — emit an empty result.
        print(json.dumps({"translations": [], "model": args.model or "", "backend": "opus-mt"}))
        return

    candidates = _resolve_models(src, tgt, args.model)
    last_err = None
    tok = model = None
    chosen_id = chosen_tag = None
    for model_id, tag in candidates:
        try:
            tok, model = _load(model_id)
            chosen_id, chosen_tag = model_id, tag
            break
        except RuntimeError as e:
            last_err = e
            continue
    if model is None:
        _fail(
            f"could not load a translation model for {src}->{tgt} "
            f"(tried: {[c[0] for c in candidates]}). "
            "On first use the model is downloaded; this fails offline or for an "
            f"unsupported pair. Last error: {last_err}"
        )
        return

    is_madlad = chosen_tag == "madlad"
    outputs: list[str] = []
    import torch  # type: ignore  # present whenever transformers loaded

    with torch.no_grad():
        for seg in segments:
            text = (seg or "").strip()
            if not text:
                outputs.append("")
                continue
            # MADLAD (T5) needs an explicit target-language token prefix.
            model_input = f"<2{tgt}> {text}" if is_madlad else text
            enc = tok(model_input, return_tensors="pt", truncation=True, max_length=512)
            gen = model.generate(**enc, max_new_tokens=args.max_new_tokens, num_beams=4)
            outputs.append(tok.batch_decode(gen, skip_special_tokens=True)[0].strip())

    print(json.dumps({"translations": outputs, "model": chosen_id, "backend": chosen_tag}))


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception as e:  # noqa: BLE001
        _fail(f"unexpected error: {e}")
