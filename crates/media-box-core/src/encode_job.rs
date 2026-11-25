//! 正式编码：组装 ffmpeg 命令、解析进度并执行。

use crate::encoder::{Av1EncodeArgs, BitrateReduceSettings};
use crate::ffmpeg_args::FfmpegCommand;
use crate::interrupter::Canceller;
use crate::process::{self, Stream};
use crate::progress::{
    FfmpegProgressLineParser, MIN_DELTA_PERCENT, ProgressEmitter, ProgressSnapshot,
};
use crate::report::Reporter;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

const PROGRESS_PUBLISH_INTERVAL: Duration = Duration::from_secs(1);

/// 组装正式编码的 ffmpeg 命令（含 -progress pipe:1 与降码滤镜）。
pub fn build_command(
    input: &Path,
    output: &Path,
    args: Av1EncodeArgs,
    settings: BitrateReduceSettings,
) -> Command {
    let mut builder = FfmpegCommand::new()
        .quiet()
        .raw(&["-progress", "pipe:1"])
        .no_data_subtitle()
        .input(input);

    args.apply(builder.as_command_mut());
    let builder = builder.raw(&["-c:a", "copy"]);

    let builder = match settings.video_filter() {
        Some(filter) => builder.video_filter(&filter),
        None => builder,
    };

    builder.raw(&["-y", &output.to_string_lossy()]).build()
}

/// 执行编码并解析进度；失败或取消时删除半成品输出，成功返回输出字节数。
///
/// 优先用 `total_size` 进度行；ffmpeg 未上报时回退到输出文件的 metadata，
/// 避免成功编码却因缺少进度字段而丢失成功报告。
pub fn run(
    command: Command,
    output_path: &Path,
    canceller: &Canceller,
    reporter: &Reporter,
    started_at: Instant,
    total_duration: Duration,
) -> Result<u64> {
    let mut parser = FfmpegProgressLineParser::default();
    let mut emitter = ProgressEmitter::new(None, total_duration, MIN_DELTA_PERCENT);
    let mut last_published_at = Instant::now();

    let result = process::run(command, canceller, |stream, line| {
        if stream != Stream::Stdout {
            return;
        }

        let Some(sample) = parser.push_line(line) else {
            return;
        };

        if last_published_at.elapsed() < PROGRESS_PUBLISH_INTERVAL {
            return;
        }

        if let Some(progress) = emitter.update(sample, started_at.elapsed()) {
            reporter.progress(progress);
            last_published_at = Instant::now();
        }
    });

    match result {
        Ok(output) if output.success() => {
            reporter.progress(ProgressSnapshot::new(
                100.0,
                started_at.elapsed(),
                Some(Duration::ZERO),
            ));
            match parser.total_size_bytes() {
                Some(bytes) => Ok(bytes),
                None => Ok(fs::metadata(output_path)
                    .with_context(|| format!("读取输出文件体积 {}", output_path.display()))?
                    .len()),
            }
        }
        Ok(output) => {
            let _ = fs::remove_file(output_path);
            bail!("压缩失败：{}", output.failure_message())
        }
        Err(error) => {
            let _ = fs::remove_file(output_path);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::{BitrateReduceSettings, FrameRateReduce, ResolutionDownScaling};
    use crate::resolution::Resolution;
    use crate::value::{ConstantRateFactor, EncoderPreset, FilmGrain, FrameRatePerSecond};

    #[test]
    fn command_keeps_progress_and_filter_order() {
        let settings = BitrateReduceSettings::new(
            ResolutionDownScaling::new(Some(Resolution::Fhd), Resolution::Uhd),
            FrameRateReduce::new(
                Some(FrameRatePerSecond::new(30.0).unwrap()),
                FrameRatePerSecond::new(60.0).unwrap(),
            ),
        );
        let args = Av1EncodeArgs::new(
            EncoderPreset::new(6).unwrap(),
            ConstantRateFactor::new(28.0).unwrap(),
            FilmGrain::new(5).unwrap(),
        );
        let command = build_command(Path::new("in.mkv"), Path::new("out.mp4"), args, settings);
        let got: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            got,
            [
                "-hide_banner",
                "-v",
                "error",
                "-progress",
                "pipe:1",
                "-dn",
                "-sn",
                "-i",
                "in.mkv",
                "-c:v",
                "libsvtav1",
                "-preset",
                "6",
                "-crf",
                "28",
                "-flags",
                "-cgop",
                "-pix_fmt",
                "yuv420p10le",
                "-svtav1-params",
                "tune=5:keyint=-1:qp-scale-compress-strength=1:film-grain=5",
                "-c:a",
                "copy",
                "-vf",
                "fps=30,scale=1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2:flags=lanczos",
                "-y",
                "out.mp4",
            ]
        );
    }
}
