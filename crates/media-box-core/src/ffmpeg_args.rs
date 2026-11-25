//! ffmpeg 命令组装的公共前缀与选项：把散落在各探测器里的裸参数拼接收敛到一处。
//!
//! 这里只提供「追加参数」的构建器，方法的调用顺序即参数顺序——`-ss` 在 `-i`
//! 之前或之后语义不同，构建器刻意不替调用方重排。纯组装、不启动进程，
//! 便于用 `get_args()` 做静态断言。

use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// ffmpeg 命令构建器。
#[derive(Debug)]
pub struct FfmpegCommand {
    command: Command,
}

impl Default for FfmpegCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl FfmpegCommand {
    /// 新建命令，带 `-hide_banner`。
    pub fn new() -> Self {
        let mut command = Command::new("ffmpeg");
        command.arg("-hide_banner");
        Self { command }
    }

    /// 追加 `-nostats`。
    pub fn no_stats(mut self) -> Self {
        self.command.arg("-nostats");
        self
    }

    /// 追加 `-v error`。
    pub fn quiet(mut self) -> Self {
        self.command.args(["-v", "error"]);
        self
    }

    /// 追加 `-an -sn -dn`：禁用音频、字幕与数据流。
    pub fn no_audio_subtitle_data(mut self) -> Self {
        self.command.args(["-an", "-sn", "-dn"]);
        self
    }

    /// 追加 `-dn -sn`：禁用数据流与字幕（编码路径历史顺序）。
    pub fn no_data_subtitle(mut self) -> Self {
        self.command.args(["-dn", "-sn"]);
        self
    }

    /// 追加 `-ss <秒>`。
    pub fn seek(mut self, at: Duration) -> Self {
        self.command.arg("-ss").arg(at.as_secs_f64().to_string());
        self
    }

    /// 追加 `-t <秒>`。
    pub fn duration(mut self, duration: Duration) -> Self {
        self.command
            .arg("-t")
            .arg(duration.as_secs_f64().to_string());
        self
    }

    /// 追加 `-i <路径>`。
    pub fn input(mut self, path: &Path) -> Self {
        self.command.arg("-i").arg(path);
        self
    }

    /// 追加 `-vf <滤镜>`。
    pub fn video_filter(mut self, filter: &str) -> Self {
        self.command.args(["-vf", filter]);
        self
    }

    /// 追加 `-lavfi <滤镜图>`。
    pub fn lavfi(mut self, filter: &str) -> Self {
        self.command.args(["-lavfi", filter]);
        self
    }

    /// 追加 `-f null -`：输出到空设备，用于分析而非写盘。
    pub fn null_output(mut self) -> Self {
        self.command.args(["-f", "null", "-"]);
        self
    }

    /// 追加未知的原始参数（如导出侧数据、编码参数），保持调用顺序。
    pub fn raw(mut self, args: &[&str]) -> Self {
        self.command.args(args);
        self
    }

    /// 暴露可变命令引用，供已有 `apply(&mut Command)` 风格的编码参数追加。
    pub fn as_command_mut(&mut self) -> &mut Command {
        &mut self.command
    }

    /// 取出组装好的命令。
    pub fn build(self) -> Command {
        self.command
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn args(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn preserves_option_order() {
        let command = FfmpegCommand::new()
            .no_stats()
            .no_audio_subtitle_data()
            .input(Path::new("in.mkv"))
            .seek(Duration::from_secs(2))
            .video_filter("scdet")
            .null_output()
            .build();
        assert_eq!(
            args(&command),
            [
                "-hide_banner",
                "-nostats",
                "-an",
                "-sn",
                "-dn",
                "-i",
                "in.mkv",
                "-ss",
                "2",
                "-vf",
                "scdet",
                "-f",
                "null",
                "-"
            ]
        );
    }

    #[test]
    fn seek_before_input_is_respected() {
        let command = FfmpegCommand::new()
            .quiet()
            .seek(Duration::from_millis(500))
            .duration(Duration::from_secs(1))
            .no_audio_subtitle_data()
            .input(Path::new("in.mkv"))
            .build();
        let got: Vec<&OsStr> = command.get_args().collect();
        let texts: Vec<String> = got
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            texts,
            [
                "-hide_banner",
                "-v",
                "error",
                "-ss",
                "0.5",
                "-t",
                "1",
                "-an",
                "-sn",
                "-dn",
                "-i",
                "in.mkv"
            ]
        );
    }
}
