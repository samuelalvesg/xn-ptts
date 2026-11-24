#!/usr/bin/env python3
"""Speak through a running ptts-ws-server, one session per language, and write the WAVs.

    ptts-ws-server --model hf://gradium/phonon-40m-multilingual --lang en --max-concurrent-requests 4
    python scripts/e2e-lang/client.py --out out

Four sessions in different languages run at the same time on the one server, Spanish follows,
then the edge cases of `setup`: an invalid `lang`, a language the checkpoint does not list, an
empty `lang`, `none`, an unknown voice, and no `lang` at all. Each session names a catalog voice
of its own language; `lang` only decides how the text is read. Needs `pip install websockets`.
Prints the sessions' timings and writes `<out>/results.json` for `transcribe.py`.
"""

import argparse
import asyncio
import base64
import glob
import json
import os
import time
import wave

import websockets

TEXT = {
    "pt": "Temos R$ 10,50 e a Dra. Ana, em 2 de março, às 15h.",
    "en": "We have $10.50 and Dr. Smith, on March 2nd at 3pm.",
    "fr": "Nous avons 10,50 € et le Dr Martin, le 2 mars à 15 h.",
    "de": "Wir haben 10,50 € und Dr. Müller, am 2. März um 15 Uhr.",
    "es": "Tenemos 10,50 € y la Dra. Ana, el 2 de marzo a las 15 h.",
}


def first_voice_per_language(catalog: str) -> dict[str, str]:
    voices: dict[str, str] = {}
    for line in open(catalog, encoding="utf-8"):
        voice = json.loads(line)
        if voice.get("language"):
            voices.setdefault(voice["language"], voice["name"])
    return voices


def default_catalog() -> str:
    found = glob.glob(
        os.path.expanduser("~/.cache/huggingface/hub/models--gradium--phonon-voices/snapshots/*/voices.jsonl")
    )
    if not found:
        raise SystemExit("no voice catalog in the Hugging Face cache: pass --voices-jsonl")
    return found[0]


async def speak(url, out_dir, tag, lang, voice, text):
    started = time.time()
    pcm, first = b"", None
    async with websockets.connect(url, max_size=None) as ws:
        setup = {"type": "setup", "output_format": "pcm"}
        if voice:
            setup["voice"] = voice
        if lang is not None:
            setup["lang"] = lang
        await ws.send(json.dumps(setup))
        ready = json.loads(await ws.recv())
        if ready["type"] != "ready":
            return tag, {"error": ready}, None
        sample_rate = ready["sample_rate"]
        await ws.send(json.dumps({"type": "text", "text": text}))
        await ws.send(json.dumps({"type": "end_of_stream"}))
        async for raw in ws:
            msg = json.loads(raw)
            if msg["type"] == "audio":
                first = first if first is not None else time.time() - started
                pcm += base64.b64decode(msg["audio"])
            elif msg["type"] == "end_of_stream":
                break
            elif msg["type"] == "error":
                return tag, {"error": msg}, None
    path = os.path.join(out_dir, f"{tag}.wav")
    with wave.open(path, "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(sample_rate)
        wav.writeframes(pcm)
    info = {
        "seconds": round(len(pcm) / 2 / sample_rate, 2),
        "first_chunk_s": round(first or -1, 2),
        "total_s": round(time.time() - started, 2),
    }
    return tag, info, path


async def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--url", default="ws://127.0.0.1:8080/speech/tts")
    parser.add_argument("--out", default="out")
    parser.add_argument("--voices-jsonl", default=None, help="the voice catalog; default: the Hugging Face cache")
    args = parser.parse_args()
    os.makedirs(args.out, exist_ok=True)
    voices = first_voice_per_language(args.voices_jsonl or default_catalog())
    run = lambda tag, lang, voice, text: speak(args.url, args.out, tag, lang, voice, text)

    results: dict[str, dict] = {}
    print("== four sessions at once, one language each; Spanish after")
    started = time.time()
    done = await asyncio.gather(*[run(f"concurrent-{l}", l, voices[l], TEXT[l]) for l in ("pt", "en", "fr", "de")])
    print("wall clock:", round(time.time() - started, 1), "s")
    done.append(await run("concurrent-es", "es", voices["es"], TEXT["es"]))
    edge = [
        ("lang-invalid", "xx", voices["en"], TEXT["en"]),
        ("lang-not-listed-ja", "ja", voices["en"], TEXT["en"]),
        ("lang-empty", "", voices["en"], TEXT["en"]),
        ("lang-none", "none", voices["en"], "The sum is 10 plus 5, written as 10+5."),
        ("unknown-voice", "pt", "NoSuchVoice", TEXT["pt"]),
        ("no-lang-uses-the-servers", None, voices["en"], TEXT["en"]),
    ]
    print("== edge cases")
    for case in edge:
        done.append(await run(*case))
    for tag, info, path in done:
        print(f"{tag:28s} {info}")
        results[tag] = {"path": path, "info": info}
    with open(os.path.join(args.out, "results.json"), "w", encoding="utf-8") as f:
        json.dump(results, f, indent=1)


asyncio.run(main())
