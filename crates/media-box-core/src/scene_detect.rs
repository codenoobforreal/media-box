use crate::interrupter::Canceller;
use crate::process;
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, PartialEq, PartialOrd, Clone, Copy)]
pub struct SceneCutDetectionRecord {
    pub scene_change_score: f64,
    pub scene_change_time_seconds: f64,
}

#[derive(Debug, PartialEq, Clone, Eq, PartialOrd, Ord)]
pub struct SceneCutDetector {
    input_video_path: PathBuf,
}

impl SceneCutDetector {
    pub fn new(input_video_path: &Path) -> Self {
        Self {
            input_video_path: input_video_path.to_path_buf(),
        }
    }

    pub fn detect_scene_cuts(&self, canceller: &Canceller) -> Result<Vec<SceneCutDetectionRecord>> {
        let mut records = Vec::new();

        let output = process::run(self.command(), canceller, |_, line| {
            if let Some(record) = parse_scdet_log_line(line) {
                records.push(record);
            }
        })?;

        if !output.success() {
            bail!("场景检测失败：{}", output.failure_message());
        }

        Ok(records)
    }

    fn command(&self) -> Command {
        let mut command = Command::new("ffmpeg");
        command
            .args(["-hide_banner", "-nostats", "-v", "info", "-i"])
            .arg(&self.input_video_path)
            .args(["-an", "-vf", "scdet", "-f", "null", "-"]);
        command
    }
}

/// 解析形如 `[scdet @ ...] lavfi.scd.score: 12.3, lavfi.scd.time: 4.5` 的日志行。
pub fn parse_scdet_log_line(log_line: &str) -> Option<SceneCutDetectionRecord> {
    Some(SceneCutDetectionRecord {
        scene_change_score: number_after(log_line, "lavfi.scd.score: ")?,
        scene_change_time_seconds: number_after(log_line, "lavfi.scd.time: ")?,
    })
}

fn number_after(log_line: &str, marker: &str) -> Option<f64> {
    let start = log_line.find(marker)? + marker.len();
    let end = log_line[start..]
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .map_or(log_line.len(), |offset| start + offset);

    log_line[start..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scdet_line() {
        let record =
            parse_scdet_log_line("[scdet @ 000001] lavfi.scd.score: 12.3, lavfi.scd.time: 4.56")
                .unwrap();
        assert!((record.scene_change_score - 12.3).abs() < 1e-9);
        assert!((record.scene_change_time_seconds - 4.56).abs() < 1e-9);
        assert!(parse_scdet_log_line("frame= 10").is_none());
    }
}
