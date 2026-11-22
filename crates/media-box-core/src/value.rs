use std::str::FromStr;

/// 各数值 newtype 在构造与解析阶段产生的错误。
///
/// 作为库的公共错误类型，不向下游暴露 `anyhow::Error`；
/// 需要 anyhow 的场景由 `From<ValueError> for anyhow::Error` 自动转换。
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ValueError {
    #[error("{what} {value} 超出范围，必须在 {min} 到 {max} 之间。")]
    OutOfRange {
        value: f64,
        min: f64,
        max: f64,
        what: &'static str,
    },
    #[error("CRF 值 {value} 无效，必须是 0.25 的整数倍。")]
    InvalidCrfStep { value: f64 },
    #[error("无法解析整数 {0}")]
    ParseInt(#[from] std::num::ParseIntError),
    #[error("无法解析数值 {0}")]
    ParseFloat(#[from] std::num::ParseFloatError),
}

macro_rules! impl_from_str_via_new {
    ($($ty:ty),+ $(,)?) => {$(
        impl FromStr for $ty {
            type Err = ValueError;

            fn from_str(text: &str) -> Result<Self, ValueError> {
                Self::new(text.parse()?)
            }
        }
    )+};
}

fn ensure_range(value: f64, min: f64, max: f64, what: &'static str) -> Result<(), ValueError> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(ValueError::OutOfRange {
            value,
            min,
            max,
            what,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConstantRateFactor(u16);

impl ConstantRateFactor {
    const STEPS_PER_UNIT: f64 = 4.0;

    pub fn new(value: f64) -> Result<Self, ValueError> {
        ensure_range(value, 1.0, 70.0, "CRF 值")?;

        let steps = value * Self::STEPS_PER_UNIT;
        if (steps - steps.round()).abs() > 1e-9 {
            return Err(ValueError::InvalidCrfStep { value });
        }

        Ok(Self(steps.round() as u16))
    }

    /// 四舍五入到 0.25 档并夹到合法区间：供内部「已保证在区间内」的地方使用，避免把不可能失败的错误一路 `?` 上去。
    pub fn snapped_clamped(value: f64) -> Self {
        Self((value.clamp(1.0, 70.0) * Self::STEPS_PER_UNIT).round() as u16)
    }

    pub fn as_f64(self) -> f64 {
        f64::from(self.0) / Self::STEPS_PER_UNIT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct VmafScore(f64);

impl VmafScore {
    pub fn new(value: f64) -> Result<Self, ValueError> {
        ensure_range(value, 1.0, 100.0, "VMAF 分数")?;
        Ok(Self(value))
    }

    pub fn as_f64(self) -> f64 {
        self.0
    }
}

impl Default for VmafScore {
    fn default() -> Self {
        Self(91.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FilmGrain(u8);

impl FilmGrain {
    pub fn new(value: u8) -> Result<Self, ValueError> {
        ensure_range(f64::from(value), 0.0, 50.0, "film-grain 强度")?;
        Ok(Self(value))
    }

    pub fn as_u8(self) -> u8 {
        self.0
    }
}

impl Default for FilmGrain {
    fn default() -> Self {
        Self(8)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct FrameRatePerSecond(f64);

impl FrameRatePerSecond {
    pub fn new(value: f64) -> Result<Self, ValueError> {
        ensure_range(value, 1.0, 120.0, "帧率")?;
        Ok(Self(value))
    }

    /// 直接采信 ffprobe：部分素材高于 CLI 允许的 120 上限。
    pub(crate) fn from_probed(value: f64) -> Self {
        Self(value)
    }

    pub fn as_f64(self) -> f64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EncoderPreset(u8);

impl EncoderPreset {
    pub fn new(value: u8) -> Result<Self, ValueError> {
        ensure_range(f64::from(value), 0.0, 13.0, "编码 preset")?;
        Ok(Self(value))
    }

    pub fn as_u8(self) -> u8 {
        self.0
    }
}

impl Default for EncoderPreset {
    fn default() -> Self {
        Self(4)
    }
}

impl_from_str_via_new!(
    ConstantRateFactor,
    VmafScore,
    FilmGrain,
    FrameRatePerSecond,
    EncoderPreset
);
