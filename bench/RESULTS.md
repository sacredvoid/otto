# Speech model bench results

- Hardware: Windows 11, 20-thread Intel CPU, NVIDIA RTX 4060 Laptop GPU, Intel UHD iGPU.
- Date: 2026-11-04. Clips: 45 recorded by Santosh (40 spoken commands, 5 silence clips). Script text is in `bench/commands.tsv`.
- Method: each model transcribes every clip raw (no VAD, no vocabulary), per device. Latency is warm release-to-text time; entity hit is a normalized match on the key entity; WER is the mean over the 40 spoken clips.

## Results

"Raw hit" comes from the runner. "Adjusted hit" also counts correct readings the normalizer misses: whole-hour answers for time02 and time07, and colon-less digit times for time06 and time08. I computed adjusted scores from the CPU run rows of each model and list them for every device.

| model | device | raw hit % | adjusted hit % | mean WER | p50 ms | p95 ms | silence clips with text | peak MB | model file MB |
|---|---|---|---|---|---|---|---|---|---|
| parakeet-0.6b-v2 Q8_0 | cpu | 67.5 | 72.5 | 0.160 | 421 | 605 | 0 | 1115 | 729.6 |
| parakeet-0.6b-v2 Q8_0 | nvidia | 67.5 | 72.5 | 0.160 | 58 | 75 | 0 | 767 | 729.6 |
| parakeet-0.6b-v2 Q8_0 | intel | 65.0 | 70.0 | 0.185 | 162 | 241 | 0 | 939 | 729.6 |
| parakeet-110m Q8_0 | cpu | 60.0 | 65.0 | 0.239 | 111 | 193 | 0 | 283 | 135.4 |
| parakeet-110m Q8_0 | nvidia | 57.5 | 62.5 | 0.239 | 32 | 40 | 0 | 198 | 135.4 |
| parakeet-110m Q8_0 | intel | 57.5 | 62.5 | 0.242 | 72 | 83 | 0 | 253 | 135.4 |
| base.en (optimized build) | cpu | 52.5 | 62.5 | 0.313 | 1333 | 1648 | 5 | 343 | 148.0 |
| base.en (as shipped, /Od build) | cpu | 50.0 | n/a | 0.322 | 10135 | 11589 | 4 | 343 | 148.0 |

The as-shipped row is latency context only. Its run skipped sil01 and used an older name03 clip, so its hit, WER and silence numbers do not compare with the other rows.

Adjusted hit flips, by model on CPU (each flip adds 2.5 points):

- parakeet-0.6b-v2: time02, time07 (time06 and time08 already hit)
- parakeet-110m: time02, time07 (time08 already hit)
- base.en optimized: time02, time06, time07, time08

## Winner

parakeet-0.6b-v2 Q8_0 wins: it has the highest key-entity hit rate (67.5% raw, 72.5% adjusted) among models with a warm CPU-only p95 at or under 1000 ms (605 ms), and its mean WER of 0.160 is the lowest of all models. Optimized base.en misses the latency bar at 1648 ms p95. Parakeet-110m also passes the bar but hits fewer entities (60.0% raw).

Installer size impact: the winner file is 729.6 MB against 148.0 MB for the base.en file Otto bundles today, an increase of 581.6 MB if it replaces base.en. Parakeet-110m is 135.4 MB, 12.6 MB smaller than base.en. Memory and installer size are reported and do not disqualify a model.

## Windows build finding

Otto's current Windows build runs base.en at p50 10135 ms and p95 11589 ms. The same model from an optimized build runs at p50 1333 ms and p95 1648 ms, about 7.6 times faster at the median. The cause, per a reviewer's build experiment (CMakeCache and compile flags): cmake-rs 0.1.58 with the Visual Studio generator compiles whisper.cpp at /Od (no optimization). Any base.en comparison on Windows should use the optimized build, and the shipped build needs the fix regardless of which model wins. This branch removes whisper-rs from Otto, and transcribe-cpp builds its native code at /O2, so the shipped app no longer has the /Od problem.

## No-GPU check

A copy of the `tc` runner without `ggml-vulkan.dll` ran 10 clips. The runner logged "device 'nvidia' not found, skipping" and transcribed on CPU.

| model | device | raw hit % | mean WER | p50 ms | p95 ms | silence clips with text | peak MB |
|---|---|---|---|---|---|---|---|
| parakeet-0.6b-v2 Q8_0 | cpu | 62.5 | 0.281 | 437 | 622 | 0 | 1072 |
| parakeet-110m Q8_0 | cpu | 50.0 | 0.286 | 96 | 156 | 0 | 240 |

These numbers come from a 10-clip subset, so they do not compare with the 45-clip table.

## Silence

Parakeet produced no text on all 5 silence clips, on every device. Optimized base.en produced text on all 5: "[BLANK_AUDIO]" (twice), "[Music]", "*squeaking*" and "*cough*". Otto's `is_speech()` filter drops text wrapped in [] or (), so it removes the three bracketed outputs. The two asterisk outputs ("*squeaking*", "*cough*") would reach the assistant as input.

## Common misses

Every model misses the same hard items:

- "Samanvya" (name06, mail03): 0.6b gives "Samanvia" and "Samanga", 110m gives "someone here" and "Samanja", base.en gives "someone here" and "someone".
- "Figma" heard as "Sigma" (app03): all three models, and in app07 both 0.6b and base.en.
- "nymbl" in emails (mail01, mail03, mail04): "number dot app" (0.6b), "Numble" and "Nimble" (110m), "number.app" and "nimble.app" (base.en and 0.6b).

## Otto mishearings

Count of the 11 commands that contain "Otto" where the hypothesis does not contain "otto":

| model | misheard or dropped | examples |
|---|---|---|
| parakeet-0.6b-v2 | 3 of 11 | "He ought to" (name05), "Auto" (name09), "Autoccpr" (mail05) |
| parakeet-110m | 11 of 11 | "Auto", "Oto" in every command |
| base.en optimized | 7 of 11 | "Arto", "Hato", "Ottawa", "auto"; Otto dropped in name05 and app03 |

Otto appears in 11 of the 40 commands, so a vocabulary or prompt bias for "Otto" is worth testing in the next stage. This bench ran no vocabulary.

## Caveats

- 40 spoken commands from one speaker on one laptop. Results do not cover other voices, accents or microphones.
- All clips are real recordings of the scripted text, with no synthetic speech.
- Cold start: the table shows warm numbers. The first GPU use after install compiles shaders, and later runs use the cache. A Task 6 smoke run on a fresh driver cache measured 13.4 s for the 0.6B model's first GPU run on the RTX 4060; this run started with a warm cache (first_run_ms.csv: 95 ms first run, 873 ms load). The CPU path has no shader compile (0.6B first run 415 ms, load 388 ms).
- The adjusted score trusts a readable number in the right slot. For base.en time02 ("Who's the standard to 10?") the rule counts a hit although the sentence is wrong. The adjustment rule also counts base.en time08 ("Schedule the renews for 230 tomorrow.") as a hit even though it has a wrong word; without the time02 and time08 adjustments the optimized base.en adjusted score is 57.5.
- Peak memory for the 0.6B model on CPU is 1115 MB, about three times the base.en peak of 343 MB.
