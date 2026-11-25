//! media-box 入口的失败路径：验证参数解析到流水线装配不会 panic。
//!
//! 只用不存在的输入路径，让流水线在扫描阶段就退出——不启动 ffmpeg/ffprobe。

use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_media-box"))
}

#[test]
fn missing_input_exits_nonzero_with_clear_error() {
    let output = Command::new(binary())
        .args(["video-compress", "-i", "/definitely/not/a/video.mkv"])
        .output()
        .expect("启动 media-box 失败");

    assert!(!output.status.success(), "不存在的输入应返回非零退出码");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.trim().is_empty(), "失败时应给出错误信息");
    assert!(!stderr.contains("panicked"), "不应 panic：{stderr}");
}

#[test]
fn missing_subcommand_shows_help() {
    let output = Command::new(binary())
        .output()
        .expect("启动 media-box 失败");
    assert!(!output.status.success());
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("video-compress"),
        "应提示子命令：{combined}"
    );
}
