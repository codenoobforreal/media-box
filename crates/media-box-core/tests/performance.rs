//! 进程内性能预算测试：约束那些会放大到外部工作量的次数与规模。
//!
//! 与 `benches/media_box_core.rs` 的分工：
//! - 基准测时间，回答「现在多快」；
//! - 本文件测次数/规模上界，回答「会不会突然变慢」。
//!
//! 硬约束：不启动 ffmpeg / ffprobe 或任何子进程。被测代码只做纯计算；
//! CRF 搜索的测量次数预算另见 `tests/crf_search.rs`。

use std::time::Duration;

use media_box_core::progress::{FfmpegProgressSample, MIN_DELTA_PERCENT, ProgressEmitter};
use media_box_core::scene_detect::SceneCutDetectionRecord;
use media_box_core::scene_sampler::SceneSampler;

/// 进度发射必须按最小百分比变化节流：高频上报不能线性放大 UI 刷新次数。
#[test]
fn progress_emission_is_bounded_by_min_delta() {
    let total = Duration::from_secs(600);
    let mut emitter = ProgressEmitter::new(None, total, MIN_DELTA_PERCENT);
    let mut emitted = 0usize;

    // 每步推进 0.1s，共 6000 步：每步仅增加约 0.017%，远低于 0.1% 阈值。
    for step in 1..=6_000u64 {
        let sample = FfmpegProgressSample::new(step * 100_000);
        if emitter.update(sample, Duration::from_millis(100)).is_some() {
            emitted += 1;
        }
    }

    // 0..100 每 0.1% 一次，加一次收尾 100%：留一格浮点余量。
    assert!(emitted <= 1_001, "发射 {emitted} 次，节流失效：UI 会被刷屏");
    assert!(emitted >= 1, "至少应发射一次进度");
}

/// 任意密度的切点都只能规划出少量短样本：每个样本都意味着一次编码 + 一次 VMAF。
#[test]
fn scene_sampling_bounds_the_search_workload() {
    let records: Vec<SceneCutDetectionRecord> = (0..2_000)
        .map(|index| {
            SceneCutDetectionRecord::new(
                0.1 + f64::from(index % 80) / 100.0,
                f64::from(index) * 1.8,
            )
        })
        .collect();
    let total = Duration::from_secs(3_600);

    let samples = SceneSampler::default().plan_samples(&records, total);

    assert!(!samples.is_empty(), "长视频应有采样点");
    assert!(
        samples.len() <= 3,
        "规划了 {} 个样本，搜索成本失控",
        samples.len()
    );
    assert!(
        samples
            .iter()
            .all(|sample| sample.duration() <= Duration::from_secs(20)),
        "存在超过 20s 的样本：{samples:?}"
    );
    assert!(
        samples
            .iter()
            .all(|sample| sample.start() + sample.duration() <= total),
        "样本越出视频边界：{samples:?}"
    );
}
