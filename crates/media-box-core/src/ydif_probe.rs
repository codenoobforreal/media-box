//! 用 signalstats 探测样本的 YDIF 值序列。

use crate::ffmpeg_args::FfmpegCommand;
use crate::interrupter::Canceller;
use crate::process;
use crate::ydif_stats::YdifValueSeries;
use anyhow::{Result, bail};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

/// 对指定时间窗执行 signalstats YDIF 探测。
pub struct YdifProbe {
    input_video_path: PathBuf,
    sample_window_start_time: Duration,
    sample_window_duration: Duration,
}

impl YdifProbe {
    /// 构造探测器，限定采样窗口。
    pub fn new(
        input_video_path: PathBuf,
        sample_window_start_time: Duration,
        sample_window_duration: Duration,
    ) -> Self {
        Self {
            input_video_path,
            sample_window_start_time,
            sample_window_duration,
        }
    }

    /// 组装 signalstats 探测命令。
    pub fn build_ffmpeg_command(&self) -> Command {
        FfmpegCommand::new()
            .quiet()
            .no_stats()
            .seek(self.sample_window_start_time)
            .duration(self.sample_window_duration)
            .no_audio_subtitle_data()
            .input(&self.input_video_path)
            .video_filter("signalstats,metadata=print:key=lavfi.signalstats.YDIF:file=-")
            .null_output()
            .build()
    }

    /// 运行探测并返回 YDIF 值序列。
    pub fn run(&self, canceller: &Canceller) -> Result<YdifValueSeries> {
        let mut ydif_values = Vec::new();

        let output = process::run(self.build_ffmpeg_command(), canceller, |stream, line| {
            if stream == process::Stream::Stdout
                && let Some(value) = parse_ydif_value_from_line(line)
            {
                ydif_values.push(value);
            }
        })?;

        if !output.success() {
            bail!("信号分析失败：{}", output.failure_message());
        }

        Ok(YdifValueSeries::from_values(ydif_values))
    }
}

/// 从一行 ffmpeg 日志中解析 YDIF 数值。
///
/// 格式示例：
/// ```text
/// frame:388  pts:155200  pts_time:12.94628
/// lavfi.signalstats.YDIF=2.87908
/// ```
pub fn parse_ydif_value_from_line(line: &str) -> Option<f64> {
    match line.rsplit_once('=') {
        Some(("lavfi.signalstats.YDIF", value_text)) => value_text.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_keeps_seek_before_input() {
        let probe = YdifProbe::new(
            PathBuf::from("in.mkv"),
            Duration::from_millis(500),
            Duration::from_secs(2),
        );
        let args: Vec<String> = probe
            .build_ffmpeg_command()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        // -ss/-t 必须在 -i 之前：这是输入定位，移到输出侧会改变采样窗口。
        assert_eq!(
            args,
            [
                "-hide_banner",
                "-v",
                "error",
                "-nostats",
                "-ss",
                "0.5",
                "-t",
                "2",
                "-an",
                "-sn",
                "-dn",
                "-i",
                "in.mkv",
                "-vf",
                "signalstats,metadata=print:key=lavfi.signalstats.YDIF:file=-",
                "-f",
                "null",
                "-",
            ]
        );
    }

    #[test]
    fn parses_ydif_value() {
        assert_eq!(
            parse_ydif_value_from_line("lavfi.signalstats.YDIF=2.87908"),
            Some(2.87908)
        );
        assert_eq!(
            parse_ydif_value_from_line("lavfi.signalstats.YDIF=bad"),
            None
        );
        assert_eq!(parse_ydif_value_from_line("frame=12"), None);
    }
}
