//! 源图片元数据读取与处理模式。
//!
//! 通过 image 解码器的 exif_metadata / icc_profile 接口读取原始字节，
//! 供 --meta list 展示与 --meta keep 透传使用。

use anyhow::{Context, Result};
use image::ImageDecoder;
use std::path::Path;
use std::str::FromStr;

/// 元数据处理模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetadataMode {
    /// 尽力保留源元数据（仅在支持的编码器上生效）。
    #[default]
    Keep,
    /// 输出不携带任何元数据。
    Strip,
    /// 只列出源图片元数据摘要，不执行转换。
    List,
}

/// 元数据处理模式解析错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MetadataModeError {
    #[error("元数据模式 {0} 无效，可选：keep、strip、list")]
    Invalid(String),
}

impl FromStr for MetadataMode {
    type Err = MetadataModeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().to_ascii_lowercase().as_str() {
            "keep" => Ok(Self::Keep),
            "strip" => Ok(Self::Strip),
            "list" => Ok(Self::List),
            other => Err(MetadataModeError::Invalid(other.to_string())),
        }
    }
}

/// 源图片的原始元数据。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Metadata {
    pub exif: Option<Vec<u8>>,
    pub icc: Option<Vec<u8>>,
}

/// 读取源图片的 EXIF 与 ICC 原始字节。
///
/// 解码器不提供某类元数据时对应字段为 None；无法解码则返回错误。
pub fn read_metadata(source: &Path) -> Result<Metadata> {
    let mut decoder = image::ImageReader::open(source)
        .with_context(|| format!("打开源图片 {}", source.display()))?
        .into_decoder()
        .with_context(|| format!("解码源图片 {}", source.display()))?;

    let exif = decoder
        .exif_metadata()
        .with_context(|| format!("读取 EXIF {}", source.display()))?;
    let icc = decoder
        .icc_profile()
        .with_context(|| format!("读取 ICC {}", source.display()))?;

    Ok(Metadata { exif, icc })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_metadata_modes() {
        assert_eq!("keep".parse::<MetadataMode>().unwrap(), MetadataMode::Keep);
        assert_eq!("strip".parse::<MetadataMode>().unwrap(), MetadataMode::Strip);
        assert_eq!("list".parse::<MetadataMode>().unwrap(), MetadataMode::List);
        assert_eq!(" KEEP ".parse::<MetadataMode>().unwrap(), MetadataMode::Keep);
    }

    #[test]
    fn rejects_unknown_metadata_mode() {
        assert_eq!(
            "drop".parse::<MetadataMode>().unwrap_err(),
            MetadataModeError::Invalid("drop".to_string())
        );
    }

    #[test]
    fn default_metadata_has_no_payload() {
        let metadata = Metadata::default();
        assert_eq!(metadata.exif, None);
        assert_eq!(metadata.icc, None);
    }
}
