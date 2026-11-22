use crate::scene_detect::SceneCutDetectionRecord;
use anyhow::{Result, bail};
use std::cmp;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
struct SceneTimeSpan {
    start_time: Duration,
    end_time: Duration,
}

impl SceneTimeSpan {
    fn duration(&self) -> Duration {
        self.end_time.saturating_sub(self.start_time)
    }
}

#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub struct SceneSample {
    start: Duration,
    duration: Duration,
}

impl SceneSample {
    pub fn new(start: Duration, duration: Duration) -> Self {
        Self { start, duration }
    }

    pub fn start(self) -> Duration {
        self.start
    }

    pub fn duration(self) -> Duration {
        self.duration
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneSampler {
    minimum_scene_duration: Duration,
    edge_exclusion_ratio: f64,
    scene_cut_merge_gap_seconds: f64,
    maximum_sample_duration: Duration,
    maximum_sample_count: u8,
}

impl Default for SceneSampler {
    fn default() -> Self {
        Self {
            minimum_scene_duration: Duration::from_secs_f64(2.0),
            edge_exclusion_ratio: 0.05,
            scene_cut_merge_gap_seconds: 1.0,
            maximum_sample_duration: Duration::from_secs_f64(20.0),
            maximum_sample_count: 3,
        }
    }
}

impl SceneSampler {
    /// 校验配置组合的合法性；只依赖常量，任务开始时调用一次即可。
    pub fn validate(&self) -> Result<()> {
        if self.edge_exclusion_ratio >= 0.5 {
            bail!("边缘排除比例 ≥ 0.5，首尾禁采样区重叠，无法采样")
        }

        let first_fallback_ratio = 1.0 / (self.maximum_sample_count as f64 + 1.0);

        if self.edge_exclusion_ratio >= first_fallback_ratio {
            bail!("边缘排除比例过大：按固定比例采样的起点会落入片头禁采样区")
        }

        Ok(())
    }

    pub fn needs_scene_detection(&self, total_video_duration: Duration) -> bool {
        total_video_duration > self.short_video_threshold()
    }

    pub fn fallback_samples(&self, total_video_duration: Duration) -> Vec<SceneSample> {
        if total_video_duration <= self.short_video_threshold() {
            self.single_fallback_sample(total_video_duration)
        } else {
            self.multi_fallback_samples(total_video_duration)
        }
    }

    pub fn plan_samples(
        &self,
        records: &[SceneCutDetectionRecord],
        total_video_duration: Duration,
    ) -> Vec<SceneSample> {
        let merged = self.merge_close_scene_cuts(records);
        if merged.is_empty() {
            return self.multi_fallback_samples(total_video_duration);
        }

        let spans = self.convert_scene_cuts_to_time_spans(&merged, total_video_duration);
        let mut filtered = self.filter_by_duration_and_edge_exclusion(&spans, total_video_duration);
        if filtered.is_empty() {
            return self.multi_fallback_samples(total_video_duration);
        }

        filtered.sort_by_key(|span| cmp::Reverse(span.duration()));
        let take = usize::from(self.maximum_sample_count).min(filtered.len());

        self.extract_samples_from_time_spans(&filtered[..take])
    }

    /// 前提：`validate()` 已通过，否则除数 `tail_bound_constraint` 可能为 0。
    fn short_video_threshold(&self) -> Duration {
        let tail_bound_constraint =
            1.0 - self.edge_exclusion_ratio * (self.maximum_sample_count as f64 + 1.0);

        (self
            .maximum_sample_duration
            .mul_f64(self.maximum_sample_count as f64 + 1.0))
        .div_f64(tail_bound_constraint)
    }

    fn single_fallback_sample(&self, total_video_duration: Duration) -> Vec<SceneSample> {
        // 元数据缺 duration 时 total 为 0：没有可采样的内容，直接空表，
        // 避免 crf_probe 对 `-t 0` 反复编码失败刷警告。
        if total_video_duration.is_zero() {
            return Vec::new();
        }

        // 短视频不足一个采样窗口时取整段，且永不越过 0。
        let sample_duration = self.maximum_sample_duration.min(total_video_duration);
        let start_time = total_video_duration.saturating_sub(sample_duration) / 2;

        vec![SceneSample::new(start_time, sample_duration)]
    }

    fn multi_fallback_samples(&self, total_video_duration: Duration) -> Vec<SceneSample> {
        let count = self.maximum_sample_count;
        let denominator = f64::from(count) + 1.0;
        let maximum_allowed_start_time =
            total_video_duration.mul_f64(1.0 - self.edge_exclusion_ratio);

        (1..=count)
            .filter_map(|step| {
                let start = total_video_duration
                    .mul_f64(f64::from(step) / denominator)
                    .min(maximum_allowed_start_time);
                let remaining = total_video_duration.saturating_sub(start);

                (!remaining.is_zero())
                    .then(|| SceneSample::new(start, self.maximum_sample_duration.min(remaining)))
            })
            .collect()
    }

    fn merge_close_scene_cuts(
        &self,
        records: &[SceneCutDetectionRecord],
    ) -> Vec<SceneCutDetectionRecord> {
        if records.is_empty() {
            return Vec::new();
        }

        // 前提：records 按时间升序（scdet 按帧序输出）；乱序输入的合并结果未定义。
        let mut merged_records = Vec::new();
        let mut current_best_record = records[0];

        for window in records.windows(2) {
            let previous_record = window[0];
            let current_record = window[1];

            if current_record.scene_change_time_seconds()
                - previous_record.scene_change_time_seconds()
                < self.scene_cut_merge_gap_seconds
            {
                if current_record.scene_change_score() > current_best_record.scene_change_score() {
                    current_best_record = current_record;
                }
            } else {
                merged_records.push(current_best_record);
                current_best_record = current_record;
            }
        }
        merged_records.push(current_best_record);
        merged_records
    }

    fn convert_scene_cuts_to_time_spans(
        &self,
        records: &[SceneCutDetectionRecord],
        total_video_duration: Duration,
    ) -> Vec<SceneTimeSpan> {
        let cut_time = |record: &SceneCutDetectionRecord| {
            Duration::try_from_secs_f64(record.scene_change_time_seconds())
                .unwrap_or(Duration::ZERO)
                .min(total_video_duration)
        };

        let mut time_spans = Vec::with_capacity(records.len() + 1);

        time_spans.push(SceneTimeSpan {
            start_time: Duration::ZERO,
            end_time: cut_time(&records[0]),
        });

        for window in records.windows(2) {
            time_spans.push(SceneTimeSpan {
                start_time: cut_time(&window[0]),
                end_time: cut_time(&window[1]),
            });
        }

        if let Some(last_record) = records.last() {
            time_spans.push(SceneTimeSpan {
                start_time: cut_time(last_record),
                end_time: total_video_duration,
            });
        }

        time_spans
    }

    fn filter_by_duration_and_edge_exclusion(
        &self,
        time_spans: &[SceneTimeSpan],
        total_video_duration: Duration,
    ) -> Vec<SceneTimeSpan> {
        let minimum_allowed_start_time = total_video_duration.mul_f64(self.edge_exclusion_ratio);
        let maximum_allowed_end_time =
            total_video_duration.mul_f64(1.0 - self.edge_exclusion_ratio);

        time_spans
            .iter()
            .filter(|span| {
                span.duration() > self.minimum_scene_duration
                    && span.start_time >= minimum_allowed_start_time
                    && span.end_time <= maximum_allowed_end_time
            })
            .copied()
            .collect()
    }

    fn extract_samples_from_time_spans(&self, time_spans: &[SceneTimeSpan]) -> Vec<SceneSample> {
        time_spans
            .iter()
            .map(|time_span| {
                let scene_duration = time_span.duration();
                let duration = self.maximum_sample_duration.min(scene_duration);

                SceneSample::new(
                    time_span.start_time + (scene_duration - duration) / 2,
                    duration,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(time: f64) -> SceneCutDetectionRecord {
        SceneCutDetectionRecord::new(0.5, time)
    }

    #[test]
    fn every_sample_fits_inside_the_video() {
        // 短视频：曾经在 total/2 - max/2 处下溢 panic。
        let short = SceneSampler::default().fallback_samples(Duration::from_secs(5));
        assert!(!short.is_empty());
        assert!(
            short
                .iter()
                .all(|sample| sample.start() + sample.duration() <= Duration::from_secs(5))
        );

        // 切点晚于总时长（duration 被截断到整秒）：不能 panic，也不能越界。
        let total = Duration::from_secs(60);
        let beyond = SceneSampler::default().plan_samples(&[record(60.5)], total);
        assert!(
            beyond
                .iter()
                .all(|sample| sample.start() + sample.duration() <= total)
        );
    }

    #[test]
    fn prefers_the_longest_interior_scenes() {
        let total = Duration::from_secs(60);
        let samples = SceneSampler::default()
            .plan_samples(&[record(10.0), record(15.0), record(40.0)], total);

        assert!(!samples.is_empty());
        assert!(samples.len() <= 3, "最多 3 个样本");
        assert!(
            samples
                .iter()
                .all(|sample| sample.start() + sample.duration() <= total)
        );
        // 最长场景 [15, 40]（25s）取中点，样本 20s：15 + (25-20)/2 = 17.5s。
        // 首尾被 5% 排除的 [0,10]、[40,60] 不会产生样本，因此首样本必来自该场景。
        assert_eq!(
            samples[0].start(),
            Duration::from_millis(17_500),
            "{samples:?}"
        );
        assert_eq!(samples[0].duration(), Duration::from_secs(20));
    }
}
