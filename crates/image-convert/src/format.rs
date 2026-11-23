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

/// image 内置格式的映射表，同时是输出格式清单。
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
        format: ImageFormat::Pnm,
        extension: "pnm",
        aliases: &["pnm"],
    },
    StandardSpec {
        format: ImageFormat::Tiff,
        extension: "tif",
        aliases: &["tif", "tiff"],
    },
    StandardSpec {
        format: ImageFormat::Tga,
        extension: "tga",
        aliases: &["tga"],
    },
    StandardSpec {
        format: ImageFormat::Dds,
        extension: "dds",
        aliases: &["dds"],
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
        format: ImageFormat::Hdr,
        extension: "hdr",
        aliases: &["hdr"],
    },
    StandardSpec {
        format: ImageFormat::OpenExr,
        extension: "exr",
        aliases: &["exr", "openexr"],
    },
    StandardSpec {
        format: ImageFormat::Farbfeld,
        extension: "ff",
        aliases: &["ff", "farbfeld"],
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
];

/// 仅可解码的扩展格式清单（输入专用）：注册解码 hook 的扩展名与别名。
struct ExtraSpec {
    variant: ExtraFormat,
    extension: &'static str,
    aliases: &'static [&'static str],
}

/// 扩展接入的目标格式变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtraFormat {
    Jxl,
    Heic,
    Jp2,
    Pcx,
    Sgi,
    Icns,
    Wbmp,
    Xpm,
    Ora,
    Otb,
    Xbm,
}

const DECODE_ONLY_TABLE: &[ExtraSpec] = &[
    ExtraSpec {
        variant: ExtraFormat::Jxl,
        extension: "jxl",
        aliases: &["jxl"],
    },
    ExtraSpec {
        variant: ExtraFormat::Heic,
        extension: "heic",
        aliases: &["heic", "heif"],
    },
    ExtraSpec {
        variant: ExtraFormat::Jp2,
        extension: "jp2",
        aliases: &["jp2", "j2k", "j2c"],
    },
    ExtraSpec {
        variant: ExtraFormat::Pcx,
        extension: "pcx",
        aliases: &["pcx"],
    },
    ExtraSpec {
        variant: ExtraFormat::Sgi,
        extension: "sgi",
        aliases: &["sgi"],
    },
    ExtraSpec {
        variant: ExtraFormat::Icns,
        extension: "icns",
        aliases: &["icns"],
    },
    ExtraSpec {
        variant: ExtraFormat::Wbmp,
        extension: "wbmp",
        aliases: &["wbmp"],
    },
    ExtraSpec {
        variant: ExtraFormat::Xpm,
        extension: "xpm",
        aliases: &["xpm"],
    },
    ExtraSpec {
        variant: ExtraFormat::Ora,
        extension: "ora",
        aliases: &["ora"],
    },
    ExtraSpec {
        variant: ExtraFormat::Otb,
        extension: "otb",
        aliases: &["otb"],
    },
    ExtraSpec {
        variant: ExtraFormat::Xbm,
        extension: "xbm",
        aliases: &["xbm"],
    },
];

/// 扫描目录时用于筛选图片的扩展名（小写）。
///
/// 包含 image 内置格式、扩展接入格式的规范扩展名与常见别名；
/// 显式传入的文件不受此清单限制。
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "pnm", "tif", "tiff", "tga", "dds", "bmp", "ico", "hdr",
    "exr", "openexr", "ff", "farbfeld", "avif", "qoi", "jxl", "heic", "heif", "jp2", "j2k", "j2c",
    "pcx", "sgi", "icns", "wbmp", "xpm", "ora", "otb", "xbm",
];

#[cfg(feature = "jxl")]
const JXL_ENABLED: bool = true;
#[cfg(not(feature = "jxl"))]
const JXL_ENABLED: bool = false;

#[cfg(feature = "heif")]
const HEIF_ENABLED: bool = true;
#[cfg(not(feature = "heif"))]
const HEIF_ENABLED: bool = false;

#[cfg(feature = "jp2")]
const JP2_ENABLED: bool = true;
#[cfg(not(feature = "jp2"))]
const JP2_ENABLED: bool = false;

#[cfg(feature = "extras")]
const EXTRAS_ENABLED: bool = true;
#[cfg(not(feature = "extras"))]
const EXTRAS_ENABLED: bool = false;

/// 输出目标格式。
///
/// Standard 系列委托给 image::ImageFormat；其余为扩展接入的格式，
/// 当前仅支持解码（输入），作为 `-f` 目标会在运行前被拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetFormat {
    Standard(ImageFormat),
    Extra(ExtraFormat),
}

impl TargetFormat {
    /// 底层 image 格式；扩展格式返回 None。
    pub fn as_image(self) -> Option<ImageFormat> {
        match self {
            Self::Standard(format) => Some(format),
            Self::Extra(_) => None,
        }
    }

    /// 规范扩展名（不含点）。
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Standard(format) => STANDARD_TABLE
                .iter()
                .find(|spec| spec.format == format)
                .map_or("", |spec| spec.extension),
            Self::Extra(extra) => DECODE_ONLY_TABLE
                .iter()
                .find(|spec| spec.variant == extra)
                .map_or("", |spec| spec.extension),
        }
    }

    /// 是否仅支持解码。
    #[must_use]
    pub fn is_decode_only(self) -> bool {
        matches!(self, Self::Extra(_))
    }

    /// 目标格式在当前编译特性下是否可写。
    #[must_use]
    pub fn can_write(self) -> bool {
        match self {
            Self::Standard(format) => format.can_write(),
            Self::Extra(_) => false,
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
                DECODE_ONLY_TABLE
                    .iter()
                    .find(|spec| {
                        spec.aliases
                            .iter()
                            .any(|alias| alias.eq_ignore_ascii_case(trimmed))
                    })
                    .map(|spec| TargetFormat::Extra(spec.variant))
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
        "  扩展接入:   jxl（特性 jxl={}）  heic/heif（特性 heif={}）  jp2/j2k/j2c（特性 jp2={}）  pcx/sgi/icns/wbmp/xpm/ora/otb/xbm（特性 extras={}）",
        JXL_ENABLED, HEIF_ENABLED, JP2_ENABLED, EXTRAS_ENABLED
    )?;
    writeln!(writer)?;
    writeln!(
        writer,
        "注: heic/jp2 等扩展格式当前仅支持解码，不能作为 -f 目标。"
    )?;
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
            ("ff", ImageFormat::Farbfeld),
            ("farbfeld", ImageFormat::Farbfeld),
            ("qoi", ImageFormat::Qoi),
        ] {
            assert_eq!(
                text.parse::<TargetFormat>().unwrap(),
                TargetFormat::Standard(expected)
            );
        }
    }

    #[test]
    fn parses_decode_only_aliases() {
        assert_eq!(
            "jxl".parse::<TargetFormat>().unwrap(),
            TargetFormat::Extra(ExtraFormat::Jxl)
        );
        assert_eq!(
            "heif".parse::<TargetFormat>().unwrap(),
            TargetFormat::Extra(ExtraFormat::Heic)
        );
        assert_eq!(
            "j2c".parse::<TargetFormat>().unwrap(),
            TargetFormat::Extra(ExtraFormat::Jp2)
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
    }

    #[test]
    fn extension_matches_canonical_name() {
        assert_eq!(TargetFormat::Standard(ImageFormat::Jpeg).extension(), "jpg");
        assert_eq!(TargetFormat::Standard(ImageFormat::Tiff).extension(), "tif");
        assert_eq!(
            TargetFormat::Standard(ImageFormat::OpenExr).extension(),
            "exr"
        );
        assert_eq!(
            TargetFormat::Standard(ImageFormat::Farbfeld).extension(),
            "ff"
        );
        assert_eq!(TargetFormat::Extra(ExtraFormat::Heic).extension(), "heic");
        assert_eq!(TargetFormat::Extra(ExtraFormat::Jp2).extension(), "jp2");
    }

    #[test]
    fn decode_only_targets_are_not_writable() {
        assert!(TargetFormat::Extra(ExtraFormat::Jxl).is_decode_only());
        assert!(!TargetFormat::Extra(ExtraFormat::Jxl).can_write());
        assert!(TargetFormat::Standard(ImageFormat::Png).can_write());
        assert!(
            TargetFormat::Standard(ImageFormat::Png)
                .as_image()
                .is_some()
        );
        assert!(TargetFormat::Extra(ExtraFormat::Heic).as_image().is_none());
    }

    #[test]
    fn quality_support_matches_encoder_capabilities() {
        assert!(TargetFormat::Standard(ImageFormat::Jpeg).supports_quality());
        assert!(TargetFormat::Standard(ImageFormat::Avif).supports_quality());
        assert!(!TargetFormat::Standard(ImageFormat::WebP).supports_quality());
        assert!(!TargetFormat::Standard(ImageFormat::Png).supports_quality());
        assert!(!TargetFormat::Extra(ExtraFormat::Jxl).supports_quality());
    }

    #[test]
    fn scan_extensions_cover_canonical_names() {
        // 每个规范扩展名都应出现在扫描集合中（别名 heif/j2k/j2c 也在）。
        for spec in STANDARD_TABLE.iter() {
            assert!(
                IMAGE_EXTENSIONS.contains(&spec.extension),
                "内置扩展 {} 未出现在扫描集合中",
                spec.extension
            );
        }
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
