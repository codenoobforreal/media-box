//! 输入解码接入层。
//!
//! 个人工具定位，只启用常用格式的解码，直接走 image 内置解码器。

use anyhow::{Context, Result};
use image::DynamicImage;

/// 按扩展名把源文件解码为 `DynamicImage`。
///
/// 常用格式（png、jpg/jpeg、gif、webp、bmp）直接交给 image。
pub(crate) fn decode_input(source: &std::path::Path) -> Result<DynamicImage> {
    image::ImageReader::open(source)
        .with_context(|| format!("打开源图片 {}", source.display()))?
        .decode()
        .with_context(|| format!("解码源图片 {}", source.display()))
}
