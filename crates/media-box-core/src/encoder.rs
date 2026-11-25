//! 分辨率 / 帧率缩减设置与 AV1 编码参数，拼接 -vf 滤镜链。

use crate::resolution::Resolution;
use crate::value::{ConstantRateFactor, EncoderPreset, FilmGrain, FrameRatePerSecond};
use std::fmt::Write;
use std::process::Command;

/// 分辨率缩减设置：用户目标分辨率（可为 None 表示跟随源）与源分辨率。
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct ResolutionDownScaling {
    user_resolution: Option<Resolution>,
    source_resolution: Resolution,
}

impl ResolutionDownScaling {
    /// 构造分辨率缩减设置。
    pub fn new(user_resolution: Option<Resolution>, source_resolution: Resolution) -> Self {
        Self {
            user_resolution,
            source_resolution,
        }
    }

    /// 实际输出分辨率。
    pub fn target_resolution(self) -> Resolution {
        self.user_resolution.unwrap_or(self.source_resolution)
    }

    fn scale_filter(self) -> Option<String> {
        let target = self.target_resolution();
        let source = self.source_resolution;

        // 宽与高分别比对：跨朝向时像素数比较会漏掉单维超限的情况
        // （如 1000x2000 对 1080p，像素更少但高度超限）。
        if source.width() <= target.width() && source.height() <= target.height() {
            return None;
        }

        // 等比缩小取更紧的一维约束即可保证另一维也不超；
        // force_original_aspect_ratio=decrease + force_divisible_by=2 由 ffmpeg
        // 处理「哪一维更紧」与 yuv420p 的偶数要求，不再依赖源/目标朝向一致。
        // 加黑边补满分辨率用pad，为了体积更小不这样做
        // scale=1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2:flags=lanczos,pad=1920:1080:(ow-iw)/2:(oh-ih)/2
        Some(format!(
            "scale={}:{}:force_original_aspect_ratio=decrease:force_divisible_by=2:flags=lanczos",
            target.width(),
            target.height()
        ))
    }
}

/// 帧率缩减设置：用户上限（可为 None）与源帧率。
#[derive(Debug, PartialEq, PartialOrd, Clone, Copy)]
pub struct FrameRateReduce {
    user_fps: Option<FrameRatePerSecond>,
    source_fps: FrameRatePerSecond,
}

impl FrameRateReduce {
    /// 构造帧率缩减设置。
    pub fn new(user_fps: Option<FrameRatePerSecond>, source_fps: FrameRatePerSecond) -> Self {
        Self {
            user_fps,
            source_fps,
        }
    }

    fn target_fps(self) -> Option<FrameRatePerSecond> {
        self.user_fps.filter(|fps| self.source_fps > *fps)
    }
}

/// 视频降码设置：分辨率与帧率的组合。
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct BitrateReduceSettings {
    resolution: ResolutionDownScaling,
    frame_rate: FrameRateReduce,
}

impl BitrateReduceSettings {
    /// 构造降码设置。
    pub fn new(resolution: ResolutionDownScaling, frame_rate: FrameRateReduce) -> Self {
        Self {
            resolution,
            frame_rate,
        }
    }

    /// 源分辨率。
    pub fn input_resolution(self) -> Resolution {
        self.resolution.source_resolution
    }

    /// 实际输出分辨率：发生缩放时取目标，否则保持源分辨率。
    ///
    /// 用户目标大于源时 `scale_filter()` 返回 None（不放大），此时实际输出仍是源，
    /// 不能把目标边界当作实际尺寸——CRF 区间与 VMAF 模型都依赖这里。
    pub fn output_resolution(self) -> Resolution {
        if self.resolution.scale_filter().is_some() {
            self.resolution.target_resolution()
        } else {
            self.input_resolution()
        }
    }

    /// 源帧率。
    pub fn input_fps(self) -> FrameRatePerSecond {
        self.frame_rate.source_fps
    }

    /// 实际输出帧率。
    pub fn target_fps(self) -> FrameRatePerSecond {
        self.frame_rate.target_fps().unwrap_or(self.input_fps())
    }

    /// 拼一个完整的 `-vf` 字符串：`fps` 在前、`scale` 在后，只分配一次。
    ///
    /// 顺序有讲究：先降帧再缩放，滤镜链更短，缩放的输入也更少。
    pub fn video_filter(self) -> Option<String> {
        let mut filter = String::new();

        if let Some(fps) = self.frame_rate.target_fps() {
            let _ = write!(filter, "fps={}", fps.as_f64());
        }

        if let Some(scale) = self.resolution.scale_filter() {
            if !filter.is_empty() {
                filter.push(',');
            }
            filter.push_str(&scale);
        }

        (!filter.is_empty()).then_some(filter)
    }

    /// 是否发生降码（分辨率或帧率至少一项被削减）。
    pub fn is_reduced(self) -> bool {
        self.resolution.scale_filter().is_some() || self.frame_rate.target_fps().is_some()
    }
}

/// AV1 编码参数：preset、CRF 与 film-grain 强度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Av1EncodeArgs {
    preset: EncoderPreset,
    crf: ConstantRateFactor,
    film_grain: FilmGrain,
}

impl Av1EncodeArgs {
    /// 构造编码参数。
    pub fn new(preset: EncoderPreset, crf: ConstantRateFactor, film_grain: FilmGrain) -> Self {
        Self {
            preset,
            crf,
            film_grain,
        }
    }

    /// 生成 -svtav1-params 的值。
    pub fn svt_av1_params(self) -> String {
        format!(
            "tune=5:keyint=-1:qp-scale-compress-strength=1:film-grain={}",
            self.film_grain.as_u8()
        )
    }

    /// 把 `-c:v … -svtav1-params …` 追加到命令行。
    pub fn apply(self, command: &mut Command) {
        command
            .args(["-c:v", "libsvtav1", "-preset"])
            .arg(self.preset.as_u8().to_string())
            .arg("-crf")
            .arg(self.crf.as_f64().to_string())
            .args(["-flags", "-cgop", "-pix_fmt", "yuv420p10le"])
            .arg("-svtav1-params")
            .arg(self.svt_av1_params());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fps(value: f64) -> FrameRatePerSecond {
        FrameRatePerSecond::new(value).expect("测试帧率必须合法")
    }

    fn settings(
        user_resolution: Option<Resolution>,
        user_fps: Option<f64>,
        source_resolution: Resolution,
        source_fps: f64,
    ) -> BitrateReduceSettings {
        BitrateReduceSettings::new(
            ResolutionDownScaling::new(user_resolution, source_resolution),
            FrameRateReduce::new(user_fps.map(fps), fps(source_fps)),
        )
    }

    #[test]
    fn keeps_the_source_when_it_already_fits() {
        assert_eq!(
            settings(None, None, Resolution::Fhd, 24.0).video_filter(),
            None,
            "没有可削减项时不应加滤镜"
        );
        assert_eq!(
            settings(Some(Resolution::Uhd), None, Resolution::Fhd, 24.0).video_filter(),
            None,
            "目标比源大时不应放大"
        );

        // 不放大时实际输出仍是源，不能把用户目标当作输出分辨率。
        assert_eq!(
            settings(Some(Resolution::Uhd), None, Resolution::Fhd, 24.0).output_resolution(),
            Resolution::Fhd
        );
        assert_eq!(
            settings(None, None, Resolution::Fhd, 24.0).output_resolution(),
            Resolution::Fhd
        );
        // 真实下采样时取目标。
        assert_eq!(
            settings(Some(Resolution::Fhd), None, Resolution::Uhd, 24.0).output_resolution(),
            Resolution::Fhd
        );
    }

    #[test]
    fn reduces_only_the_dimensions_that_exceed_the_limit() {
        let landscape = settings(Some(Resolution::Fhd), Some(30.0), Resolution::Uhd, 60.0);
        let filter = landscape.video_filter().expect("应生成滤镜");
        assert!(filter.contains("fps=30"), "{filter}");
        assert!(
            filter.contains("scale=1920:1080:force_original_aspect_ratio=decrease"),
            "{filter}"
        );
        assert_eq!(landscape.output_resolution(), Resolution::Fhd);

        let portrait = settings(Some(Resolution::Vfhd), None, Resolution::Vuhd, 24.0);
        let filter = portrait.video_filter().expect("竖屏也要缩放");
        assert!(
            filter.contains("scale=1080:1920:force_original_aspect_ratio=decrease"),
            "{filter}"
        );

        // 跨朝向：横屏源 + 竖屏目标，宽高都必须被约束住。
        let cross = settings(Some(Resolution::Vfhd), None, Resolution::Uhd, 24.0);
        let filter = cross.video_filter().expect("跨朝向也要缩放");
        assert!(
            filter.contains("scale=1080:1920:force_original_aspect_ratio=decrease"),
            "{filter}"
        );

        // 像素更少但单维超限：也必须缩放。
        let thin = settings(
            Some(Resolution::Fhd),
            None,
            Resolution::new(1000, 2000).unwrap(),
            24.0,
        );
        assert!(thin.video_filter().is_some(), "高度 2000 超 1080，必须缩放");

        // 源帧率本就更低时不该降帧。
        assert_eq!(
            settings(None, Some(30.0), Resolution::Fhd, 24.0).video_filter(),
            None
        );
    }
}
