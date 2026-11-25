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
pub(crate) struct FfmpegCommand {
    command: Command,
}

impl Default for FfmpegCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl FfmpegCommand {
    /// 新建命令，带 `-hide_banner`。
    pub(crate) fn new() -> Self {
        let mut command = Command::new("ffmpeg");
        command.arg("-hide_banner");
        Self { command }
    }

    /// 追加 `-nostats`。
    pub(crate) fn no_stats(mut self) -> Self {
        self.command.arg("-nostats");
        self
    }

    /// 追加 `-v error`。
    pub(crate) fn quiet(mut self) -> Self {
        self.command.args(["-v", "error"]);
        self
    }

    /// 追加 `-an -sn -dn`：禁用音频、字幕与数据流。
    ///
    /// 用于探测路径（VMAF / scdet / YDIF）：音频与字幕完全不参与分析。
    pub(crate) fn no_audio_subtitle_data(mut self) -> Self {
        self.command.args(["-an", "-sn", "-dn"]);
        self
    }

    /// 追加 `-dn -sn`：禁用数据流与字幕（编码路径历史顺序）。
    ///
    /// 仅编码路径使用：音频要 `-c:a copy` 保留，不能加 `-an`；
    /// 保留原有的 `-dn -sn` 顺序，避免无意义的命令差异。
    pub(crate) fn no_data_subtitle(mut self) -> Self {
        self.command.args(["-dn", "-sn"]);
        self
    }

    /// 追加 `-ss <秒>`。
    pub(crate) fn seek(mut self, at: Duration) -> Self {
        self.command.arg("-ss").arg(at.as_secs_f64().to_string());
        self
    }

    /// 追加 `-t <秒>`。
    pub(crate) fn duration(mut self, duration: Duration) -> Self {
        self.command
            .arg("-t")
            .arg(duration.as_secs_f64().to_string());
        self
    }

    /// 追加 `-i <路径>`。
    pub(crate) fn input(mut self, path: &Path) -> Self {
        self.command.arg("-i").arg(path);
        self
    }

    /// 追加 `-y <路径>`：覆盖写输出。
    ///
    /// 直接传 `&Path`（`AsRef<OsStr>`），不经 `to_string_lossy`——非 UTF-8
    /// 文件名必须原样交给 ffmpeg，否则会写到被 U+FFFD 替换后的另一个名字。
    pub(crate) fn output(mut self, path: &Path) -> Self {
        self.command.arg("-y").arg(path);
        self
    }

    /// 追加 `-vf <滤镜>`。
    pub(crate) fn video_filter(mut self, filter: &str) -> Self {
        self.command.args(["-vf", filter]);
        self
    }

    /// 追加 `-lavfi <滤镜图>`。
    pub(crate) fn lavfi(mut self, filter: &str) -> Self {
        self.command.args(["-lavfi", filter]);
        self
    }

    /// 追加 `-f null -`：输出到空设备，用于分析而非写盘。
    pub(crate) fn null_output(mut self) -> Self {
        self.command.args(["-f", "null", "-"]);
        self
    }

    /// 追加未知的原始参数（如导出侧数据、编码参数），保持调用顺序。
    pub(crate) fn raw(mut self, args: &[&str]) -> Self {
        self.command.args(args);
        self
    }

    /// 暴露可变命令引用，供已有 `apply(&mut Command)` 风格的编码参数追加。
    pub(crate) fn as_command_mut(&mut self) -> &mut Command {
        &mut self.command
    }

    /// 取出组装好的命令。
    pub(crate) fn build(self) -> Command {
        self.command
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    fn args(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn output_keeps_the_path_as_os_str() {
        let path = PathBuf::from("有 空格 的目录/out.mp4");
        let command = FfmpegCommand::new().output(&path).build();

        let all: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(
            all,
            [
                OsStr::new("-hide_banner"),
                OsStr::new("-y"),
                path.as_os_str(),
            ]
        );
    }

    /// 非 UTF-8 路径必须原样传给 ffmpeg；用 `to_string_lossy` 会写错文件名。
    #[cfg(windows)]
    #[test]
    fn output_preserves_non_utf8_path_bytes() {
        use std::ffi::OsString;
        use std::os::windows::ffi::{OsStrExt, OsStringExt};

        let mut wide: Vec<u16> = OsStr::new("out").encode_wide().collect();
        wide.push(0xD800); // 未配对代理项：合法 OsStr，但不是合法 UTF-8
        wide.extend(OsStr::new(".mp4").encode_wide());
        let path = PathBuf::from(OsString::from_wide(&wide));

        // 前提校验：该路径确实无法无损转成 UTF-8。
        assert!(path.to_string_lossy().contains('\u{FFFD}'));

        let command = FfmpegCommand::new().output(&path).build();
        let last = command.get_args().last();
        assert_eq!(last, Some(path.as_os_str()));
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
