#!/usr/bin/env python3
"""Write `web/voices.json`, the catalog voices by language, for the page in `web/`.

    python scripts/e2e-lang/voices_json.py [--voices-jsonl voices.jsonl]
"""

import argparse
import glob
import json
import os

parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
parser.add_argument("--voices-jsonl", default=None)
args = parser.parse_args()
catalog = args.voices_jsonl or (
    glob.glob(os.path.expanduser("~/.cache/huggingface/hub/models--gradium--phonon-voices/snapshots/*/voices.jsonl"))
    or [None]
)[0]
if not catalog:
    raise SystemExit("no voice catalog in the Hugging Face cache: pass --voices-jsonl")
by_language: dict[str, list[str]] = {}
for line in open(catalog, encoding="utf-8"):
    voice = json.loads(line)
    if voice.get("language"):
        by_language.setdefault(voice["language"], []).append(voice["name"])
target = os.path.join(os.path.dirname(os.path.abspath(__file__)), "web", "voices.json")
with open(target, "w", encoding="utf-8") as f:
    json.dump(by_language, f, ensure_ascii=False)
print(f"wrote {target}: " + ", ".join(f"{lang} {len(names)}" for lang, names in sorted(by_language.items())))
