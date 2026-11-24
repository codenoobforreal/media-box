//! 端到端测试：直接运行编译出的 `image-convert` 二进制，用仓库内的本地图片
//! 样本做真实转换。
//!
//! 样本位于 `tests/fixtures/`，均为仓库内的静态文件，测试过程不联网；所有
//! 输出写入工作区内的临时目录（`.tmp-tests/`），测试结束自动删除。

use std::path::{Path, PathBuf};
use std::process::Command;

use image::{GenericImageView, ImageReader};

#[path = "support/test_dir.rs"]
mod test_dir;

use test_dir::TestDir;

/// 仓库内图片样本目录。
fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// 被测二进制路径（Cargo 在集成测试中注入）。
fn image_convert() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_image-convert"))
}

/// 取本地样本路径，缺失时直接失败并指明位置。
fn fixture(name: &str) -> PathBuf {
    let path = fixtures_dir().join(name);
    assert!(path.exists(), "缺少本地样本：{}", path.display());
    path
}

/// 以指定格式转换样本，返回退出状态与 stderr 文本。
fn convert(source: &Path, format: &str, out: &Path) -> (std::process::ExitStatus, String) {
    let output = Command::new(image_convert())
        .arg("-i")
        .arg(source)
        .arg("-f")
        .arg(format)
        .arg("-o")
        .arg(out)
        .output()
        .expect("启动 image-convert 失败");

    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (output.status, stderr)
}

/// 校验源图与输出尺寸一致（两者都能被 image 内置解码器读取）。
fn assert_dimensions_match(source: &Path, target: &Path) {
    let source_dims = ImageReader::open(source)
        .unwrap()
        .decode()
        .unwrap()
        .dimensions();
    let target_dims = ImageReader::open(target)
        .unwrap()
        .decode()
        .unwrap()
        .dimensions();
    assert_eq!(target_dims, source_dims, "输出尺寸应与源图一致");
}

#[test]
fn converts_local_png_fixture_to_webp() {
    let root = TestDir::new("e2e-png-webp");
    let out = root.join("out");

    let (status, stderr) = convert(&fixture("photo_mountain.png"), "webp", &out);
    assert!(status.success(), "转换应成功：{stderr}");

    let target = out.join("photo_mountain.webp");
    assert!(target.exists(), "输出文件应存在：{}", target.display());
    assert_dimensions_match(&fixture("photo_mountain.png"), &target);
}

#[test]
fn converts_local_jpg_fixture_to_png() {
    let root = TestDir::new("e2e-jpg-png");
    let out = root.join("out");

    let (status, stderr) = convert(&fixture("photo_desert.jpg"), "png", &out);
    assert!(status.success(), "转换应成功：{stderr}");

    let target = out.join("photo_desert.png");
    assert!(target.exists(), "输出文件应存在：{}", target.display());
    assert_dimensions_match(&fixture("photo_desert.jpg"), &target);
}

#[test]
fn dry_run_plans_without_writing() {
    let root = TestDir::new("e2e-dry-run");
    let out = root.join("out");

    let output = Command::new(image_convert())
        .arg("-i")
        .arg(fixture("photo_mountain.png"))
        .arg("-f")
        .arg("webp")
        .arg("--dry-run")
        .arg("-o")
        .arg(&out)
        .output()
        .expect("启动 image-convert 失败");

    assert!(output.status.success(), "dry-run 应成功");
    assert!(!out.exists(), "dry-run 不应写盘");
}
