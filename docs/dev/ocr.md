<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
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
| Windows | `winocr` | `Windows.Media.Ocr` (in-box, offline) | `paniolo setup` builds `ocr/winocr`; the release zip ships it in `libexec` |
| Linux (console/firmware screens) | `linuxocr` | Tesseract 5 | `paniolo setup` copies `ocr/linuxocr`; needs `tesseract-ocr` |
| Linux (GUI screens) | `rapidocr` | PP-OCRv6 via RapidOCR on ONNX Runtime | `paniolo setup` copies `ocr/rapidocr` and, when a lab file asks for it, builds its venv |

`$PANIOLO_VISIONOCR` overrides the choice with an explicit path.

**Consequence worth knowing:** the same screen OCRs differently depending on
which control host owns the video channel, so an agent's behaviour can be
host-dependent. Two things make that tractable rather than mysterious: every
result names the engine that produced it (`engine`, `engine_detail`), and the
override above lets a lab force one engine across hosts when comparing runs.

### Linux needs two engines; the other platforms need one

Selected by `ocr_mode` on the target's `video` channel — `"text"` (the default)
or `"gui"`. Measured on a Pi 5 control host against `evals/ocr`:

| screen type | rapidocr | linuxocr (Tesseract) |
| --- | --- | --- |
| GUI | **0.083** token-recall error, ~4.2 s | 0.312, ~1.7 s |
| text | 0.025 CER, ~6.5 s | **0.019**, ~2.0 s |

So it is a genuine split rather than a better engine: ~4x more accurate on
anti-aliased UI text, slightly worse on console text, at 2-3x the latency.

Three things make that trade worth taking on GUI screens, and they are the
reasoning to revisit if this is ever reopened:

1. **Latency is affordable because OCR is not in a polling loop.** Change
   detection uses the frame hash (`/status`, `--changed-since`); OCR runs only
   when something asks what the screen says, usually once after a state change,
   against a boot that took tens of seconds.
2. **Tesseract's GUI failure is silent omission.** On a BIOS boot-order page it
   returned the headings and dropped every value. An agent asking "what is the
   boot order?" gets a confident, complete-looking, wrong answer — worse than a
   slow correct one.
3. **That also rules out the obvious hybrid.** "Run Tesseract, fall back on low
   confidence" cannot work: Tesseract is confident about the rows it *did* read,
   so the missing ones raise no signal to fall back on.

`ocr_mode` exists because the choice cannot be inferred at runtime — see the
confidence table below. Set it with `paniolo video set -t <target> --ocr-mode
gui` (or `text`, the default) — see [video.md](../video.md#ocr).

**No preprocessing for rapidocr.** The benchmark swept upscaling, inversion and
binarisation: every variant was slower than `raw` and none more accurate. Unlike
`visionocr` and `linuxocr`, which upscale internally, it gets the frame
untouched.

**The venv is opt-in.** ~317 MB (onnxruntime 58 MB, models 31 MB, numpy/opencv
the rest), built by `paniolo setup` only when some target sets
`ocr_mode = "gui"`. Pi OS is PEP 668-managed, so a venv rather than a system
install. `opencv-python-headless` is forced over the `opencv-python` RapidOCR
pulls in — the full build needs `libGL.so.1`, absent on a headless Pi OS, and it
fails at first OCR rather than at install.

## The contract

**Input** — a PNG on stdin (`-`) or a path argument. **Output** — plain text by
default, one line per recognized line, in reading order. That form is for humans
running the helper by hand.

`rapidocr` holds callers to that literally and errors on anything without the
PNG signature; see "Resource limits" for why. The others accept whatever their
platform's image loader recognizes, which is more than a PNG — but nothing in
paniolo sends them anything else, so do not rely on it.

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
- `lines[].confidence` — `0.0`–`1.0`, and **optional**. Engines that report on
  another scale are normalized by their helper: Tesseract's `0`–`100` is divided
  by 100, and its `-1` ("no text") means the line is omitted, never reported as
  `0.0`. An engine with no confidence to report omits the field; a consumer must
  not read its absence as zero.

  Confidence turned out to be the least dependable part of this contract, so do
  not design around it:

  | Engine | Confidence |
  | --- | --- |
  | Tesseract | Real per-word values |
  | Apple Vision | **Constant** — 0.5 for every line in `--fast`, 1.0 in `--accurate`. Measured over a 56-line frame: one distinct value. It indicates the recognition level, not quality. |
  | `Windows.Media.Ocr` | **Not exposed at all** — the field is absent |

  That is why routing between engines or recognition levels is configured rather
  than inferred from confidence: on two of three platforms there is nothing to
  infer from.
- `lines[].bbox` — `[x, y, w, h]` in **pixels, origin top-left, in source-image
  coordinates**. Always the intersection of the recognized box with the source
  frame: a line whose recognition rectangle crosses an edge is reported at the
  size it actually occupies inside `[0, width) x [0, height)`, not the size it
  had before clipping.

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

A follow-up defect (#149) got the origin right but not the extent: both
helpers clamped a mapped corner to 0 at the left/top edge without also
shrinking the width/height that had been measured in the padding, so a line
flush with the frame's left edge came back a few pixels too wide instead of
clipped. The fix maps *both* corners of a box into source coordinates,
intersects the result with `[0, width] x [0, height]`, and derives width and
height from the clipped corners — so a crossing on any edge shrinks the
reported box, and a box landing entirely outside the source (only possible for
garbage input) clips to zero size rather than a negative one.

## What paniolo does with it

`hdmicap`'s `GET /ocr` runs the helper with `--json` and returns the envelope as
`application/json`. `paniolo video read` prints `text` by default — what a human
or an agent grepping the screen wants, and what the command printed before the
envelope existed — and the whole envelope under `--json`.

**Version skew degrades rather than fails, in both directions.** If a helper's
stdout does not parse as an envelope, `/ocr` treats it as plain text from a
pre-v1 helper and synthesizes an envelope with no `lines`, logging a warning
that names the binary — omitting boxes is honest, fabricating them is not. And
`paniolo video read` passes a non-envelope body through unchanged, so a CLI
newer than the daemon it is talking to still reads screens.

Both paths matter because the helper, the daemon and the CLI are installed
separately and upgrade at different times. The failure they replace looks to an
agent like a broken capture rather than a version mismatch.

The envelope check is `version` being present, not merely "the body is JSON" —
otherwise a screen that happens to *show* JSON containing a `text` key would be
mined for it.

### Coordinates cross-check

The three helpers arrive at boxes from three different native conventions —
Apple Vision reports normalized, bottom-left-origin boxes against an upscaled
and padded buffer; Tesseract reports pixels on that same preprocessed image;
`Windows.Media.Ocr` reports pixels on the source. On the same frame's first
line they converge:

| Helper | bbox |
| --- | --- |
| `visionocr` | `[0, 33, 396, 21]` |
| `linuxocr` | `[2, 34, 390, 16]` |
| `winocr` | `[2, 34, 391, 17]` |

Agreement within a few pixels across three independent implementations is the
check that the mapping-back rule is actually being applied, rather than each
helper reporting something self-consistent and wrong. Worth re-running when a
helper's preprocessing changes.

## What the engines actually do

Measured on the 13 dongle captures in `evals/ocr/dataset`, same bytes to each.
The hardest frame is an AMI BIOS page whose boot-order values sit inside
cyan-filled dropdown widgets:

| Engine | Boot Option values |
| --- | --- |
| Apple Vision `--accurate` | `UEFI: PXE IPv4 Intel(R) Ethernet C` — exact |
| `Windows.Media.Ocr` | `UEFI: PXE IPva Intel(R) Ethernet C` — reads them, fumbles digits |
| Tesseract | **None.** The widget text does not survive at all |

Tesseract's failure there is the dangerous one: it returns well-formed text with
whole rows missing, so an agent asking "what is the boot order?" gets a
confident, complete-looking, wrong answer. Vision and winocr garble visibly.

Digit/letter confusion is the common weakness, and it lands hardest on exactly
the strings bring-up cares about. On a PXE screen's MAC address:

| Engine | Result |
| --- | --- |
| Apple Vision `--accurate` | `54-B2-03-F0-B5-5C` — exact |
| `Windows.Media.Ocr` | `S4-B2-03-FO-BS-SC` — 5→S, 0→O |
| Apple Vision `--fast` | `54-B2-03-FO-B5-5C` — one 0→O |

So match on such strings loosely, or corroborate them, rather than trusting an
exact compare.

## Resource limits

Every helper is handed whatever bytes and resolution the target's video
channel produces, which is not bounded by anything paniolo controls. Each
enforces the same three limits, checked before the expensive work (a full
image decode, or the 2x upscale `linuxocr`/`visionocr` do for small console
fonts) happens, so a hostile or merely oversized input fails fast instead of
driving an oversized allocation:

- **64 MiB of encoded input.** Each helper reads stdin/the file argument in
  bounded chunks (up to the limit plus one byte, so "exactly at the limit" and
  "over it" are both detectable without ever buffering much past the cap) and
  errors as soon as it sees more than that arrives.
- **8192 px on a side, 33,177,600 px total** (7680x4320). Checked against the
  image header *before* a full decode: `linuxocr`/`rapidocr` parse the PNG
  IHDR by hand, `visionocr` reads ImageIO's properties
  (`CGImageSourceCopyPropertiesAtIndex`), and `winocr` reads
  `BitmapDecoder.PixelWidth`/`PixelHeight` before calling
  `GetSoftwareBitmapAsync()` — all of which report a header-declared size
  without rasterizing pixels. Each helper also checks again right after its
  own decode, unconditionally, before doing anything with the result — a
  backstop for whatever the header parse missed (a format the PNG-specific
  check doesn't recognize, or Pillow/OpenCV succeeding where it didn't). The
  decode has already happened by then, but the expensive step for
  `linuxocr`/`visionocr` — the 2x upscale — has not.

  `rapidocr` also **refuses input that is not a PNG**, before it decodes
  anything. Its pre-decode check reads the PNG IHDR, but `cv2.imdecode`
  accepts JPEG, BMP and WebP too, so a JPEG under the byte cap whose SOF
  declared an enormous size used to reach the decoder in full with only the
  post-decode backstop — which runs after the allocation it exists to prevent
  — left to catch it (#167). Teaching the helper a second header format would
  fix the symptom; refusing non-PNG is what the input contract above already
  promises, and what hdmicap actually sends (its own PNG encoder's output).

  33,177,600 is exactly 2x a 4K capture (3840x2160) in each dimension, so any
  4K frame passes — with margin on the per-side number, none on the
  pixel-count one, since a 4K frame doubled is precisely at that limit.
  `linuxocr` and `visionocr` check this pixel/dimension pair twice: once
  against the source size, once against the *working* size their own 2x
  upscale is about to allocate (before the small fixed padding on top, which
  isn't part of the limit — a constant ~20-32 px, negligible against this
  budget). `rapidocr` and `winocr` do no upscaling, so it applies directly to
  the source. `winocr` additionally intersects the shared per-side limit with
  `OcrEngine::MaxImageDimension()`, using whichever is stricter.

The three numbers must stay **identical** across `ocr/linuxocr`,
`ocr/rapidocr`, `ocr/visionocr.swift` and `ocr/winocr/src/main.rs` — each
defines them once as named constants, with a comment pointing at the other
three so a future change to one doesn't silently drift from the rest.

**Errors are one line, on stderr, non-zero exit** — the same path each helper
already uses for "cannot read", "could not decode image", and so on, not a
new failure mode a caller has to learn to recognize.

## Adding an engine

1. Read a PNG from stdin or a path; support `--json`.
2. Emit the envelope above, with boxes mapped back to source coordinates.
3. Name it in `daemons::helper_dirs()`'s install path and in `paniolo setup`.

Confidence is what makes the interesting things possible — routing a screen to
a cheap engine and falling back when it reports low confidence, or keying
downstream matching on high-confidence tokens rather than raw string equality —
so an engine that cannot report it should report the absence rather than invent
a number.
