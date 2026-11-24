#!/usr/bin/env python3
"""Transcribe the WAVs `client.py` wrote, to check each session read its text in its language.

    python scripts/e2e-lang/transcribe.py --out out

The numbers and abbreviations are what tell the languages apart: `R$ 10,50` and `Dra.` should come
back as "reais" and "doutora" in the `pt` session, `$10.50` and `Dr.` as written out English in the
`en` one. Needs `pip install faster-whisper` (a multilingual model, `base`, is downloaded).
"""

import argparse
import json
import os
import wave

import numpy as np
from faster_whisper import WhisperModel

SPOKEN = {
    "concurrent-pt": "pt", "concurrent-en": "en", "concurrent-fr": "fr", "concurrent-de": "de",
    "concurrent-es": "es", "lang-not-listed-ja": "en", "lang-empty": "en", "lang-none": "en",
    "no-lang-uses-the-servers": "en",
}


def to_16k(pcm: np.ndarray, rate: int) -> np.ndarray:
    """Resample in the frequency domain: faster-whisper's own decoding breaks with some PyAV releases."""
    n = len(pcm) * 16000 // rate
    return (np.fft.irfft(np.fft.rfft(pcm)[: n // 2 + 1], n) * n / len(pcm)).astype(np.float32)


parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
parser.add_argument("--out", default="out")
args = parser.parse_args()
model = WhisperModel("base", device="cpu", compute_type="int8")
results = json.load(open(os.path.join(args.out, "results.json"), encoding="utf-8"))
for tag, language in SPOKEN.items():
    path = (results.get(tag) or {}).get("path")
    if not path:
        print(f"{tag:28s}| (no audio)")
        continue
    with wave.open(path) as wav:
        rate = wav.getframerate()
        pcm = np.frombuffer(wav.readframes(wav.getnframes()), dtype="<i2") / 32768
    segments, _ = model.transcribe(to_16k(pcm, rate), language=language, beam_size=5, condition_on_previous_text=False)
    print(f"{tag:28s}| {' '.join(s.text.strip() for s in segments)}")
