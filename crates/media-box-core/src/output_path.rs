//! 输出路径推导：基名、带时间戳的最终名与同批冲突去重。

use crate::format::format_unix_timestamp_compact;
use crate::path_budget::{safe_output_stem, validate_output_path};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// `final_output_path` 追加的后缀：时间戳 12 + `-` 1 + `.mp4` 4 + 冲突余量 11。
const FINAL_NAME_RESERVE: usize = 28;

/// 输出基名：`<输出目录或源目录>/<源文件名去扩展名>`。
///
/// # Errors
///
/// 源文件名不是合法 UTF-8、无法创建输出目录或取不到源目录时返回错误。
pub fn output_base_path(video: &Path, output_dir: Option<&Path>) -> Result<PathBuf> {
    let stem = video
        .file_stem()
        .and_then(|stem| stem.to_str())
        .with_context(|| format!("源文件名包含非 UTF-8 字符：{}", video.display()))?;

    let directory = match output_dir {
        Some(directory) => {
            fs::create_dir_all(directory)
                .with_context(|| format!("创建输出目录 {}", directory.display()))?;
            directory
        }
        None => video
            .parent()
            .with_context(|| format!("获取源视频所在目录：{}", video.display()))?,
    };

    // 相对路径受 MAX_PATH 260 限制，需把目录前缀计入文件名预算；
    // 绝对路径由 Rust std 自动加 `\\?\`，只受组件 255 限制。
    #[cfg(windows)]
    let directory_units = if directory.is_absolute() {
        0
    } else {
        directory
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .count()
            + 1
    };
    #[cfg(not(windows))]
    let directory_units = 0;

    let stem = safe_output_stem(stem, FINAL_NAME_RESERVE.saturating_add(directory_units))?;
    let base = directory.join(stem);
    validate_output_path(&base)?;
    Ok(base)
}

/// 最终文件名：`<基名>-<yymmddHHMMSS>.mp4`。时间戳由调用方注入，便于测试。
///
/// # Errors
///
/// 基名不是合法 UTF-8 或检查路径是否存在失败时返回错误。
pub fn final_output_path(base: &Path, unix_seconds: u64) -> Result<PathBuf> {
    let stem = base
        .file_name()
        .and_then(|stem| stem.to_str())
        .with_context(|| format!("生成输出文件名：{}", base.display()))?;

    let timestamp = format_unix_timestamp_compact(unix_seconds);
    let mut suffix = 0u32;

    loop {
        let suffix_text = if suffix == 0 {
            String::new()
        } else {
            format!("-{suffix}")
        };
        let candidate = base.with_file_name(format!("{stem}-{timestamp}{suffix_text}.mp4"));
        validate_output_path(&candidate)?;

        // 顺序批处理下够用；不防多个进程同时写同一路径的竞态。
        if !candidate
            .try_exists()
            .with_context(|| format!("检查输出文件 {} 是否存在", candidate.display()))?
        {
            return Ok(candidate);
        }

        suffix += 1;
    }
}

/// 当前 Unix 时间戳（秒）；系统时钟早于纪元时退回 0。
pub fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// 处理同批次内的输出冲突：desired 已被占用时追加 -1、-2 后缀。
///
/// 只解决批次内冲突，不检查磁盘上是否已存在；是否跳过已存在文件由调用方决定。
/// 典型用法：主线程先为整批预留输出路径，再分发并发任务。
#[must_use]
pub fn collision_free<H: std::hash::BuildHasher>(
    desired: &Path,
    taken: &mut HashSet<PathBuf, H>,
) -> PathBuf {
    if taken.insert(desired.to_path_buf()) {
        return desired.to_path_buf();
    }

    let stem = desired.file_stem().unwrap_or(OsStr::new(""));
    let extension = desired.extension().unwrap_or(OsStr::new(""));
    let mut suffix = 1u32;
    loop {
        // 无扩展名时不追加尾点。
        let candidate = if extension.is_empty() {
            desired.with_file_name(format!("{}-{suffix}", stem.display()))
        } else {
            desired.with_file_name(format!(
                "{}-{suffix}.{}",
                stem.display(),
                extension.display()
            ))
        };
        if taken.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path_budget::MAX_COMPONENT_UTF16;

    #[test]
    fn batch_collision_free_keeps_first_and_suffixes_rest() {
        let mut taken = HashSet::new();
        let desired = Path::new("out/photo.webp");

        assert_eq!(collision_free(desired, &mut taken), *desired);
        assert_eq!(
            collision_free(desired, &mut taken),
            PathBuf::from("out/photo-1.webp")
        );
        assert_eq!(
            collision_free(desired, &mut taken),
            PathBuf::from("out/photo-2.webp")
        );
    }

    #[test]
    fn batch_collision_free_handles_extensionless_names() {
        let mut taken = HashSet::new();
        let desired = Path::new("out/movie");

        assert_eq!(collision_free(desired, &mut taken), *desired);
        assert_eq!(
            collision_free(desired, &mut taken),
            PathBuf::from("out/movie-1")
        );
    }

    #[test]
    fn final_name_uses_injected_timestamp() {
        let output = final_output_path(Path::new("out/movie"), 0).unwrap();
        assert_eq!(output, PathBuf::from("out/movie-700101000000.mp4"));
    }

    #[test]
    fn long_source_stem_is_truncated_within_final_name_budget() {
        // Windows 相对路径受 MAX_PATH 260 限制，这里用短相对路径避免绝对路径
        // 触发扩展长度路径预算；output_base_path 只在该 crate 根下建目录。
        let dir = Path::new(".tmp-tests").join(format!("m{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let name = format!("{}.mkv", "a".repeat(300));
        let base = output_base_path(Path::new(&name), Some(&dir)).unwrap();
        let final_name = final_output_path(&base, 0).unwrap();
        let file_name = final_name.file_name().unwrap().to_string_lossy();
        assert!(
            file_name.encode_utf16().count() <= MAX_COMPONENT_UTF16,
            "最终输出名不能超组件上限：{file_name}"
        );
        assert!(file_name.ends_with("-700101000000.mp4"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn colliding_names_get_increasing_suffixes() {
        // 副作用收敛在项目内（crate 根下的临时目录）：系统临时目录在
        // DSH 工作区之外，工作区策略下创建目录会被沙箱拒绝。
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("media-box-outpath-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let first = final_output_path(&dir.join("movie"), 0).unwrap();
        fs::write(&first, b"x").unwrap();

        let second = final_output_path(&dir.join("movie"), 0).unwrap();

        assert_ne!(first, second, "同一时间戳不得返回已存在的路径");
        assert!(
            second
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("-1.mp4")
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
