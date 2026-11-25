//! 解析 ffmpeg -progress 输出，计算百分比与 ETA，并按最小变化量节流。

use std::fmt;
use std::time::Duration;

use crate::format::format_duration_fixed;

/// 进度百分比的最小变化量；低于它的更新不发射，避免刷屏。
pub const MIN_DELTA_PERCENT: f32 = 0.1;

/// 一次进度快照：完成百分比、已用时间与预计剩余时间。
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ProgressSnapshot {
    percentage: f32,
    elapsed: Duration,
    eta: Option<Duration>,
}

impl ProgressSnapshot {
    /// 构造进度快照。
    pub fn new(percentage: f32, elapsed: Duration, eta: Option<Duration>) -> Self {
        Self {
            percentage,
            elapsed,
            eta,
        }
    }

    /// 相对上一帧的变化是否达到发射阈值。
    ///
    /// 首次更新、百分比越过 0 / 100 边界时总是发射；非法值一律不发射。
    pub fn should_emit(&self, previous: &Option<Self>, min_delta_percentage: f32) -> bool {
        if !self.percentage.is_finite()
            || !min_delta_percentage.is_finite()
            || min_delta_percentage < 0.0
        {
            return false;
        }

        let Some(previous) = previous else {
            return true; // 第一次更新必须允许
        };

        let current_percentage = self.percentage.clamp(0.0, 100.0);
        let prev_percentage = previous.percentage.clamp(0.0, 100.0);

        if current_percentage <= 0.0 {
            return prev_percentage > 0.0;
        }

        if current_percentage >= 100.0 {
            return prev_percentage < 100.0;
        }

        current_percentage > prev_percentage
            && (current_percentage - prev_percentage) >= min_delta_percentage
    }

    /// 完成百分比（0.0~100.0）。
    pub fn percentage(&self) -> f32 {
        self.percentage
    }

    /// 已用时间。
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// 预计剩余时间，无法估算时为 None。
    pub fn eta(&self) -> Option<Duration> {
        self.eta
    }
}

impl fmt::Display for ProgressSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let percentage_str = format!("{:>5.1}%", self.percentage());
        let eta_str = match self.eta() {
            Some(eta) => format_duration_fixed(eta),
            None => "--:--:--:--".to_string(),
        };

        write!(
            f,
            "[{}] {} | 剩余: {}",
            percentage_str,
            format_duration_fixed(self.elapsed()),
            eta_str
        )
    }
}

/// 按最小变化量节流的进度发射器，避免百分比小幅抖动导致刷屏。
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct ProgressEmitter {
    last_emitted: Option<ProgressSnapshot>,
    total_duration: Duration,
    min_delta_percent: f32,
}

impl ProgressEmitter {
    /// 构造发射器；`last_emitted` 用于延续上一次已发射的进度。
    pub fn new(
        last_emitted: Option<ProgressSnapshot>,
        total_duration: Duration,
        min_delta_percent: f32,
    ) -> Self {
        Self {
            last_emitted,
            total_duration,
            min_delta_percent,
        }
    }

    /// 用最新样本更新并返回需要发射的快照；无需发射时返回 None。
    pub fn update(
        &mut self,
        sample: FfmpegProgressSample,
        elapsed: Duration,
    ) -> Option<ProgressSnapshot> {
        let snapshot = progress_sample_to_snapshot(sample, self.total_duration, elapsed);

        if snapshot.should_emit(&self.last_emitted, self.min_delta_percent) {
            self.last_emitted = Some(snapshot);
            Some(snapshot)
        } else {
            None
        }
    }
}

/// 增量解析 ffmpeg -progress 输出的解析器，在遇到 progress 行时产出样本。
#[derive(Debug, Default)]
pub struct FfmpegProgressLineParser {
    out_time_ms: Option<u64>,
    total_size_bytes: Option<u64>,
}

impl FfmpegProgressLineParser {
    /// 喂入一行；返回 Some 表示该行结束了一条进度样本。
    pub fn push_line(&mut self, line: &str) -> Option<FfmpegProgressSample> {
        let trim_line = line.trim();

        if trim_line.is_empty() {
            return None;
        }

        let (key, value) = match trim_line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => return None,
        };

        if value.is_empty() || value == "N/A" {
            return None;
        }

        match key {
            "total_size" => {
                if let Ok(v) = value.parse::<u64>() {
                    self.total_size_bytes = Some(v);
                }
                None
            }
            "out_time_ms" => {
                if let Ok(v) = value.parse::<u64>() {
                    self.out_time_ms = Some(v);
                }
                None
            }
            "progress" => {
                let out_time_ms = self.out_time_ms.take();
                out_time_ms.map(FfmpegProgressSample::new)
            }
            _ => None,
        }
    }

    /// 最近一次读到的 `total_size`（字节）。
    pub fn total_size_bytes(&self) -> Option<u64> {
        self.total_size_bytes
    }
}

/// 一条 ffmpeg 进度样本，内部把外部的 `out_time_ms` 按微秒解释。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FfmpegProgressSample {
    /// 外部 ffmpeg 的 key 是 `out_time_ms`，但取值实际按微秒解释，这里保持同一口径。
    out_time_us: u64,
}

impl FfmpegProgressSample {
    /// 由已换算的微秒时间构造样本。
    pub fn new(out_time_us: u64) -> Self {
        Self { out_time_us }
    }

    /// 完成百分比；`total_duration` 为 0 时返回 0.0。
    pub fn percentage(&self, total_duration: Duration) -> f32 {
        if total_duration.is_zero() {
            return 0.0;
        }

        let ratio = self.out_time_duration().div_duration_f32(total_duration);
        let p = ratio * 100.0;

        if p.is_finite() {
            p.clamp(0.0, 100.0)
        } else {
            debug_assert!(false, "进度百分比为无穷大或 NaN");
            0.0
        }
    }

    /// 按平均速度估算剩余时间；无法估算时为 None。
    pub fn eta(&self, total_duration: Duration, elapsed: Duration) -> Option<Duration> {
        if total_duration.is_zero() {
            return None;
        }

        if self.percentage(total_duration) >= 100.0 {
            return Some(Duration::ZERO);
        }

        let avg_speed = self.average_speed_x(elapsed);
        (avg_speed.is_finite() && avg_speed > 0.0).then(|| {
            total_duration
                .saturating_sub(self.out_time_duration())
                .div_f32(avg_speed)
        })
    }

    fn average_speed_x(self, elapsed: Duration) -> f32 {
        if elapsed.is_zero() {
            0.0
        } else {
            self.out_time_duration().div_duration_f32(elapsed)
        }
    }

    fn out_time_duration(self) -> Duration {
        Duration::from_micros(self.out_time_us)
    }
}

fn progress_sample_to_snapshot(
    sample: FfmpegProgressSample,
    total_duration: Duration,
    elapsed: Duration,
) -> ProgressSnapshot {
    ProgressSnapshot::new(
        sample.percentage(total_duration),
        elapsed,
        sample.eta(total_duration, elapsed),
    )
}
