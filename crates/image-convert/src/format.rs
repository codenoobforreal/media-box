//! 目标格式定义：内置格式表、扩展名解析和能力矩阵。
//!
//! 个人工具定位，只保留常用格式：png、jpg/jpeg、gif、webp、bmp。

use image::ImageFormat;
use std::str::FromStr;

/// 目标格式解析错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("不支持的图片格式：{0}")]
    Unsupported(String),
}

/// 纳入支持范围的格式映射表，同时是输出格式清单。
const STANDARD_TABLE: &[StandardSpec] = &[
    StandardSpec {
        format: ImageFormat::Png,
        extension: "png",
        aliases: &["png"],
    },
    StandardSpec {
        format: ImageFormat::Jpeg,
        extension: "jpg",
        aliases: &["jpg", "jpeg"],
    },
    StandardSpec {
        format: ImageFormat::Gif,
        extension: "gif",
        aliases: &["gif"],
    },
    StandardSpec {
        format: ImageFormat::WebP,
        extension: "webp",
        aliases: &["webp"],
    },
    StandardSpec {
        format: ImageFormat::Bmp,
        extension: "bmp",
        aliases: &["bmp"],
    },
];

struct StandardSpec {
    format: ImageFormat,
    extension: &'static str,
    aliases: &'static [&'static str],
}

/// 扫描目录时用于筛选图片的扩展名（小写）。
///
/// 包含内置格式的规范扩展名与常见别名；
/// 显式传入的文件不受此清单限制。
pub const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// 输出目标格式。
///
/// 委托给 image::ImageFormat，全部可写。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFormat {
    Standard(ImageFormat),
}

impl TargetFormat {
    /// 规范扩展名（不含点）。
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Standard(format) => STANDARD_TABLE
                .iter()
                .find(|spec| spec.format == format)
                .map_or("", |spec| spec.extension),
        }
    }

    /// 目标格式在当前编译特性下是否可写。
    #[must_use]
    pub fn can_write(self) -> bool {
        match self {
            Self::Standard(format) => format.can_write(),
        }
    }

    /// -q 质量参数是否对该输出格式生效。
    #[must_use]
    pub fn supports_quality(self) -> bool {
        matches!(self, Self::Standard(ImageFormat::Jpeg))
    }
}

impl Default for TargetFormat {
    fn default() -> Self {
        Self::Standard(ImageFormat::Png)
    }
}

impl FromStr for TargetFormat {
    type Err = FormatError;

    fn from_str(text: &str) -> Result<Self, FormatError> {
        let trimmed = text.trim();
        STANDARD_TABLE
            .iter()
            .find(|spec| {
                spec.aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(trimmed))
            })
            .map(|spec| TargetFormat::Standard(spec.format))
            .ok_or_else(|| FormatError::Unsupported(text.to_string()))
    }
}

/// 向 writer 打印输出格式清单（--list-formats）。
pub fn print_capabilities(writer: &mut impl std::io::Write) -> std::io::Result<()> {
    writeln!(writer, "输出格式（-f 编码目标）:")?;
    for spec in STANDARD_TABLE.iter() {
        let target = TargetFormat::Standard(spec.format);
        let quality = if target.supports_quality() {
            "有损，-q 生效"
        } else {
            "无损，-q 不生效"
        };
        writeln!(writer, "  {:<6} {}", spec.extension, quality)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_canonical_and_alias_names() {
        for (text, expected) in [
            ("png", ImageFormat::Png),
            ("jpg", ImageFormat::Jpeg),
            ("jpeg", ImageFormat::Jpeg),
            ("gif", ImageFormat::Gif),
            ("webp", ImageFormat::WebP),
            ("bmp", ImageFormat::Bmp),
        ] {
            assert_eq!(
                text.parse::<TargetFormat>().unwrap(),
                TargetFormat::Standard(expected)
            );
        }
    }

    #[test]
    fn parsing_is_case_insensitive_and_trims() {
        assert_eq!(
            "PNG".parse::<TargetFormat>().unwrap(),
            TargetFormat::Standard(ImageFormat::Png)
        );
        assert_eq!(
            "  WebP ".parse::<TargetFormat>().unwrap(),
            TargetFormat::Standard(ImageFormat::WebP)
        );
    }

    #[test]
    fn unknown_format_is_rejected() {
        assert_eq!(
            "bmp4".parse::<TargetFormat>().unwrap_err(),
            FormatError::Unsupported("bmp4".to_string())
        );
        assert_eq!(
            "jp2".parse::<TargetFormat>().unwrap_err(),
            FormatError::Unsupported("jp2".to_string())
        );
        assert_eq!(
            "avif".parse::<TargetFormat>().unwrap_err(),
            FormatError::Unsupported("avif".to_string())
        );
        assert_eq!(
            "jxl".parse::<TargetFormat>().unwrap_err(),
            FormatError::Unsupported("jxl".to_string())
        );
        assert_eq!(
            "heic".parse::<TargetFormat>().unwrap_err(),
            FormatError::Unsupported("heic".to_string())
        );
    }

    #[test]
    fn extension_matches_canonical_name() {
        assert_eq!(TargetFormat::Standard(ImageFormat::Jpeg).extension(), "jpg");
        assert_eq!(TargetFormat::Standard(ImageFormat::Png).extension(), "png");
        assert_eq!(TargetFormat::Standard(ImageFormat::Bmp).extension(), "bmp");
        assert_eq!(TargetFormat::Standard(ImageFormat::Gif).extension(), "gif");
        assert_eq!(
            TargetFormat::Standard(ImageFormat::WebP).extension(),
            "webp"
        );
    }

    #[test]
    fn quality_support_matches_encoder_capabilities() {
        assert!(TargetFormat::Standard(ImageFormat::Jpeg).supports_quality());
        assert!(!TargetFormat::Standard(ImageFormat::Png).supports_quality());
        assert!(!TargetFormat::Standard(ImageFormat::WebP).supports_quality());
        assert!(!TargetFormat::Standard(ImageFormat::Gif).supports_quality());
        assert!(!TargetFormat::Standard(ImageFormat::Bmp).supports_quality());
    }

    #[test]
    fn scan_extensions_cover_canonical_names() {
        // 每个规范扩展名都应出现在扫描集合中（别名也在）。
        for spec in STANDARD_TABLE.iter() {
            assert!(
                IMAGE_EXTENSIONS.contains(&spec.extension),
                "内置扩展 {} 未出现在扫描集合中",
                spec.extension
            );
        }
        // 反向：扫描集合中每个扩展名都能解析为某个格式。
        for extension in IMAGE_EXTENSIONS {
            assert!(
                extension.parse::<TargetFormat>().is_ok(),
                "扫描扩展 {extension} 无法解析为目标格式"
            );
        }
    }
}
