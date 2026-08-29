<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

# OCR benchmark

Scores paniolo's OCR helpers against real dongle captures, so the production
config per screen type is chosen from numbers rather than impressions.

```sh
uv sync
uv run python -m bench.runner                        # everything
uv run python -m bench.runner --limit 2 --repeats 1  # smoke test
uv run python -m bench.runner --engines visionocr,linuxocr --variants raw,luma_up2
```

Results land in `results/` (gitignored): `raw.csv`, one row per
(image × engine × variant), and `summary.md`, which is meant to be sufficient on
its own to pick a config.

## The adapter is paniolo's OCR envelope

Engines are driven through the **v1 envelope** (`docs/ocr.md`) rather than a
Python protocol invented here. That is a departure from the plan this harness
was specced from, and the reason is that the envelope arrived in between: it
already carries text, per-line confidence, boxes and engine identity in one
shape, and it is what paniolo actually runs. Scoring Python library calls would
measure something adjacent to what ships.

So `HelperEngine` runs an installed helper (`visionocr`, `winocr`, `linuxocr`)
exactly as hdmicap does — `--json`, PNG on stdin. A candidate paniolo does not
ship yet, like RapidOCR/PP-OCRv5, implements the same envelope in Python
(`RapidOcrEngine`). A candidate that wins can then be promoted to a helper
binary without changing anything that scores it.

**Put the helpers on `$PATH` before running**, and check `engine_detail` in
`raw.csv` afterwards. A stale binary is the easiest way to get confident, wrong
numbers — the first real run here silently measured an old `visionocr` whose
default was `.fast`, and `engine_detail` is what caught it.

## Ground truth

Beside each image: `<stem>.gt.txt` (full transcription, scored by CER and WER)
or `<stem>.gt.json` with `required_tokens` (scored by recall).

Token recall exists because transcribing a busy GUI frame in full is expensive
and mostly scores things nobody needs — window chrome, a clock, wallpaper
noise. What matters is whether `UEFI: PXE IPv4` and `KINGSTON` came back. The
report lists **which** tokens were missed, because that is the finding: an
engine that reads a BIOS page's headings and drops every boot-order value
scores respectably and cannot do the job.

**Ground truth is transcribed by eye, never from an engine's output.** Deriving
it from OCR would measure agreement with itself.

Coverage is currently **3 of 13 frames**, and two of those three are the
near-identical boot-priority pair — so treat the "recommended config" lines as
provisional. Extending coverage is the highest-value work left here.

## What the numbers say so far

On the frames that have ground truth, the ranking is consistent and matches
what eyeballing suggested:

| Screen type | Best | Error |
| --- | --- | --- |
| GUI | `visionocr` (`.accurate`) | 0.000–0.083 token recall error |
| GUI | `linuxocr` (Tesseract) | 0.208 |
| GUI | `visionocr --fast` | 0.417 |
| text | `visionocr` (`.accurate`) | CER 0.008 |
| text | `linuxocr` | CER 0.016 |
| text | `visionocr --fast` | CER 0.078 |

`.accurate` beats `.fast` by roughly 10x on character error, which is the
measured basis for the default having been changed.

## A question this harness cannot answer

`pipeline_luma_direct` and `pipeline_rgb_roundtrip` exist to test whether
paniolo's capture path costs OCR quality: on macOS and Windows a frame is NV12,
so the Y plane *is* the greyscale image OCR wants, yet `/ocr` converts to RGB,
encodes PNG, and the helper decodes and re-greyscales it.

**The two variants score identically here, and that result is not meaningful.**
The dataset is saved PNGs, so the RGB conversion already happened upstream —
both variants start from the same RGB image and neither has the original luma.
The harness is measuring a round trip applied *on top of* a round trip.

Answering it properly needs frames captured as NV12 and as PNG from the *same*
device frame, which is a change to the capture path, not to this harness. Until
then, do not read the tie as evidence that the pipeline is harmless.

## Running on a Pi

The engine choice for Linux is the reason this exists, and a Pi 4B under
sustained neural OCR thermally throttles — which looks exactly like a slow
engine. The runner records the device-tree model, CPU governor and throttle
state, samples temperature around each engine's block, and flags any run where
the throttle bits changed. Prefer the `performance` governor, run nothing else,
and copy the dataset to local storage first: image I/O jitter off a network
mount lands directly in the latency column.
