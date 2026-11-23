//! 输入解码接入层。
//!
//! jxl / JPEG 2000 / image-extras 系列（PCX、SGI、ICNS、WBMP、XPM、OpenRaster 等）
//! 通过 image 0.25 的解码 hook 接入：注册后 `ImageReader::open(...).decode()`
//! 会按扩展名自动分发，无需自定义调度。HEIF/HEIC 没有可用的 hook 接口，
//! 由 `decode_input` 内的专用桥接处理。
//!
//! 所有接入 crate 均为纯 Rust、宽松许可（核验时点 2026-11-23，见 Cargo.toml 注释）。

use anyhow::{Context, Result};
use image::DynamicImage;

/// 注册全部已启用特性的输入解码 hook。幂等，进程内只需调用一次。
pub fn register_input_decoders() {
    #[cfg(feature = "extras")]
    image_extras::register();

    #[cfg(feature = "jxl")]
    {
        let _ = jxl_oxide::integration::register_image_decoding_hook();
    }

    #[cfg(feature = "jp2")]
    {
        let _ = hayro_jpeg2000::integration::register_decoding_hook();
    }
}

/// 按扩展名把源文件解码为 DynamicImage。
///
/// heic/heif 走 heif-oxide 桥接，其余交给 image（内置格式 + 已注册 hook）。
pub fn decode_input(source: &std::path::Path) -> Result<DynamicImage> {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    #[cfg(feature = "heif")]
    if extension == "heic" || extension == "heif" {
        return decode_heif(source);
    }

    image::ImageReader::open(source)
        .with_context(|| format!("打开源图片 {}", source.display()))?
        .decode()
        .with_context(|| format!("解码源图片 {}", source.display()))
}

/// heif-oxide 桥接：容器解析 + 纯 Rust HEVC 解码，统一转 8bit DynamicImage。
#[cfg(feature = "heif")]
fn decode_heif(source: &std::path::Path) -> Result<DynamicImage> {
    use image::{RgbImage, RgbaImage};

    let decoded = heif_oxide::decode_file(source)
        .with_context(|| format!("解码源图片 {}", source.display()))?;
    let (width, height) = (decoded.width, decoded.height);

    let image = match decoded.pixels {
        heif_oxide::Pixels::Rgb8(buf) => DynamicImage::ImageRgb8(
            RgbImage::from_raw(width, height, buf)
                .with_context(|| format!("HEIF 像素缓冲与尺寸不符：{}x{}", width, height))?,
        ),
        heif_oxide::Pixels::Rgba8(buf) => DynamicImage::ImageRgba8(
            RgbaImage::from_raw(width, height, buf)
                .with_context(|| format!("HEIF 像素缓冲与尺寸不符：{}x{}", width, height))?,
        ),
        // 16bit 输出统一压到 8bit（to_rgba8 已做归一化）。
        _ => {
            let rgba = decoded.to_rgba8();
            DynamicImage::ImageRgba8(
                RgbaImage::from_raw(width, height, rgba)
                    .with_context(|| format!("HEIF 像素缓冲与尺寸不符：{}x{}", width, height))?,
            )
        }
    };
    Ok(image)
}
