//! libvmaf 滤镜图构造：同规格 / 全规格两种测量口径。

use crate::resolution::Resolution;
use crate::value::FrameRatePerSecond;
use std::sync::LazyLock;

/// 构造 VMAF 滤镜图所需的最小视频规格。
///
/// 刻意不依赖 `encoder::BitrateReduceSettings`：滤镜图只关心源/输出分辨率、
/// 源帧率与缩放滤镜串，调用方从自己的设置里取出这四项即可。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoSpec<'a> {
    pub source_resolution: Resolution,
    pub output_resolution: Resolution,
    pub source_fps: FrameRatePerSecond,
    /// 与样本编码同源的滤镜链（`BitrateReduceSettings::video_filter`）。
    pub filter: Option<&'a str>,
}

static VMAF_THREADS: LazyLock<usize> = LazyLock::new(|| {
    std::thread::available_parallelism().map_or(1, |parallelism| parallelism.get())
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum VmafMeasureMode {
    /// 同规格：参考下采样到输出规格，只衡量编码损失。
    SameSpec,
    /// 全规格：失真上采样回原始规格，衡量编码 + 降规格的总损失。
    FullSpec,
}

impl VmafMeasureMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::SameSpec => "同规格",
            Self::FullSpec => "全规格",
        }
    }
}

/// 按分辨率选择 libvmaf 模型名：4K 用 2160 模型，其余用 3d0h。
fn model(resolution: Resolution) -> &'static str {
    if resolution.pixels() >= Resolution::Uhd.pixels() {
        "vmaf_v1.0.16_1d5h_2160"
    } else {
        "vmaf_v1.0.16_3d0h"
    }
}

/// 构造 libvmaf 滤镜图；只取决于视频规格，调用方按模式各构一次即可。
pub fn filter_graph(mode: VmafMeasureMode, spec: VideoSpec<'_>) -> String {
    let threads = *VMAF_THREADS;

    match mode {
        VmafMeasureMode::SameSpec => {
            // 失真帧是样本编码产物，已带 spec.filter；参考帧必须走同一条滤镜链
            // 才能两路同规格。不能拿目标边界分辨率硬拼 scale：
            // 等比缩小时实际输出小于目标，不放大时实际输出等于源。
            let reference_filter = match spec.filter {
                Some(filter) => format!("{filter},"),
                None => String::new(),
            };
            format!(
                "[0:v]setpts=PTS-STARTPTS[dist];\
                 [1:v]{reference_filter}setpts=PTS-STARTPTS[ref];\
                 [dist][ref]libvmaf=model=version={model}:n_threads={threads}",
                model = model(spec.output_resolution),
            )
        }
        VmafMeasureMode::FullSpec => {
            let resolution = spec.source_resolution;
            format!(
                "[0:v]scale=w={w}:h={h}:flags=lanczos,fps={fps},setpts=PTS-STARTPTS[dist];\
                 [1:v]setpts=PTS-STARTPTS[ref];\
                 [dist][ref]libvmaf=model=version={model}:n_threads={threads}",
                w = resolution.width(),
                h = resolution.height(),
                fps = spec.source_fps.as_f64(),
                model = model(resolution),
            )
        }
    }
}

/// 取最后一条 `VMAF score:`：`-v info` 下它是整段测量的最终值。
pub fn parse_score(stderr_lines: &[String]) -> Option<f64> {
    stderr_lines.iter().rev().find_map(|line| {
        let (_, score) = line.split_once("VMAF score:")?;
        score
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|score| score.is_finite())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const DOWNSCALED: &str = "fps=30,scale=1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2:flags=lanczos";

    fn reduced_spec() -> VideoSpec<'static> {
        // 4K60 → 1080p30：源与输出不同，滤镜链非空。
        VideoSpec {
            source_resolution: Resolution::Uhd,
            output_resolution: Resolution::Fhd,
            source_fps: FrameRatePerSecond::new(60.0).unwrap(),
            filter: Some(DOWNSCALED),
        }
    }

    #[test]
    fn the_two_modes_measure_different_resolutions() {
        // 同规格：参考帧走与样本编码相同的滤镜链（先 fps 后等比 scale），
        // 不再拿目标边界分辨率硬拼 scale；输出规格用 3d0h 模型。
        let spec = reduced_spec();
        let same_spec = filter_graph(VmafMeasureMode::SameSpec, spec);
        assert!(
            same_spec.contains(&format!("[1:v]{DOWNSCALED},setpts=PTS-STARTPTS[ref]")),
            "{same_spec}"
        );
        assert!(!same_spec.contains("scale=w="), "{same_spec}");
        assert!(same_spec.contains("vmaf_v1.0.16_3d0h"), "{same_spec}");

        // 全规格：失真帧上采样回源规格，参考帧原样。
        let full_spec = filter_graph(VmafMeasureMode::FullSpec, spec);
        assert!(
            full_spec.contains("[0:v]scale=w=3840:h=2160"),
            "{full_spec}"
        );
        assert!(
            full_spec.contains("[1:v]setpts=PTS-STARTPTS[ref]"),
            "{full_spec}"
        );
    }

    #[test]
    fn same_spec_keeps_the_reference_untouched_when_nothing_is_reduced() {
        // 源小于用户目标时不放大：滤镜链为空，两路都不加 scale/fps。
        let spec = VideoSpec {
            source_resolution: Resolution::Hd,
            output_resolution: Resolution::Hd,
            source_fps: FrameRatePerSecond::new(24.0).unwrap(),
            filter: None,
        };
        let graph = filter_graph(VmafMeasureMode::SameSpec, spec);
        assert!(graph.contains("[1:v]setpts=PTS-STARTPTS[ref]"), "{graph}");
        assert!(!graph.contains("scale="), "{graph}");
        assert!(!graph.contains("fps="), "{graph}");
    }

    #[test]
    fn takes_the_last_score() {
        let lines = [
            "VMAF score: 88.50".to_owned(),
            "VMAF score: 92.25".to_owned(),
        ];
        assert_eq!(parse_score(&lines), Some(92.25));
        assert_eq!(parse_score(&["没有分数".to_owned()]), None);
        assert_eq!(parse_score(&["VMAF score: NaN".to_owned()]), None);
        assert_eq!(parse_score(&["VMAF score: inf".to_owned()]), None);
    }
}
