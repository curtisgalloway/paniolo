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

# OCR: the helper protocol

paniolo reads the target's screen by piping a captured frame to an **OCR helper
binary** and parsing what comes back. The helper is a separate process, not a
library, because the best engine on each platform is written in a different
language — Swift against Apple Vision, Rust against Windows' `Windows.Media.Ocr`,
Python around Tesseract. A process boundary is what lets those coexist.

## Which engine runs where

Defaults are **platform-native**, chosen for accuracy and latency on the host
paniolo is actually running on:

| Platform | Helper | Engine | Installed by |
| --- | --- | --- | --- |
| macOS | `visionocr` | Apple Vision (`VNRecognizeTextRequest`) | `paniolo setup` compiles `ocr/visionocr.swift` with `swiftc` |
| Windows | `winocr` | `Windows.Media.Ocr` (in-box, offline) | shipped in the release zip's `libexec` |
| Linux | `linuxocr` | Tesseract 5 | `paniolo setup` copies `ocr/linuxocr`; needs `tesseract-ocr` |

`$PANIOLO_VISIONOCR` overrides the choice with an explicit path.

**Consequence worth knowing:** the same screen OCRs differently depending on
which control host owns the video channel, so an agent's behaviour can be
host-dependent. Two things make that tractable rather than mysterious: every
result names the engine that produced it (`engine`, `engine_detail`), and the
override above lets a lab force one engine across hosts when comparing runs.

## The contract

**Input** — a PNG on stdin (`-`) or a path argument. **Output** — plain text by
default, one line per recognized line, in reading order. That form is for humans
running the helper by hand.

**paniolo always passes `--json`**, and that is the machine contract:

```json
{
  "version": 1,
  "engine": "visionocr",
  "engine_detail": "Apple Vision VNRecognizeTextRequest, fast",
  "width": 1920,
  "height": 1080,
  "text": "login:\nPassword:",
  "lines": [
    { "text": "login:", "confidence": 0.97, "bbox": [120, 880, 96, 28] }
  ]
}
```

- `version` — this document's version. Bump on any incompatible change.
- `engine` / `engine_detail` — identity of what produced the result. Not
  decoration: with platform-native defaults these are how you tell why two hosts
  disagree about the same screen.
- `width` / `height` — **the source image's** dimensions, in pixels.
- `text` — every line joined with `\n`, in reading order. Retained so consumers
  that only want text do not have to reassemble it.
- `lines[].confidence` — `0.0`–`1.0`. Engines that report otherwise are
  normalized by their helper: Tesseract's `0`–`100` is divided by 100, and its
  `-1` ("no text") means the line is omitted, never reported as `0.0`.
- `lines[].bbox` — `[x, y, w, h]` in **pixels, origin top-left, in source-image
  coordinates**.

### The bbox rule is the sharp edge

Helpers preprocess before recognizing — `visionocr` upscales 2× and pads 16 px,
because small console fonts recognize far better enlarged and glyphs flush to
the frame edge get clipped. Engine coordinates therefore refer to a *different
image* than the caller supplied.

**Every helper must map its boxes back to the source frame.** A consumer's whole
reason for wanting a bbox is to act on it — crop it, or click it through the hid
channel — and a box in the coordinates of an intermediate buffer aims slightly
wrong, in a way that is easy to miss precisely because it is close.

This was a real defect. `visionocr --json` reported coordinates normalized
against its padded, upscaled buffer rather than the source frame. Scaling those
back by the source dimensions leaves a systematic error from the padding that
was never removed — a few pixels at 800x600, and proportionally worse the larger
the padding is relative to the frame. Enough to clip a tight crop, close enough
to look plausible. It went unnoticed because nothing consumed `--json` yet, and
the units differed too: a consumer expecting pixels would have read `0.137`
where the answer was `104`.

Apple Vision additionally reports **normalized, bottom-left-origin** boxes, so
`visionocr` flips the y axis as well as undoing its own scale and padding.

## What paniolo does with it

`hdmicap`'s `GET /ocr` runs the helper with `--json` and returns the envelope as
`application/json`. `paniolo video read` prints `text` by default and the whole
envelope under `--json`.

**Legacy helpers degrade rather than fail.** If a helper's stdout does not parse
as an envelope, `/ocr` treats it as plain text from a pre-v1 helper, synthesizes
an envelope with no lines and a warning naming the binary. A new CLI against an
old installed helper then still reads screens, just without confidences —
instead of failing in a way that looks like a broken capture.

## Adding an engine

1. Read a PNG from stdin or a path; support `--json`.
2. Emit the envelope above, with boxes mapped back to source coordinates.
3. Name it in `daemons::helper_dirs()`'s install path and in `paniolo setup`.

Confidence is what makes the interesting things possible — routing a screen to
a cheap engine and falling back when it reports low confidence, or keying
downstream matching on high-confidence tokens rather than raw string equality —
so an engine that cannot report it should report the absence rather than invent
a number.
