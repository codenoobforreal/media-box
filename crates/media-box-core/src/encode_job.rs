//! 正式编码：组装 ffmpeg 命令、解析进度并执行。

use crate::encoder::{Av1EncodeArgs, BitrateReduceSettings};
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
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-v",
            "error",
            "-progress",
            "pipe:1",
            "-dn",
            "-sn",
            "-i",
        ])
        .arg(input);

    args.apply(&mut command);
    command.args(["-c:a", "copy"]);

    if let Some(filter) = settings.video_filter() {
        command.arg("-vf").arg(filter);
    }

    command.arg("-y").arg(output);
    command
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
