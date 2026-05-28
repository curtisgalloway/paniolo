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

// visionocr — read text from an image using Apple's Vision framework.
// On-device, no network, no model download. Reads an image (path arg or PNG on
// stdin) and prints recognized text in reading order, one observation per line.
//
//   visionocr [--fast] [--json] [PATH | -]
//
//   --fast   use the fast recognition level (lower latency, less accurate)
//   --json   emit [{text, confidence, x, y, w, h}] with normalized bboxes
//            (origin top-left) instead of plain text lines

import CoreGraphics
import Foundation
import ImageIO
import Vision

func die(_ msg: String) -> Never {
    FileHandle.standardError.write(("visionocr: " + msg + "\n").data(using: .utf8)!)
    exit(1)
}

// Upscale and black-pad an image. Small thin console text recognizes far better
// when enlarged, and padding stops glyphs flush to the frame edge from being
// clipped (which drops the first/last character of a line).
func upscaleAndPad(_ img: CGImage, scale: CGFloat, pad: Int) -> CGImage? {
    let w = Int((CGFloat(img.width) * scale).rounded())
    let h = Int((CGFloat(img.height) * scale).rounded())
    let outW = w + pad * 2
    let outH = h + pad * 2
    guard
        let ctx = CGContext(
            data: nil, width: outW, height: outH, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { return nil }
    ctx.setFillColor(CGColor(red: 0, green: 0, blue: 0, alpha: 1))
    ctx.fill(CGRect(x: 0, y: 0, width: outW, height: outH))
    ctx.interpolationQuality = .high
    ctx.draw(img, in: CGRect(x: pad, y: pad, width: w, height: h))
    return ctx.makeImage()
}

var accurate = false
var json = false
var path: String? = nil
for arg in CommandLine.arguments.dropFirst() {
    switch arg {
    case "--accurate": accurate = true
    case "--fast": accurate = false  // default; accepted for compatibility
    case "--json": json = true
    case "-": path = nil
    default: path = arg
    }
}

let data: Data
if let p = path {
    guard let d = FileManager.default.contents(atPath: p) else { die("cannot read \(p)") }
    data = d
} else {
    data = FileHandle.standardInput.readDataToEndOfFile()
}
if data.isEmpty { die("no image data") }

guard let src = CGImageSourceCreateWithData(data as CFData, nil),
    let decoded = CGImageSourceCreateImageAtIndex(src, 0, nil)
else { die("could not decode image") }

let image = upscaleAndPad(decoded, scale: 2.0, pad: 16) ?? decoded

let request = VNRecognizeTextRequest()
// Counterintuitively, .fast detects small thin console fonts that .accurate
// (tuned for natural document text) misses entirely. Default to .fast; let
// callers opt into .accurate for large, clean text.
request.recognitionLevel = accurate ? .accurate : .fast
// Console/boot/code text is not natural language; correction hurts more than
// it helps (it "fixes" identifiers, hex, paths).
request.usesLanguageCorrection = false
// Vision's default minimumTextHeight (1/32 of image height) skips small console
// fonts. It's a fraction of height; 0.0 means "default", so use a small
// positive floor to catch tiny text.
request.minimumTextHeight = 0.005

let handler = VNImageRequestHandler(cgImage: image, options: [:])
do {
    try handler.perform([request])
} catch {
    die("\(error)")
}

let observations = request.results ?? []

// Vision returns observations unordered. Sort into reading order. boundingBox
// origin is bottom-left, so a larger y is higher on screen.
let sorted = observations.sorted { a, b in
    let dy = a.boundingBox.origin.y - b.boundingBox.origin.y
    if abs(dy) > 0.01 { return dy > 0 }
    return a.boundingBox.origin.x < b.boundingBox.origin.x
}

if json {
    var items: [[String: Any]] = []
    for obs in sorted {
        guard let top = obs.topCandidates(1).first else { continue }
        let b = obs.boundingBox
        items.append([
            "text": top.string,
            "confidence": top.confidence,
            "x": b.origin.x,
            // Convert to top-left origin for consumers that expect it.
            "y": 1.0 - b.origin.y - b.size.height,
            "w": b.size.width,
            "h": b.size.height,
        ])
    }
    let out = try JSONSerialization.data(withJSONObject: items, options: [.prettyPrinted])
    FileHandle.standardOutput.write(out)
    FileHandle.standardOutput.write("\n".data(using: .utf8)!)
} else {
    for obs in sorted {
        if let top = obs.topCandidates(1).first {
            print(top.string)
        }
    }
}
