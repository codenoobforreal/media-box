use image::ImageFormat;
use std::str::FromStr;

/// 目标格式解析错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("不支持的图片格式：{0}")]
    Unsupported(String),
}

/// image 内置格式清单：规范扩展名与可接受的别名。
struct StandardSpec {
    format: ImageFormat,
    extension: &'static str,
    aliases: &'static [&'static str],
}

/// image 内置、纳入支持范围的格式映射表，同时是输出格式清单。
///
/// 只保留常用格式与有实际价值的现代/专业格式；小众或过时格式不在当前范围。
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
    StandardSpec {
        format: ImageFormat::Ico,
        extension: "ico",
        aliases: &["ico"],
    },
    StandardSpec {
        format: ImageFormat::Tiff,
        extension: "tif",
        aliases: &["tif", "tiff"],
    },
    StandardSpec {
        format: ImageFormat::Avif,
        extension: "avif",
        aliases: &["avif"],
    },
    StandardSpec {
        format: ImageFormat::Qoi,
        extension: "qoi",
        aliases: &["qoi"],
    },
    StandardSpec {
        format: ImageFormat::OpenExr,
        extension: "exr",
        aliases: &["exr", "openexr"],
    },
    StandardSpec {
        format: ImageFormat::Hdr,
        extension: "hdr",
        aliases: &["hdr"],
    },
];

/// 仅可解码的扩展格式（输入专用）：注册解码桥接的扩展名与别名。
struct ExtraSpec {
    variant: DecodeOnlyFormat,
    extension: &'static str,
    aliases: &'static [&'static str],
}

/// 仅支持解码的扩展格式变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeOnlyFormat {
    Heic,
}

const DECODE_ONLY_TABLE: &[ExtraSpec] = &[ExtraSpec {
    variant: DecodeOnlyFormat::Heic,
    extension: "heic",
    aliases: &["heic", "heif"],
}];

/// 扫描目录时用于筛选图片的扩展名（小写）。
///
/// 包含 image 内置格式、扩展接入格式的规范扩展名与常见别名；
/// 显式传入的文件不受此清单限制。
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff", "avif", "qoi", "exr",
    "openexr", "hdr", "jxl", "heic", "heif",
];

#[cfg(feature = "jxl")]
const JXL_ENABLED: bool = true;
#[cfg(not(feature = "jxl"))]
const JXL_ENABLED: bool = false;

#[cfg(feature = "heif")]
const HEIF_ENABLED: bool = true;
#[cfg(not(feature = "heif"))]
const HEIF_ENABLED: bool = false;

/// 输出目标格式。
///
/// `Standard` 委托给 image::ImageFormat；`Jxl` 由 zune-jpegxl 编码；
/// `DecodeOnly` 仅支持解码，作为 `-f` 目标会在运行前被拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFormat {
    Standard(ImageFormat),
    Jxl,
    DecodeOnly(DecodeOnlyFormat),
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
            Self::Jxl => "jxl",
            Self::DecodeOnly(variant) => DECODE_ONLY_TABLE
                .iter()
                .find(|spec| spec.variant == variant)
                .map_or("", |spec| spec.extension),
        }
    }

    /// 是否仅支持解码。
    #[must_use]
    pub fn is_decode_only(self) -> bool {
        matches!(self, Self::DecodeOnly(_))
    }

    /// 目标格式在当前编译特性下是否可写。
    #[must_use]
    pub fn can_write(self) -> bool {
        match self {
            Self::Standard(format) => format.can_write(),
            Self::Jxl => JXL_ENABLED,
            Self::DecodeOnly(_) => false,
        }
    }

    /// -q 质量参数是否对该输出格式生效。
    #[must_use]
    pub fn supports_quality(self) -> bool {
        matches!(
            self,
            Self::Standard(ImageFormat::Jpeg) | Self::Standard(ImageFormat::Avif)
        )
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
            .or_else(|| {
                if trimmed.eq_ignore_ascii_case("jxl") {
                    Some(TargetFormat::Jxl)
                } else {
                    DECODE_ONLY_TABLE
                        .iter()
                        .find(|spec| {
                            spec.aliases
                                .iter()
                                .any(|alias| alias.eq_ignore_ascii_case(trimmed))
                        })
                        .map(|spec| TargetFormat::DecodeOnly(spec.variant))
                }
            })
            .ok_or_else(|| FormatError::Unsupported(text.to_string()))
    }
}

/// 向 writer 打印输入/输出能力矩阵（--list-formats）。
pub fn print_capabilities(writer: &mut impl std::io::Write) -> std::io::Result<()> {
    writeln!(writer, "输出格式（-f 编码目标）:")?;
    for spec in STANDARD_TABLE.iter() {
        let target = TargetFormat::Standard(spec.format);
        let quality = if target.supports_quality() {
            "有损，-q 生效"
        } else if spec.format == ImageFormat::WebP {
            "无损（纯 Rust 生态暂无 WebP 有损编码器，-q 不生效）"
        } else {
            "无损，-q 不生效"
        };
        writeln!(writer, "  {:<6} {}", spec.extension, quality)?;
    }
    let jxl_note = if JXL_ENABLED {
        "无损，-q 不生效"
    } else {
        "无损，-q 不生效（特性 jxl 未启用，不可写）"
    };
    writeln!(writer, "  {:<6} {}", "jxl", jxl_note)?;

    writeln!(writer)?;
    writeln!(writer, "输入格式（解码）:")?;
    writeln!(
        writer,
        "  image 内置: {}",
        STANDARD_TABLE
            .iter()
            .map(|spec| spec.extension)
            .collect::<Vec<_>>()
            .join(" ")
    )?;
    writeln!(
        writer,
        "  扩展接入:   jxl（特性 jxl={}）  heic/heif（特性 heif={}）",
        JXL_ENABLED, HEIF_ENABLED
    )?;
    writeln!(writer)?;
    writeln!(writer, "注: heic 等扩展格式当前仅支持解码，不能作为 -f 目标。")?;
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
            ("webp", ImageFormat::WebP),
            ("avif", ImageFormat::Avif),
            ("tif", ImageFormat::Tiff),
            ("tiff", ImageFormat::Tiff),
            ("exr", ImageFormat::OpenExr),
            ("openexr", ImageFormat::OpenExr),
            ("qoi", ImageFormat::Qoi),
        ] {
            assert_eq!(
                text.parse::<TargetFormat>().unwrap(),
                TargetFormat::Standard(expected)
            );
        }
    }

    #[test]
    fn parses_jxl_and_decode_only_aliases() {
        assert_eq!("jxl".parse::<TargetFormat>().unwrap(), TargetFormat::Jxl);
        assert_eq!(
            "heic".parse::<TargetFormat>().unwrap(),
            TargetFormat::DecodeOnly(DecodeOnlyFormat::Heic)
        );
        assert_eq!(
            "heif".parse::<TargetFormat>().unwrap(),
            TargetFormat::DecodeOnly(DecodeOnlyFormat::Heic)
        );
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
    }

    #[test]
    fn extension_matches_canonical_name() {
        assert_eq!(TargetFormat::Standard(ImageFormat::Jpeg).extension(), "jpg");
        assert_eq!(TargetFormat::Standard(ImageFormat::Tiff).extension(), "tif");
        assert_eq!(
            TargetFormat::Standard(ImageFormat::OpenExr).extension(),
            "exr"
        );
        assert_eq!(TargetFormat::Jxl.extension(), "jxl");
        assert_eq!(
            TargetFormat::DecodeOnly(DecodeOnlyFormat::Heic).extension(),
            "heic"
        );
    }

    #[test]
    fn decode_only_targets_are_not_writable() {
        assert!(TargetFormat::DecodeOnly(DecodeOnlyFormat::Heic).is_decode_only());
        assert!(!TargetFormat::DecodeOnly(DecodeOnlyFormat::Heic).can_write());
        assert!(!TargetFormat::Jxl.is_decode_only());
        assert_eq!(TargetFormat::Jxl.can_write(), JXL_ENABLED);
        assert!(TargetFormat::Standard(ImageFormat::Png).can_write());
    }

    #[test]
    fn quality_support_matches_encoder_capabilities() {
        assert!(TargetFormat::Standard(ImageFormat::Jpeg).supports_quality());
        assert!(TargetFormat::Standard(ImageFormat::Avif).supports_quality());
        assert!(!TargetFormat::Standard(ImageFormat::WebP).supports_quality());
        assert!(!TargetFormat::Standard(ImageFormat::Png).supports_quality());
        assert!(!TargetFormat::Jxl.supports_quality());
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
        assert!(IMAGE_EXTENSIONS.contains(&"jxl"));
        for spec in DECODE_ONLY_TABLE.iter() {
            assert!(
                IMAGE_EXTENSIONS.contains(&spec.extension),
                "扩展格式 {} 未出现在扫描集合中",
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
