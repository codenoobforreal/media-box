use anyhow::{Result, bail};
use std::fmt;
use std::str::FromStr;

/// 分辨率枚举，包含常见标准分辨率及任意自定义分辨率。命名约定：`V` 前缀表示竖屏（Vertical）版本
#[non_exhaustive]
#[derive(Debug, PartialEq, Clone, Copy, Default, Eq, PartialOrd, Ord)]
pub enum Resolution {
    /// 3840x2160 (4K UHD 横屏)
    Uhd,
    /// 2160x3840 (4K UHD 竖屏)
    Vuhd,
    /// 2560x1440 (2K QHD 横屏)
    Qhd,
    /// 1440x2560 (2K QHD 竖屏)
    Vqhd,
    /// 1920x1080 (1080p 横屏)
    Fhd,
    /// 1080x1920 (1080p 竖屏)
    Vfhd,
    /// 1280x720 (720p 横屏)
    #[default]
    Hd,
    /// 720x1280 (720p 竖屏)
    Vhd,
    /// 自定义分辨率
    Arbitrary { width: u16, height: u16 },
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ResolutionError {
    #[error("宽或高不能为 0")]
    Zero,
    #[error("缺少分隔符")]
    NoDelimiterX,
    #[error("缺少宽度")]
    MissingWidth,
    #[error("缺少高度")]
    MissingHeight,
    #[error("解析 {0} 失败")]
    Parse(String),
}

impl FromStr for Resolution {
    type Err = ResolutionError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let s = s.trim();
        let (width_str, height_str) = s
            .split_once('x')
            .or_else(|| s.split_once('X'))
            .or_else(|| s.split_once('×'))
            .ok_or(ResolutionError::NoDelimiterX)?;

        if width_str.is_empty() {
            return Err(ResolutionError::MissingWidth);
        }

        if height_str.is_empty() {
            return Err(ResolutionError::MissingHeight);
        }

        let parse_u16 = |val: &str| -> std::result::Result<u16, ResolutionError> {
            val.trim()
                .parse::<u16>()
                .map_err(|_| ResolutionError::Parse(val.trim().to_owned()))
        };

        let width = parse_u16(width_str)?;
        let height = parse_u16(height_str)?;

        Resolution::new(width, height).map_err(|_| ResolutionError::Zero)
    }
}

impl Resolution {
    pub fn new(width: u16, height: u16) -> Result<Self> {
        if width == 0 || height == 0 {
            bail!("宽或高不能为 0");
        }

        Ok(match (width, height) {
            (3_840, 2_160) => Self::Uhd,
            (2_160, 3_840) => Self::Vuhd,
            (2_560, 1_440) => Self::Qhd,
            (1_440, 2_560) => Self::Vqhd,
            (1_920, 1_080) => Self::Fhd,
            (1_080, 1_920) => Self::Vfhd,
            (1_280, 720) => Self::Hd,
            (720, 1_280) => Self::Vhd,
            _ => Self::Arbitrary { width, height },
        })
    }

    pub fn pixels(self) -> u32 {
        u32::from(self.width()) * u32::from(self.height())
    }

    pub fn width(self) -> u16 {
        match self {
            Self::Uhd => 3_840,
            Self::Vuhd => 2_160,
            Self::Qhd => 2_560,
            Self::Vqhd => 1_440,
            Self::Fhd => 1_920,
            Self::Vfhd => 1_080,
            Self::Hd => 1_280,
            Self::Vhd => 720,
            Self::Arbitrary { width, height: _ } => width,
        }
    }

    pub fn height(self) -> u16 {
        match self {
            Self::Uhd => 2_160,
            Self::Vuhd => 3_840,
            Self::Qhd => 1_440,
            Self::Vqhd => 2_560,
            Self::Fhd => 1_080,
            Self::Vfhd => 1_920,
            Self::Hd => 720,
            Self::Vhd => 1_280,
            Self::Arbitrary { width: _, height } => height,
        }
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Uhd => write!(f, "3840x2160"),
            Self::Vuhd => write!(f, "2160x3840"),
            Self::Qhd => write!(f, "2560x1440"),
            Self::Vqhd => write!(f, "1440x2560"),
            Self::Fhd => write!(f, "1920x1080"),
            Self::Vfhd => write!(f, "1080x1920"),
            Self::Hd => write!(f, "1280x720"),
            Self::Vhd => write!(f, "720x1280"),
            Self::Arbitrary { width, height } => write!(f, "{width}x{height}"),
        }
    }
}
