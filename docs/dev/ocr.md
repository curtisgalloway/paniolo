<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# OCR: the helper protocol

paniolo reads the target's screen with OCR (optical character recognition:
turning an image of text into text). It pipes a captured frame to an **OCR
helper binary** and parses what comes back. This page is the contract a helper
implements, and which helper runs where.

The helper is a separate process, not a library, because the best engine on
each platform is written in a different language: Swift against Apple Vision,
Rust against Windows' `Windows.Media.Ocr`, Python around Tesseract (an
open-source OCR engine). A process boundary lets those coexist.

## Which engine runs where

Defaults are **platform-native**, chosen for accuracy and latency on the host
paniolo is running on:

| Platform | Helper | Engine | Installed by |
| --- | --- | --- | --- |
| macOS | `visionocr` | Apple Vision (`VNRecognizeTextRequest`) | `paniolo setup` compiles `ocr/visionocr.swift` with `swiftc` |
| Windows | `winocr` | `Windows.Media.Ocr` (in-box, offline) | `paniolo setup` builds `ocr/winocr`; the release zip ships it in `libexec` |
| Linux (console/firmware screens) | `linuxocr` | Tesseract 5 | `paniolo setup` copies `ocr/linuxocr`; needs `tesseract-ocr` |
| Linux (GUI screens) | `rapidocr` | PP-OCRv6 via RapidOCR on ONNX Runtime | `paniolo setup` copies `ocr/rapidocr` and, when a lab file asks for it, builds its venv |

`$PANIOLO_VISIONOCR` overrides the choice with an explicit path.

The same screen therefore OCRs differently depending on which control host owns
the video channel, so an agent's behavior can be host-dependent. Two things
make that tractable: every result names the engine that produced it (`engine`,
`engine_detail`), and the override above lets a lab force one engine across
hosts when comparing runs.

### Linux needs two engines; the other platforms need one

`ocr_mode` on the target's `video` channel selects the Linux engine: `"text"`
(the default) or `"gui"`. Set it with `paniolo video set -t <target> --ocr-mode
gui` (or `text`, the default) — see [video.md](../video.md#ocr). It is
configured because the choice cannot be inferred at runtime (see the confidence
table under [The contract](#the-contract)).

Measured on a Pi 5 control host against `evals/ocr`:

| screen type | rapidocr | linuxocr (Tesseract) |
| --- | --- | --- |
| GUI | **0.083** token-recall error, ~4.2 s | 0.312, ~1.7 s |
| text | 0.025 CER, ~6.5 s | **0.019**, ~2.0 s |

Neither engine is simply better. rapidocr (which runs the PP-OCRv6 models on
ONNX Runtime, a portable ML inference library) is ~4x more accurate on
anti-aliased UI text, slightly worse on console text, and 2-3x slower. The
trade is worth it on GUI screens for three reasons, which are the ones to
revisit if this is reopened:

1. **Latency is affordable because OCR is not in a polling loop.** Change
   detection uses the frame hash (`/status`, `--changed-since`). OCR runs only
   when something asks what the screen says, usually once after a state change,
   against a boot that took tens of seconds.
2. **Tesseract fails on GUIs by silently omitting text.** On a BIOS boot-order
   page it returned the headings and dropped every value (see
   [What the engines actually do](#what-the-engines-actually-do)). An agent
   asking "what is the boot order?" gets a confident, complete-looking, wrong
   answer — worse than a slow correct one.
3. **That rules out the obvious hybrid.** "Run Tesseract, fall back on low
   confidence" cannot work: Tesseract is confident about the rows it *did*
   read, so the missing ones raise no signal to fall back on.

**No preprocessing for rapidocr.** The benchmark swept upscaling, inversion and
binarization: every variant was slower than `raw` and none was more accurate.
So rapidocr gets the frame untouched, unlike `visionocr` and `linuxocr`, which
upscale internally.

**The venv is opt-in.** It is ~317 MB (onnxruntime 58 MB, models 31 MB,
numpy/opencv the rest), and `paniolo setup` builds it only when some target
sets `ocr_mode = "gui"`. It is a venv rather than a system install because Pi
OS is PEP 668-managed (the system Python refuses `pip install`).
`opencv-python-headless` is forced over the `opencv-python` that RapidOCR pulls
in: the full build needs `libGL.so.1`, which headless Pi OS lacks, and it fails
at first OCR rather than at install.

## The contract

**Input:** a PNG on stdin (`-`) or a path argument.

- `rapidocr` enforces this literally and errors on anything without the PNG
  signature; see [Resource limits](#resource-limits) for why.
- The others accept whatever their platform's image loader recognizes, which is
  more than PNG. Nothing in paniolo sends them anything else, so do not rely on
  it.

**Output:** plain text by default, one line per recognized line, in reading
order. That form is for humans running the helper by hand.

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
- `engine` / `engine_detail` — what produced the result. With platform-native
  defaults, these are how you tell why two hosts disagree about the same
  screen.
- `width` / `height` — **the source image's** dimensions, in pixels.
- `text` — every line joined with `\n`, in reading order, so consumers that
  only want text do not have to reassemble it.
- `lines[].confidence` — `0.0`–`1.0`, and **optional**.
  - A helper normalizes an engine's other scale: Tesseract's `0`–`100` is
    divided by 100, and its `-1` ("no text") means the line is omitted, never
    reported as `0.0`.
  - An engine with no confidence to report omits the field. A consumer must not
    read its absence as zero.

  Confidence is the least dependable part of this contract, so do not design
  around it:

  | Engine | Confidence |
  | --- | --- |
  | Tesseract | Real per-word values |
  | Apple Vision | **Constant** — 0.5 for every line in `--fast`, 1.0 in `--accurate`. Measured over a 56-line frame: one distinct value. It indicates the recognition level, not quality. |
  | `Windows.Media.Ocr` | **Not exposed at all** — the field is absent |

  Routing between engines or recognition levels is therefore configured, not
  inferred from confidence: on two of three platforms there is nothing to infer
  from.
- `lines[].bbox` — `[x, y, w, h]` in **pixels, origin top-left, in source-image
  coordinates**. It is always the intersection of the recognized box with the
  source frame: a line whose recognition rectangle crosses an edge is reported
  at the size it actually occupies inside `[0, width) x [0, height)`, not the
  size it had before clipping.

### The bbox rule is the sharp edge

**Every helper must map its boxes back to the source frame.** Helpers
preprocess before recognizing, so engine coordinates refer to a *different
image* than the caller supplied. `visionocr`, for example, upscales 2× and pads
16 px, because small console fonts recognize far better enlarged and glyphs
flush to the frame edge get clipped.

A consumer wants a bbox to act on it: crop it, or click it through the hid
channel. A box in an intermediate buffer's coordinates aims slightly wrong,
which is easy to miss precisely because it is close.

Each helper undoes its own conventions:

- Apple Vision reports **normalized, bottom-left-origin** boxes against the
  upscaled, padded buffer. `visionocr` flips the y axis as well as undoing its
  scale and padding.
- Tesseract reports pixels on that same preprocessed image.
- `Windows.Media.Ocr` reports pixels on the source.

**Mapping must clip, not clamp.** A helper maps *both* corners of a box into
source coordinates, intersects the result with `[0, width] x [0, height]`, and
derives width and height from the clipped corners. A crossing on any edge then
shrinks the reported box, and a box entirely outside the source (only possible
for garbage input) clips to zero size rather than a negative one.

Two real defects show why:

- `visionocr --json` once reported coordinates normalized against its padded,
  upscaled buffer. Scaling those back by the source dimensions left a
  systematic error from the never-removed padding: a few pixels at 800x600, and
  proportionally worse the larger the padding relative to the frame. Enough to
  clip a tight crop, close enough to look plausible. The units were wrong too:
  a consumer expecting pixels would have read `0.137` where the answer was
  `104`. It went unnoticed because nothing consumed `--json` yet.
- A follow-up fix got the origin right but not the extent: both helpers clamped
  a mapped corner to 0 at the left/top edge without shrinking the width/height
  measured in the padding. A line flush with the frame's left edge came back a
  few pixels too wide instead of clipped.

## What paniolo does with it

`hdmicap`'s `GET /ocr` runs the helper with `--json` and returns the envelope as
`application/json`. `paniolo video read` prints `text` by default (what a human
or an agent grepping the screen wants, and what the command printed before the
envelope existed) and the whole envelope under `--json`.

**Version skew degrades rather than fails, in both directions.** The helper,
the daemon and the CLI are installed separately and upgrade at different times;
without this, a version mismatch looks to an agent like a broken capture.

- If a helper's stdout does not parse as an envelope, `/ocr` treats it as plain
  text from a pre-v1 helper. It synthesizes an envelope with no `lines` and
  logs a warning naming the binary. Omitting boxes is accurate; fabricating
  them is not.
- `paniolo video read` passes a non-envelope body through unchanged, so a CLI
  newer than its daemon still reads screens.

The envelope check is `version` being present, not merely "the body is JSON".
Otherwise a screen that happens to *show* JSON containing a `text` key would be
mined for it.

### Coordinates cross-check

The three helpers start from three different native conventions (see
[the bbox rule](#the-bbox-rule-is-the-sharp-edge)). On the same frame's first
line they converge:

| Helper | bbox |
| --- | --- |
| `visionocr` | `[0, 33, 396, 21]` |
| `linuxocr` | `[2, 34, 390, 16]` |
| `winocr` | `[2, 34, 391, 17]` |

Agreement within a few pixels across three independent implementations checks
that the mapping-back rule is actually applied, rather than each helper
reporting something self-consistent and wrong. Re-run it when a helper's
preprocessing changes.

## What the engines actually do

Measured on the 13 dongle captures in `evals/ocr/dataset`, same bytes to each.
The hardest frame is an AMI BIOS page whose boot-order values sit inside
cyan-filled dropdown widgets:

| Engine | Boot Option values |
| --- | --- |
| Apple Vision `--accurate` | `UEFI: PXE IPv4 Intel(R) Ethernet C` — exact |
| `Windows.Media.Ocr` | `UEFI: PXE IPva Intel(R) Ethernet C` — reads them, fumbles digits |
| Tesseract | **None.** The widget text does not survive at all |

Tesseract's failure is the dangerous one: well-formed text with whole rows
missing. Vision and winocr garble visibly.

Digit/letter confusion is the common weakness, and it lands hardest on exactly
the strings bring-up cares about. On a PXE (network boot) screen's MAC address:

| Engine | Result |
| --- | --- |
| Apple Vision `--accurate` | `54-B2-03-F0-B5-5C` — exact |
| `Windows.Media.Ocr` | `S4-B2-03-FO-BS-SC` — 5→S, 0→O |
| Apple Vision `--fast` | `54-B2-03-FO-B5-5C` — one 0→O |

So match such strings loosely, or corroborate them, rather than trusting an
exact compare.

## Resource limits

A helper gets whatever bytes and resolution the target's video channel
produces, which paniolo does not bound. Every helper enforces the same three
limits before the expensive work (a full image decode, or the 2x upscale
`linuxocr`/`visionocr` do for small console fonts), so a hostile or merely
oversized input fails fast instead of driving an oversized allocation.

**64 MiB of encoded input.** Each helper reads stdin or the file argument in
bounded chunks, up to the limit plus one byte, so "exactly at the limit" and
"over it" are both detectable without buffering much past the cap. It errors as
soon as it sees more than the limit.

**8192 px on a side, 33,177,600 px total** (7680x4320).

- **Before decode**, each helper checks the size the image header declares,
  without rasterizing pixels:
  - `linuxocr`/`rapidocr` parse the PNG IHDR (header chunk) by hand.
  - `visionocr` reads ImageIO's properties (`CGImageSourceCopyPropertiesAtIndex`).
  - `winocr` reads `BitmapDecoder.PixelWidth`/`PixelHeight` before calling
    `GetSoftwareBitmapAsync()`.
- **After decode**, each helper checks again, unconditionally, before using the
  result. This backstops whatever the header parse missed (a format the
  PNG-specific check doesn't recognize, or Pillow/OpenCV succeeding where it
  didn't). The decode has already happened, but for `linuxocr`/`visionocr` the
  expensive step, the 2x upscale, has not.
- 33,177,600 is exactly 2x a 4K capture (3840x2160) in each dimension, so any
  4K frame passes: with margin on the per-side number, none on the pixel count,
  since a doubled 4K frame is precisely at that limit.
- `linuxocr` and `visionocr` check the pair twice: once against the source
  size, once against the *working* size their 2x upscale is about to allocate.
  The small fixed padding on top (a constant ~20-32 px, negligible here) is not
  part of the limit.
- `rapidocr` and `winocr` do no upscaling, so the limit applies directly to the
  source. `winocr` also intersects the per-side limit with
  `OcrEngine::MaxImageDimension()`, using whichever is stricter.

**`rapidocr` refuses input that is not a PNG**, before decoding anything. Its
pre-decode check reads the PNG IHDR, but `cv2.imdecode` also accepts JPEG, BMP
and WebP. A JPEG under the byte cap whose SOF (the JPEG size header) declared
an enormous size used to reach the decoder in full, leaving only the
post-decode backstop, which runs after the allocation it exists to prevent
(#167). Refusing non-PNG, rather than teaching the helper a second header
format, matches the input contract above and what hdmicap actually sends (its
own PNG encoder's output).

The three numbers must stay **identical** across `ocr/linuxocr`,
`ocr/rapidocr`, `ocr/visionocr.swift` and `ocr/winocr/src/main.rs`. Each
defines them once as named constants, with a comment pointing at the other
three so a change to one doesn't silently drift from the rest.

**Errors are one line, on stderr, non-zero exit** — the same path each helper
already uses for "cannot read", "could not decode image", and so on, not a new
failure mode a caller has to learn to recognize.

## Every failure leaves the same way

Not only the limits: a missing input file, bytes that are not an image, an I/O
error part-way through a read all go through the helper's own error exit,
because the caller is a daemon parsing stderr, not a person reading a stack
trace. These paths once escaped it and now go through the error exit:

| Helper | Was | Now |
| --- | --- | --- |
| `linuxocr` | Opened its input file bare: a missing path gave a `FileNotFoundError` traceback. Preprocessing caught only `ImportError` (a missing Pillow), so non-image bytes gave a `PIL.UnidentifiedImageError` traceback. | Both go through `die()`. |
| `rapidocr` | Opened its input file bare, with the same result. | Goes through the error exit. |
| `visionocr` | Read with `try?`, which turns a failed read into `nil`, the same value as EOF. A descriptor error went unreported: the helper OCR'd whatever prefix it had read and returned that text. This was the worst of them, because nothing downstream can tell a truncated screen from a short one. | Its bounded reader `rethrows`, and the caller dies with the underlying error. |
| `linuxocr` | Spawned `tesseract` bare. A host without the binary (an ordinary first-run state, since it is a system package the `.deb` only Recommends) got a `FileNotFoundError` traceback, and a non-zero tesseract exit had its raw multi-line stderr forwarded verbatim. | Both go through `die()`: the missing-binary message names the apt package that provides it, and a non-zero exit collapses to one line that keeps tesseract's own message. |

CI runs the Python helpers' error-path tests (`ocr/tests`) and `visionocr`'s
`--self-test`. A new helper hooks in as described below.

## Adding an engine

1. Read a PNG from stdin or a path; support `--json`.
2. Emit the envelope above, with boxes mapped back to source coordinates.
3. Name it in `daemons::helper_dirs()`'s install path and in `paniolo setup`.

If the engine cannot report confidence, report the absence rather than invent a
number. Confidence is what makes the interesting things possible: routing a
screen to a cheap engine and falling back when it reports low confidence, or
keying downstream matching on high-confidence tokens rather than raw string
equality.
