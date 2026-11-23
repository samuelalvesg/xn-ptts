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

- ws-server: deterministic. Six consecutive runs of the test on this branch (one plus five
  repeats, same machine) gave the same hash every time, so the test has no tolerance.
- FFI: not deterministic on the CPU, even upstream. In 8 runs of `c81eca4`, one produced a
  different hash. The test passes if any of 5 attempts matches the modal upstream hash.

## Regenerating

Create a worktree of `c81eca4`, copy the test body (with upstream's `load_ptts` signature),
run it with
`PTTS_TEST_MODEL=hf://gradium/phonon-40m-multilingual cargo test -p ptts-ws-server -- --ignored --nocapture`
and read the `fnv1a` line. The FFI value comes from the same method with the FFI test and
`PTTS_TEST_MODEL_DIR`. Never regenerate from the `multilang` branch.

The hash depends on the CPU and the q8 kernel version: on another machine, regenerate it from
upstream on that machine.
