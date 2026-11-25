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
        .expect("打开源样本")
        .decode()
        .expect("解码源样本")
        .dimensions();
    let target_dims = ImageReader::open(target)
        .expect("打开输出文件")
        .decode()
        .expect("解码输出文件")
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

/// 所有内置格式都能作为输入和输出：用仓库内夹具做参数化 round-trip。
#[test]
fn round_trips_every_supported_input_format() {
    let cases = [
        ("photo_mountain.png", "webp"),
        ("photo_mountain.bmp", "png"),
        ("photo_mountain.jpg", "gif"),
        ("photo_mountain.webp", "jpg"),
        ("composite image.gif", "bmp"),
        ("composite image.png", "jpg"),
    ];

    for (index, (source_name, format)) in cases.iter().enumerate() {
        let root = TestDir::new(&format!("e2e-round-{index}"));
        let out = root.join("out");
        let source = fixture(source_name);

        let (status, stderr) = convert(&source, format, &out);
        assert!(status.success(), "{source_name} -> {format} 失败：{stderr}");

        let stem = Path::new(source_name)
            .file_stem()
            .unwrap()
            .to_string_lossy();
        let target = out.join(format!("{stem}.{format}"));
        assert!(target.exists(), "输出应存在：{}", target.display());
        assert_dimensions_match(&source, &target);
    }
}

/// 信息查询开关无需 -f 也能成功，且输出可读。
#[test]
fn info_switches_work_without_a_target_format() {
    let source = fixture("photo_mountain.png");
    let cases: &[&[&str]] = &[&["--list-formats"], &["--meta", "list"]];

    for extra in cases {
        let output = Command::new(image_convert())
            .arg("-i")
            .arg(&source)
            .args(*extra)
            .output()
            .expect("启动 image-convert 失败");
        assert!(output.status.success(), "{extra:?} 应成功");
        assert!(
            !String::from_utf8_lossy(&output.stdout).is_empty(),
            "{extra:?} 应有输出"
        );
    }
}

/// 目标已存在时默认跳过，--overwrite 才覆盖；错误输入返回非零退出码。
#[test]
fn overwrite_and_error_exit_codes() {
    let root = TestDir::new("e2e-overwrite");
    let out = root.join("out");
    let source = fixture("photo_mountain.png");
    let target = out.join("photo_mountain.webp");

    let (status, stderr) = convert(&source, "webp", &out);
    assert!(status.success(), "首次转换应成功：{stderr}");
    let first = std::fs::read(&target).unwrap();

    let (status, _) = convert(&source, "webp", &out);
    assert!(status.success(), "跳过已存在文件不应失败");

    let output = Command::new(image_convert())
        .args(["-i", &source.to_string_lossy(), "-f", "webp", "-o"])
        .arg(&out)
        .arg("--overwrite")
        .output()
        .expect("启动 image-convert 失败");
    assert!(output.status.success(), "覆盖转换应成功");
    let second = std::fs::read(&target).unwrap();
    assert_eq!(first.len(), second.len(), "覆盖后文件仍应可读");

    let output = Command::new(image_convert())
        .args(["-i", "/definitely/not/a/file.png", "-f", "webp"])
        .output()
        .expect("启动 image-convert 失败");
    assert!(!output.status.success(), "不存在的输入应返回非零退出码");
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
