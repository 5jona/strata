//! Pure-Rust SIMD image downscaling (`fast_image_resize`) and PNG/JPEG/WebP encoding.

use std::io::Cursor;

use fast_image_resize::images::Image;
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{ColorType, ImageEncoder};

use super::RawFrame;

/// Encoded image payload ready for disk persistence or MCP inline return.
#[derive(Debug, Clone)]
pub(crate) struct EncodedImage {
    pub(crate) format: String,
    pub(crate) mime_type: String,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) bytes: Vec<u8>,
}

/// Downscales an RGBA frame proportionally when `max_width` is smaller than `frame.width`.
pub(crate) fn downscale_rgba(frame: &RawFrame, max_width: Option<u32>) -> Result<RawFrame, String> {
    let Some(mw) = max_width else {
        return Ok(frame.clone());
    };
    if mw == 0 || frame.width <= mw {
        return Ok(frame.clone());
    }
    let dst_w = mw;
    let dst_h =
        (((frame.height as f64) * (dst_w as f64) / (frame.width as f64)).round() as u32).max(1);

    let src = Image::from_vec_u8(
        frame.width,
        frame.height,
        frame.rgba.clone(),
        PixelType::U8x4,
    )
    .map_err(|e| format!("source image error: {e}"))?;
    let mut dst = Image::new(dst_w, dst_h, PixelType::U8x4);
    let mut resizer = Resizer::new();
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(
        fast_image_resize::FilterType::Bilinear,
    ));
    resizer
        .resize(&src, &mut dst, Some(&opts))
        .map_err(|e| format!("resize failed: {e}"))?;

    Ok(RawFrame {
        width: dst_w,
        height: dst_h,
        rgba: dst.into_vec(),
    })
}

/// Encodes an RGBA frame into `"jpeg"`, `"png"`, or `"webp"` (pure Rust, zero C deps).
pub(crate) fn encode_frame(
    frame: &RawFrame,
    format: &str,
    quality: u8,
    max_width: Option<u32>,
) -> Result<EncodedImage, String> {
    let scaled = downscale_rgba(frame, max_width)?;
    let fmt = format.to_ascii_lowercase();
    let mut buf = Vec::new();

    match fmt.as_str() {
        "jpg" | "jpeg" => {
            let mut rgb = Vec::with_capacity((scaled.width * scaled.height * 3) as usize);
            for px in scaled.rgba.as_chunks::<4>().0 {
                rgb.extend_from_slice(&px[..3]);
            }
            let enc = JpegEncoder::new_with_quality(&mut buf, quality.clamp(10, 100));
            enc.write_image(&rgb, scaled.width, scaled.height, ColorType::Rgb8.into())
                .map_err(|e| format!("jpeg encode failed: {e}"))?;
            Ok(EncodedImage {
                format: "jpeg".to_owned(),
                mime_type: "image/jpeg".to_owned(),
                width: scaled.width,
                height: scaled.height,
                bytes: buf,
            })
        }
        "webp" => {
            let enc = WebPEncoder::new_lossless(Cursor::new(&mut buf));
            enc.write_image(
                &scaled.rgba,
                scaled.width,
                scaled.height,
                ColorType::Rgba8.into(),
            )
            .map_err(|e| format!("webp encode failed: {e}"))?;
            Ok(EncodedImage {
                format: "webp".to_owned(),
                mime_type: "image/webp".to_owned(),
                width: scaled.width,
                height: scaled.height,
                bytes: buf,
            })
        }
        _ => {
            let enc = PngEncoder::new(&mut buf);
            enc.write_image(
                &scaled.rgba,
                scaled.width,
                scaled.height,
                ColorType::Rgba8.into(),
            )
            .map_err(|e| format!("png encode failed: {e}"))?;
            Ok(EncodedImage {
                format: "png".to_owned(),
                mime_type: "image/png".to_owned(),
                width: scaled.width,
                height: scaled.height,
                bytes: buf,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscales_and_encodes_png_jpeg_webp() {
        let mut rgba = Vec::with_capacity(64 * 32 * 4);
        for y in 0..32u8 {
            for x in 0..64u8 {
                rgba.extend_from_slice(&[x.wrapping_mul(4), y.wrapping_mul(8), 128, 255]);
            }
        }
        let f = RawFrame {
            width: 64,
            height: 32,
            rgba,
        };
        let jpg = encode_frame(&f, "jpeg", 80, Some(32)).expect("jpeg");
        assert_eq!((jpg.width, jpg.height), (32, 16));
        assert_eq!(&jpg.bytes[..2], &[0xFF, 0xD8]);

        let png = encode_frame(&f, "png", 80, None).expect("png");
        assert_eq!(&png.bytes[..4], b"\x89PNG");

        let webp = encode_frame(&f, "webp", 80, Some(32)).expect("webp");
        assert_eq!(&webp.bytes[..4], b"RIFF");
    }
}
