use image::ImageFormat;
use std::str::FromStr;

/// 目标格式解析错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormatError {
    #[error("不支持的图片格式：{0}")]
    Unsupported(String),
}

/// 一种目标格式的元数据：底层格式、规范扩展名与可接受的别名。
struct FormatSpec {
    format: ImageFormat,
    extension: &'static str,
    aliases: &'static [&'static str],
}

/// 支持的目标格式清单，同时是规范扩展名与别名的映射表。
const FORMAT_TABLE: &[FormatSpec] = &[
    FormatSpec {
        format: ImageFormat::Png,
        extension: "png",
        aliases: &["png"],
    },
    FormatSpec {
        format: ImageFormat::Jpeg,
        extension: "jpg",
        aliases: &["jpg", "jpeg"],
    },
    FormatSpec {
        format: ImageFormat::Gif,
        extension: "gif",
        aliases: &["gif"],
    },
    FormatSpec {
        format: ImageFormat::WebP,
        extension: "webp",
        aliases: &["webp"],
    },
    FormatSpec {
        format: ImageFormat::Pnm,
        extension: "pnm",
        aliases: &["pnm"],
    },
    FormatSpec {
        format: ImageFormat::Tiff,
        extension: "tif",
        aliases: &["tif", "tiff"],
    },
    FormatSpec {
        format: ImageFormat::Tga,
        extension: "tga",
        aliases: &["tga"],
    },
    FormatSpec {
        format: ImageFormat::Dds,
        extension: "dds",
        aliases: &["dds"],
    },
    FormatSpec {
        format: ImageFormat::Bmp,
        extension: "bmp",
        aliases: &["bmp"],
    },
    FormatSpec {
        format: ImageFormat::Ico,
        extension: "ico",
        aliases: &["ico"],
    },
    FormatSpec {
        format: ImageFormat::Hdr,
        extension: "hdr",
        aliases: &["hdr"],
    },
    FormatSpec {
        format: ImageFormat::OpenExr,
        extension: "exr",
        aliases: &["exr", "openexr"],
    },
    FormatSpec {
        format: ImageFormat::Farbfeld,
        extension: "ff",
        aliases: &["ff", "farbfeld"],
    },
    FormatSpec {
        format: ImageFormat::Avif,
        extension: "avif",
        aliases: &["avif"],
    },
    FormatSpec {
        format: ImageFormat::Qoi,
        extension: "qoi",
        aliases: &["qoi"],
    },
];

/// 扫描目录时用于筛选图片的扩展名（小写，含别名）。
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "pnm", "tif", "tiff", "tga", "dds", "bmp", "ico", "hdr",
    "exr", "openexr", "ff", "farbfeld", "avif", "qoi",
];

/// 目标输出格式，包装 image::ImageFormat，避免第三方类型泄漏到公共签名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetFormat(ImageFormat);

impl TargetFormat {
    /// 底层 image 格式。
    pub fn as_image(self) -> ImageFormat {
        self.0
    }

    /// 规范输出扩展名（不含点）。
    #[must_use]
    pub fn extension(self) -> &'static str {
        FORMAT_TABLE
            .iter()
            .find(|spec| spec.format == self.0)
            .map_or("", |spec| spec.extension)
    }

    /// 目标格式在当前编译特性下是否可写。
    #[must_use]
    pub fn can_write(self) -> bool {
        self.0.can_write()
    }
}

impl FromStr for TargetFormat {
    type Err = FormatError;

    fn from_str(text: &str) -> Result<Self, FormatError> {
        FORMAT_TABLE
            .iter()
            .find(|spec| {
                spec.aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(text.trim()))
            })
            .map(|spec| TargetFormat(spec.format))
            .ok_or_else(|| FormatError::Unsupported(text.to_string()))
    }
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
            assert_eq!(text.parse::<TargetFormat>().unwrap().as_image(), expected);
        }
    }

    #[test]
    fn parsing_is_case_insensitive_and_trims() {
        assert_eq!(
            "PNG".parse::<TargetFormat>().unwrap(),
            TargetFormat(ImageFormat::Png)
        );
        assert_eq!(
            "  WebP ".parse::<TargetFormat>().unwrap(),
            TargetFormat(ImageFormat::WebP)
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
        assert_eq!(TargetFormat(ImageFormat::Jpeg).extension(), "jpg");
        assert_eq!(TargetFormat(ImageFormat::Tiff).extension(), "tif");
        assert_eq!(TargetFormat(ImageFormat::OpenExr).extension(), "exr");
        assert_eq!(TargetFormat(ImageFormat::Farbfeld).extension(), "ff");
    }

    #[test]
    fn extension_set_covers_every_alias() {
        let listed: Vec<&str> = IMAGE_EXTENSIONS.to_vec();
        for spec in FORMAT_TABLE.iter() {
            for alias in spec.aliases.iter() {
                assert!(listed.contains(alias), "别名 {alias} 未出现在扩展名集合中");
            }
        }
        // 反向：集合中每个扩展名都能解析为某个格式。
        for extension in IMAGE_EXTENSIONS {
            assert!(extension.parse::<TargetFormat>().is_ok());
        }
    }
}
