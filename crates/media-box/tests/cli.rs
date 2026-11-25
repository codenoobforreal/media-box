//! media-box 入口的失败路径：验证参数解析到流水线装配不会 panic。
//!
//! 只用不存在的输入路径，让流水线在扫描阶段就退出——不启动 ffmpeg/ffprobe。

use std::path::PathBuf;
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_media-box"))
}

#[test]
fn missing_input_exits_nonzero_without_touching_external_tools() {
    let output = Command::new(binary())
        .args(["video-compress", "-i", "/definitely/not/a/video.mkv"])
        .output()
        .expect("启动 media-box 失败");

    assert!(!output.status.success(), "不存在的输入应返回非零退出码");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("读取输入路径"),
        "应报告路径错误而非 panic：{stderr}"
    );
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
