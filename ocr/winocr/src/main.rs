// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! `winocr` — read text from an image using `Windows.Media.Ocr`.
//!
//! The Windows sibling of `visionocr` (Apple Vision) and `linuxocr`
//! (Tesseract), and the reason "platform-native OCR defaults" is a real
//! statement on Windows rather than an aspiration. On-device, offline, no model
//! download: the engine ships with the OS.
//!
//!   winocr [--json] [PATH | -]
//!
//!   --json   emit the v1 OCR envelope (see docs/ocr.md) instead of plain text
//!
//! **No confidence scores.** `Windows.Media.Ocr` does not expose a per-word or
//! per-line confidence — unlike Tesseract, and unlike Apple Vision, which
//! exposes one that turns out to be constant. Per docs/ocr.md an engine that
//! cannot report confidence reports its absence rather than inventing a number,
//! so `confidence` is simply omitted from every line here. A consumer that
//! needs to rank results must not read a missing field as zero.

use std::io::Read;

#[cfg(windows)]
mod win {
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    pub struct Line {
        pub text: String,
        pub bbox: [i32; 4],
    }

    pub struct Recognized {
        pub lines: Vec<Line>,
        pub width: u32,
        pub height: u32,
        pub language: String,
    }

    /// Decode `png` and run the OS OCR engine over it.
    pub fn recognize(png: &[u8]) -> windows::core::Result<Recognized> {
        // WinRT decodes from a random-access stream, so the bytes go through an
        // in-memory one rather than a temp file.
        let stream = InMemoryRandomAccessStream::new()?;
        let writer = DataWriter::CreateDataWriter(&stream)?;
        writer.WriteBytes(png)?;
        writer.StoreAsync()?.join()?;
        writer.FlushAsync()?.join()?;
        writer.DetachStream()?;
        stream.Seek(0)?;

        let decoder = BitmapDecoder::CreateAsync(&stream)?.join()?;
        let bitmap = decoder.GetSoftwareBitmapAsync()?.join()?;
        let width = decoder.PixelWidth()?;
        let height = decoder.PixelHeight()?;

        // Prefer the user's own languages; fall back to English, which is what
        // every screen paniolo reads is in. A machine with no OCR language pack
        // installed yields neither, and the caller gets a clear error rather
        // than empty text that looks like a blank screen.
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()
            .ok()
            .or_else(|| {
                Language::CreateLanguage(&windows::core::HSTRING::from("en-US"))
                    .ok()
                    .and_then(|l| OcrEngine::TryCreateFromLanguage(&l).ok())
            });
        let Some(engine) = engine else {
            return Err(windows::core::Error::new(
                windows::core::HRESULT(-1),
                "no OCR language pack available (Settings > Language > \
                 add the Optional feature \"Optical character recognition\")",
            ));
        };
        let language = engine
            .RecognizerLanguage()
            .and_then(|l| l.LanguageTag())
            .map(|t| t.to_string_lossy())
            .unwrap_or_else(|_| "unknown".to_string());

        // Windows OCR refuses images past a per-engine limit. The limit is
        // generous — a 3840x2160 capture passes — so this is a safety net
        // rather than a path normal captures hit. It exists because the failure
        // without it is opaque, and because paniolo hands this whatever
        // resolution the panel is running at, which is not bounded by anything
        // paniolo controls.
        if let Ok(max) = OcrEngine::MaxImageDimension() {
            if width > max || height > max {
                return Err(windows::core::Error::new(
                    windows::core::HRESULT(-1),
                    format!(
                        "image is {width}x{height}; Windows.Media.Ocr accepts at most \
                         {max} on a side. Downscale before OCR."
                    ),
                ));
            }
        }

        let result = engine.RecognizeAsync(&bitmap)?.join()?;
        let mut lines = Vec::new();
        for line in result.Lines()? {
            let text = line.Text()?.to_string_lossy();
            // A line has no rect of its own; take the union of its words'.
            let (mut x0, mut y0) = (f64::MAX, f64::MAX);
            let (mut x1, mut y1) = (f64::MIN, f64::MIN);
            let mut any = false;
            for word in line.Words()? {
                let r = word.BoundingRect()?;
                x0 = x0.min(r.X as f64);
                y0 = y0.min(r.Y as f64);
                x1 = x1.max((r.X + r.Width) as f64);
                y1 = y1.max((r.Y + r.Height) as f64);
                any = true;
            }
            let bbox = if any {
                [
                    x0.round() as i32,
                    y0.round() as i32,
                    (x1 - x0).round() as i32,
                    (y1 - y0).round() as i32,
                ]
            } else {
                [0, 0, 0, 0]
            };
            lines.push(Line { text, bbox });
        }
        Ok(Recognized {
            lines,
            width,
            height,
            language,
        })
    }
}

fn die(msg: &str) -> ! {
    eprintln!("winocr: {msg}");
    std::process::exit(1);
}

fn main() {
    let mut json = false;
    let mut path: Option<String> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--json" => json = true,
            "-" => path = None,
            other => path = Some(other.to_string()),
        }
    }

    let data = match &path {
        Some(p) => std::fs::read(p).unwrap_or_else(|e| die(&format!("cannot read {p}: {e}"))),
        None => {
            let mut buf = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buf)
                .unwrap_or_else(|e| die(&format!("reading stdin: {e}")));
            buf
        }
    };
    if data.is_empty() {
        die("no image data");
    }

    #[cfg(not(windows))]
    {
        let _ = (json, data);
        die("winocr is Windows-only (macOS uses visionocr, Linux uses linuxocr)");
    }

    #[cfg(windows)]
    {
        let r = win::recognize(&data).unwrap_or_else(|e| die(&format!("{e}")));
        let text: Vec<&str> = r.lines.iter().map(|l| l.text.as_str()).collect();
        if !json {
            for t in &text {
                println!("{t}");
            }
            return;
        }
        // Note the absent `confidence`: see the module docs.
        let lines: Vec<serde_json::Value> = r
            .lines
            .iter()
            .map(|l| serde_json::json!({ "text": l.text, "bbox": l.bbox }))
            .collect();
        let envelope = serde_json::json!({
            "version": 1,
            "engine": "winocr",
            "engine_detail": format!("Windows.Media.Ocr, {}", r.language),
            "width": r.width,
            "height": r.height,
            "text": text.join("\n"),
            "lines": lines,
        });
        println!("{}", serde_json::to_string_pretty(&envelope).unwrap());
    }
}
