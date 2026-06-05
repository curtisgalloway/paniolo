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

//! Pixel-format carrier and RGB conversion.
//!
//! Frames travel from the capture backend to the HTTP handlers in their
//! native format; RGB is materialized lazily, only by the handler that needs
//! it (/snapshot, /ocr, the macOS preview encode). This keeps the per-frame
//! hot loop free of full-image conversion.

use std::sync::Arc;

use image::RgbImage;

/// Native pixel data of one captured frame.
#[derive(Clone)]
pub enum PixelData {
    /// No pixels available (NoDevice placeholder, or Linux MJPEG-only frames
    /// where `FrameState.jpeg` carries the image).
    Empty,
    /// Packed RGB8 (Linux decode path, YUYV fallbacks).
    Rgb(Arc<[u8]>),
    /// Bi-planar 4:2:0 as delivered by AVFoundation ('420v', video-range
    /// BT.601): `y` is w*h luma, `cbcr` is w*(h/2) interleaved chroma.
    /// Only the macOS backend constructs this; Linux still matches on it
    /// in the shared handlers, hence the cfg'd dead-code allow.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    Nv12 { y: Arc<[u8]>, cbcr: Arc<[u8]> },
}

/// Video-range BT.601 YCbCr -> RGB, integer arithmetic.
/// y is the raw luma byte (16..235 nominal), cb/cr raw chroma bytes.
#[inline]
fn ycbcr_to_rgb(y: u8, cb: u8, cr: u8) -> [u8; 3] {
    let c = y as i32 - 16;
    let d = cb as i32 - 128;
    let e = cr as i32 - 128;
    let clamp = |v: i32| v.clamp(0, 255) as u8;
    [
        clamp((298 * c + 409 * e + 128) >> 8),
        clamp((298 * c - 100 * d - 208 * e + 128) >> 8),
        clamp((298 * c + 516 * d + 128) >> 8),
    ]
}

/// Full-resolution NV12 -> RGB. Used by /snapshot and /ocr, where every
/// pixel matters.
pub fn nv12_to_rgb(y: &[u8], cbcr: &[u8], w: u32, h: u32) -> RgbImage {
    let mut rgb = RgbImage::new(w, h);
    let (w, h) = (w as usize, h as usize);
    for row in 0..h {
        let c_row = row / 2;
        for col in 0..w {
            let yi = row * w + col;
            let ci = c_row * w + (col & !1);
            if yi >= y.len() || ci + 1 >= cbcr.len() {
                continue;
            }
            let px = ycbcr_to_rgb(y[yi], cbcr[ci], cbcr[ci + 1]);
            rgb.put_pixel(col as u32, row as u32, image::Rgb(px));
        }
    }
    rgb
}

/// Half-resolution NV12 -> RGB for the preview encode. 4:2:0 chroma is
/// stored exactly per 2x2 luma block, so halving is the natural downscale:
/// average the 4 lumas, take the block's chroma as-is.
pub fn nv12_to_rgb_half(y: &[u8], cbcr: &[u8], w: u32, h: u32) -> RgbImage {
    let (ow, oh) = (w / 2, h / 2);
    let mut rgb = RgbImage::new(ow, oh);
    let w = w as usize;
    for row in 0..oh as usize {
        for col in 0..ow as usize {
            let yi = (row * 2) * w + col * 2;
            let ci = row * w + col * 2;
            if yi + w + 1 >= y.len() || ci + 1 >= cbcr.len() {
                continue;
            }
            let y_avg =
                (y[yi] as u32 + y[yi + 1] as u32 + y[yi + w] as u32 + y[yi + w + 1] as u32 + 2) / 4;
            let px = ycbcr_to_rgb(y_avg as u8, cbcr[ci], cbcr[ci + 1]);
            rgb.put_pixel(col as u32, row as u32, image::Rgb(px));
        }
    }
    rgb
}

/// Packed YUYV (Y0 Cb Y1 Cr) -> RGB, full-range coefficients. Kept for the
/// Linux YUYV fallback path and macOS 'yuvs'-only devices.
pub fn yuyv_to_rgb(buf: &[u8], w: u32, h: u32) -> RgbImage {
    let mut rgb = RgbImage::new(w, h);
    let pairs = (w * h / 2) as usize;
    for i in 0..pairs {
        let base = i * 4;
        if base + 3 >= buf.len() {
            break;
        }
        let y0 = buf[base] as f32;
        let cb = buf[base + 1] as f32 - 128.0;
        let y1 = buf[base + 2] as f32;
        let cr = buf[base + 3] as f32 - 128.0;
        let to_u8 = |v: f32| v.clamp(0.0, 255.0) as u8;
        let r = |y: f32| to_u8(y + 1.402 * cr);
        let g = |y: f32| to_u8(y - 0.344 * cb - 0.714 * cr);
        let b = |y: f32| to_u8(y + 1.772 * cb);
        let x0 = ((i * 2) % w as usize) as u32;
        let y_row = ((i * 2) / w as usize) as u32;
        if x0 < w && y_row < h {
            rgb.put_pixel(x0, y_row, image::Rgb([r(y0), g(y0), b(y0)]));
        }
        if x0 + 1 < w && y_row < h {
            rgb.put_pixel(x0 + 1, y_row, image::Rgb([r(y1), g(y1), b(y1)]));
        }
    }
    rgb
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nv12_solid(y_val: u8, cb: u8, cr: u8, w: u32, h: u32) -> (Vec<u8>, Vec<u8>) {
        let y = vec![y_val; (w * h) as usize];
        let cbcr: Vec<u8> = (0..(w * h / 2) as usize)
            .map(|i| if i % 2 == 0 { cb } else { cr })
            .collect();
        (y, cbcr)
    }

    #[test]
    fn nv12_video_black_is_rgb_black() {
        // Video-range black: Y=16, Cb=Cr=128.
        let (y, c) = nv12_solid(16, 128, 128, 8, 8);
        let img = nv12_to_rgb(&y, &c, 8, 8);
        assert_eq!(img.get_pixel(3, 3).0, [0, 0, 0]);
    }

    #[test]
    fn nv12_video_white_is_rgb_white() {
        // Video-range white: Y=235, Cb=Cr=128.
        let (y, c) = nv12_solid(235, 128, 128, 8, 8);
        let img = nv12_to_rgb(&y, &c, 8, 8);
        assert_eq!(img.get_pixel(3, 3).0, [255, 255, 255]);
    }

    #[test]
    fn nv12_half_dims_and_value() {
        let (y, c) = nv12_solid(126, 128, 128, 16, 16);
        let img = nv12_to_rgb_half(&y, &c, 16, 16);
        assert_eq!((img.width(), img.height()), (8, 8));
        // mid-grey stays mid-grey-ish through video-range expansion
        let p = img.get_pixel(4, 4).0;
        assert!(p[0] > 100 && p[0] < 160, "{p:?}");
        assert_eq!(p[0], p[1]);
        assert_eq!(p[1], p[2]);
    }
}
