use anyhow::{Result, bail};
use std::str::FromStr;

macro_rules! impl_from_str_via_new {
    ($($ty:ty),+ $(,)?) => {$(
        impl FromStr for $ty {
            type Err = anyhow::Error;

            fn from_str(text: &str) -> Result<Self> {
                Self::new(text.parse()?)
            }
        }
    )+};
}

fn ensure_range(value: f64, min: f64, max: f64, what: &str) -> Result<()> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        bail!("{what} {value} 超出范围，必须在 {min} 到 {max} 之间。")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ConstantRateFactor(u16);

impl ConstantRateFactor {
    const STEPS_PER_UNIT: f64 = 4.0;

    pub fn new(value: f64) -> Result<Self> {
        ensure_range(value, 1.0, 70.0, "CRF 值")?;

        let steps = value * Self::STEPS_PER_UNIT;
        if (steps - steps.round()).abs() > 1e-9 {
            bail!("CRF 值 {value} 无效，必须是 0.25 的整数倍。");
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
    pub fn new(value: f64) -> Result<Self> {
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
    pub fn new(value: u8) -> Result<Self> {
        if value > 50 {
            bail!("film-grain 强度 {value} 超出范围，必须在 0 到 50 之间。");
        }
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
    pub fn new(value: f64) -> Result<Self> {
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
    pub fn new(value: u8) -> Result<Self> {
        if value > 13 {
            bail!("编码 preset {value} 超出范围，必须在 0 到 13 之间。");
        }
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
