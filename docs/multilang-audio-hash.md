# Reference audio hashes

Both tests compare the FNV-1a of the audio generated for the fixed text
`We have $10.50 and Dr. Smith, on March 2nd at 3pm.` (default voice, `en`, q8, CPU) with the
value produced by upstream `c81eca4`, so the default path (no `lang` in the request) is proven
to be unchanged by the multilingual work.

| Test | Hash | Hashed data |
|---|---|---|
| `ptts-ws-server`: `the_default_path_still_produces_the_upstream_audio` | `0x12d7e2d3cfa715c1` | PCM s16le, seed 7, server started with `--lang en` |
| `ptts-ffi`: `the_cpu_engine_still_produces_the_upstream_audio` | `0x230f1b8ea3d36faf` | f32 samples as little-endian bytes, `ptts_new` with the `en` language |

## Determinism

- ws-server, `phonon-40m-multilingual`: 27 of 28 runs on this machine gave the hash above (six
  earlier runs, five and twelve later ones, plus upstream `c81eca4` once); one run gave
  `0xd5d18773e242262a`, cause not found. A hard failure of the exact comparison is possible.
- ws-server, `phonon-31m-en` (seed 7, temperature 0.3, `--lang en`, q8, CPU, same text): upstream
  `c81eca4` gives `0xa2005e12605cc91a` (10 runs, identical bytes) and this branch gives the same
  hash and the same bytes in 10 runs. Debug test profile, about 13 to 15 s per run after the
  model is cached.
- FFI: not deterministic on the CPU, even upstream. In 8 runs of `c81eca4`, one produced a
  different hash. On this branch, 5 runs of the test needed 1, 2, 3, 5 and 1 attempts to match
  (one run matched only on the last attempt), so 5 attempts is the margin, not a comfortable one.
  The test passes if any of 5 attempts matches the modal upstream hash.

## Regenerating

Create a worktree of `c81eca4`, copy the test body (with upstream's `load_ptts` signature),
run it with
`PTTS_TEST_MODEL=hf://gradium/phonon-40m-multilingual cargo test -p ptts-ws-server -- --ignored --nocapture`
and read the `fnv1a` line. The FFI value comes from the same method with the FFI test and
`PTTS_TEST_MODEL_DIR`. Never regenerate from the `multilang` branch.

The hash depends on the CPU and the q8 kernel version: on another machine, regenerate it from
upstream on that machine.
