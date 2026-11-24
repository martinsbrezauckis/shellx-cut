"""Optional speech-based refinement, kept separate from objective FFmpeg sound
measurement. Imports/models are loaded only inside an explicit instrument call.
"""
import attempt_evidence


@attempt_evidence.instrument("silence", "silero")
def silences(wav16k, minimum_seconds, log):
    import soundfile as sf
    import torch
    from silero_vad import load_silero_vad, get_speech_timestamps

    log("silence: silero-vad")
    model = load_silero_vad()
    data, sr = sf.read(wav16k, dtype="float32")
    if getattr(data, "ndim", 1) > 1:
        data = data.mean(axis=1)
    if sr != 16000:
        raise RuntimeError(f"expected 16 kHz extracted wav, got {sr}")
    wav = torch.from_numpy(data.copy())  # 16 kHz mono; avoids torchaudio I/O.
    speech = get_speech_timestamps(wav, model)
    total_ms = int(len(wav) / 16000 * 1000)

    # Invert speech to silence, including the head and tail.
    min_ms = int(minimum_seconds * 1000)
    result = []
    cursor = 0
    for span in speech:
        start_ms, end_ms = int(span["start"] / 16), int(span["end"] / 16)
        if start_ms - cursor >= min_ms:
            result.append((cursor, start_ms))
        cursor = end_ms
    if total_ms - cursor >= min_ms:
        result.append((cursor, total_ms))
    return result
