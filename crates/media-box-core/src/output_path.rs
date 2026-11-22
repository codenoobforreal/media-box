use crate::format::format_unix_timestamp_compact;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// 输出基名：`<输出目录或源目录>/<源文件名去扩展名>`。
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

    Ok(directory.join(stem))
}

/// 最终文件名：`<基名>-<yymmddHHMMSS>.mp4`。时间戳由调用方注入，便于测试。
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

pub fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_name_uses_injected_timestamp() {
        let output = final_output_path(Path::new("out/movie"), 0).unwrap();
        assert_eq!(output, PathBuf::from("out/movie-700101000000.mp4"));
    }

    #[test]
    fn colliding_names_get_increasing_suffixes() {
        let dir = std::env::temp_dir().join(format!("media-box-outpath-{}", std::process::id()));
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
