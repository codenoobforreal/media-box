use crate::byte_size::ByteSize;
use crate::interrupter::Canceller;
use crate::process::{self, Stream};
use crate::resolution::Resolution;
use crate::value::FrameRatePerSecond;
use anyhow::{Result, anyhow, bail};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceMeta {
    resolution: Resolution,
    fps: FrameRatePerSecond,
    duration: Duration,
    size: ByteSize,
}

impl SourceMeta {
    pub fn new(
        resolution: Resolution,
        fps: FrameRatePerSecond,
        duration: Duration,
        size: ByteSize,
    ) -> Self {
        Self {
            resolution,
            fps,
            duration,
            size,
        }
    }

    pub fn resolution(self) -> Resolution {
        self.resolution
    }

    pub fn fps(self) -> FrameRatePerSecond {
        self.fps
    }

    pub fn duration(self) -> Duration {
        self.duration
    }

    pub fn size(self) -> ByteSize {
        self.size
    }
}

pub fn read_metadata(path: &Path, canceller: &Canceller) -> Result<SourceMeta> {
    let mut raw = ProbeRaw::default();
    let output = process::run(probe_command(path), canceller, |stream, line| {
        if stream == Stream::Stdout {
            raw.absorb(line);
        }
    })?;

    if !output.success() {
        bail!("读取视频元数据失败：{}", output.failure_message());
    }

    raw.finish()
}

fn probe_command(path: &Path) -> Command {
    let mut command = Command::new("ffprobe");
    command
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate,avg_frame_rate",
            "-show_entries",
            "format=duration,size",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(path);
    command
}

#[derive(Debug, Default)]
struct ProbeRaw {
    width: Option<u16>,
    height: Option<u16>,
    avg_frame_rate: Option<f64>,
    avg_frame_rate_error: Option<String>,
    r_frame_rate: Option<f64>,
    r_frame_rate_error: Option<String>,
    duration_secs: Option<f64>,
    size_bytes: Option<u64>,
}

impl ProbeRaw {
    fn absorb(&mut self, line: &str) {
        let Some((key, value)) = line.trim().split_once('=') else {
            return;
        };
        let (key, value) = (key.trim(), value.trim());

        match key {
            "width" => self.width = value.parse().ok(),
            "height" => self.height = value.parse().ok(),
            "avg_frame_rate" => match parse_rate(value) {
                Ok(rate) => self.avg_frame_rate = Some(rate),
                Err(message) => self.avg_frame_rate_error = Some(message),
            },
            "r_frame_rate" => match parse_rate(value) {
                Ok(rate) => self.r_frame_rate = Some(rate),
                Err(message) => self.r_frame_rate_error = Some(message),
            },
            "duration" => {
                self.duration_secs = value
                    .parse::<f64>()
                    .ok()
                    .filter(|secs| secs.is_finite() && *secs >= 0.0);
            }
            "size" => self.size_bytes = value.parse().ok(),
            _ => {}
        }
    }

    fn finish(self) -> Result<SourceMeta> {
        let width = self.width.ok_or_else(|| anyhow!("缺少字段 width"))?;
        let height = self.height.ok_or_else(|| anyhow!("缺少字段 height"))?;
        let size = self.size_bytes.ok_or_else(|| anyhow!("缺少字段 size"))?;
        let fps = self.fps()?;

        Ok(SourceMeta::new(
            Resolution::new(width, height)?,
            FrameRatePerSecond::from_probed(fps),
            self.duration_secs
                .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
                .unwrap_or(Duration::ZERO),
            ByteSize::b(size),
        ))
    }

    fn fps(&self) -> Result<f64> {
        if let Some(rate) = self.avg_frame_rate {
            return Ok(rate);
        }
        if let Some(rate) = self.r_frame_rate {
            return Ok(rate);
        }

        bail!(
            "读取帧率失败：avg_frame_rate 不可用（{}）；r_frame_rate 也不可用（{}）",
            self.avg_frame_rate_error.as_deref().unwrap_or("缺少字段"),
            self.r_frame_rate_error.as_deref().unwrap_or("缺少字段"),
        )
    }
}

/// 解析 ffprobe 的帧率字符串，如 `30000/1001` 或 `30`。
fn parse_rate(text: &str) -> std::result::Result<f64, String> {
    let (numerator, denominator) = text.split_once('/').unwrap_or((text, "1"));

    let parsed = || -> Option<f64> {
        let numerator: f64 = numerator.trim().parse().ok()?;
        let denominator: f64 = denominator.trim().parse().ok()?;
        (denominator != 0.0).then(|| numerator / denominator)
    };

    match parsed() {
        Some(rate) if rate.is_finite() && rate > 0.0 => Ok(rate),
        _ => Err(format!("无法从 {text} 得到有限正数")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_from(lines: &[&str]) -> ProbeRaw {
        let mut raw = ProbeRaw::default();
        for line in lines {
            raw.absorb(line);
        }
        raw
    }

    #[test]
    fn parses_ffprobe_output() {
        // 分数帧率 + avg 优先 + duration 截到整秒。
        let meta = raw_from(&[
            "width=1920",
            "height=1080",
            "avg_frame_rate=24000/1001",
            "r_frame_rate=24",
            "duration=12.5",
            "size=1048576",
        ])
        .finish()
        .expect("字段齐全应解析成功");

        assert_eq!(meta.resolution, Resolution::Fhd);
        assert!((meta.fps.as_f64() - 23.976).abs() < 0.01);
        assert_eq!(meta.duration, Duration::from_secs_f64(12.5));

        // avg 为 0/0 时退回 r_frame_rate。
        let meta = raw_from(&[
            "width=1280",
            "height=720",
            "avg_frame_rate=0/0",
            "r_frame_rate=60",
            "size=1",
        ])
        .finish()
        .expect("应退回 r_frame_rate");
        assert_eq!(meta.fps.as_f64(), 60.0);

        // 缺字段要报错，不能给出 0 分辨率的元数据。
        assert!(raw_from(&["width=1280", "height=720"]).finish().is_err());

        // 亚秒素材必须保留精度：截断到 0 会让采样阶段拿到 -t 0。
        let meta = raw_from(&[
            "width=640",
            "height=360",
            "avg_frame_rate=30",
            "duration=0.5",
            "size=1",
        ])
        .finish()
        .expect("应解析成功");
        assert_eq!(meta.duration, Duration::from_millis(500));
    }
}
