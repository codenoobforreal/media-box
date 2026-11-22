use crate::encoder::{Av1EncodeArgs, BitrateReduceSettings};
use crate::interrupter::Canceller;
use crate::process::{self, Stream};
use crate::progress::{
    FfmpegProgressLineParser, MIN_DELTA_PERCENT, ProgressEmitter, ProgressSnapshot,
};
use crate::report::Reporter;
use anyhow::{Result, bail};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

const PROGRESS_PUBLISH_INTERVAL: Duration = Duration::from_secs(1);

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

pub fn run(
    command: Command,
    output_path: &Path,
    canceller: &Canceller,
    reporter: &Reporter,
    started_at: Instant,
    total_duration: Duration,
) -> Result<Option<u64>> {
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
            Ok(parser.total_size_bytes())
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
