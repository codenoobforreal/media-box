use crate::encoder::BitrateReduceSettings;
use crate::resolution::Resolution;
use std::sync::LazyLock;

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

fn model(resolution: Resolution) -> &'static str {
    if resolution.pixels() >= Resolution::Uhd.pixels() {
        "vmaf_v1.0.16_1d5h_2160"
    } else {
        "vmaf_v1.0.16_3d0h"
    }
}

/// 构造 libvmaf 滤镜图；只取决于设置，调用方按模式各构一次即可。
pub fn filter_graph(mode: VmafMeasureMode, settings: BitrateReduceSettings) -> String {
    let threads = *VMAF_THREADS;

    match mode {
        VmafMeasureMode::SameSpec => {
            let resolution = settings.output_resolution();
            format!(
                "[0:v]setpts=PTS-STARTPTS[dist];\
                 [1:v]scale=w={w}:h={h}:flags=lanczos,fps={fps},setpts=PTS-STARTPTS[ref];\
                 [dist][ref]libvmaf=model=version={model}:n_threads={threads}",
                w = resolution.width(),
                h = resolution.height(),
                fps = settings.target_fps().as_f64(),
                model = model(resolution),
            )
        }
        VmafMeasureMode::FullSpec => {
            let resolution = settings.input_resolution();
            format!(
                "[0:v]scale=w={w}:h={h}:flags=lanczos,fps={fps},setpts=PTS-STARTPTS[dist];\
                 [1:v]setpts=PTS-STARTPTS[ref];\
                 [dist][ref]libvmaf=model=version={model}:n_threads={threads}",
                w = resolution.width(),
                h = resolution.height(),
                fps = settings.input_fps().as_f64(),
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
    use crate::encoder::{BitrateReduceSettings, FrameRateReduce, ResolutionDownScaling};
    use crate::value::FrameRatePerSecond;

    fn settings() -> BitrateReduceSettings {
        BitrateReduceSettings::new(
            ResolutionDownScaling::new(Some(Resolution::Fhd), Resolution::Uhd),
            FrameRateReduce::new(
                Some(FrameRatePerSecond::new(30.0).unwrap()),
                FrameRatePerSecond::new(60.0).unwrap(),
            ),
        )
    }

    #[test]
    fn the_two_modes_measure_different_resolutions() {
        // 同规格：参考帧缩到输出规格；4K 源要用 4K 模型。
        let same_spec = filter_graph(VmafMeasureMode::SameSpec, settings());
        assert!(
            same_spec.contains("[1:v]scale=w=1920:h=1080"),
            "{same_spec}"
        );
        assert!(same_spec.contains("vmaf_v1.0.16_3d0h"), "{same_spec}");

        // 全规格：失真帧上采样回源规格，参考帧原样。
        let full_spec = filter_graph(VmafMeasureMode::FullSpec, settings());
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
