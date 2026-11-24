# One server, one language per session: end-to-end check

A check against a running `ptts-ws-server`, with real sessions and real audio. It is not part of
CI: it needs the multilingual checkpoint, the voice catalog and a Whisper model.

```sh
# 1. The server: the multilingual checkpoint, up to four open sessions.
cargo run --release -p ptts-ws-server -- \
  --model hf://gradium/phonon-40m-multilingual --lang en --max-concurrent-requests 4

# 2. Sessions in several languages at once, then the edge cases of `setup`. Writes WAVs and results.json.
pip install websockets
python scripts/e2e-lang/client.py --out out

# 3. What was said in each session.
pip install faster-whisper
python scripts/e2e-lang/transcribe.py --out out
```

`client.py` opens four sessions together (pt, en, fr, de), then es, each with a catalog voice of
its own language, and the cases an invalid `lang` (400), a language the checkpoint does not list
(`ja`, a warning in the server log), an empty `lang`, `none`, an unknown voice (404) and no `lang`
(the server's `--lang`).

Expected reading, which `transcribe.py` prints: `R$ 10,50` and `Dra.` come back as "reais" and
"doutora" in the `pt` session, `$10.50` and `Dr.` as English words in the `en` one, `10,50 €` as
"Euro" or "euros" in `de`, `fr` and `es`, with every session reading its own text at the same time.

## Listening in a browser

```sh
python scripts/e2e-lang/voices_json.py
python -m http.server 8000 --directory scripts/e2e-lang/web
```

Open <http://127.0.0.1:8000/>. The page connects to `ws://<host>:8080/speech/tts` (change it with
`?ws=ws://host:port/speech/tts`). Pick a language and a voice and press Speak, or press
"pt and en at the same time" to open two sessions on the one server.
