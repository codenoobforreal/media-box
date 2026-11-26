//! 由源文件推导输出路径：镜像目录结构或就地转换。

use media_box_core::path_budget::{PathBudgetError, safe_output_stem, validate_output_path};
use std::path::{Path, PathBuf};

/// 输出路径推导错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OutputPathError {
    #[error("源文件名包含非 UTF-8 字符：{0}")]
    NonUtf8Source(PathBuf),
    #[error(transparent)]
    PathBudget(#[from] PathBudgetError),
}

/// 由源文件推导目标输出路径。
///
/// - `output_dir` 为 `Some` 时，把源文件相对 `input_root` 的目录结构镜像到 `output_dir` 下；
/// - `output_dir` 为 `None` 时，输出到源文件所在目录（就地转换）；
/// - 源文件不在 `input_root` 下时，直接平铺到目标目录。
///
/// 文件名统一改为 `<stem>.<extension>`。
pub(crate) fn output_path(
    source: &Path,
    output_dir: Option<&Path>,
    input_root: &Path,
    extension: &str,
) -> Result<PathBuf, OutputPathError> {
    let stem = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| OutputPathError::NonUtf8Source(source.to_path_buf()))?;
    let mirrored = source
        .strip_prefix(input_root)
        .ok()
        .and_then(|relative| relative.parent())
        .unwrap_or_else(|| Path::new(""));

    let directory = match output_dir {
        Some(directory) => directory.join(mirrored),
        None => source
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf(),
    };

    // 目标名还要容纳 `.<pid>.tmp` 临时后缀与 `-N` 冲突后缀。
    let stem = safe_output_stem(stem, 1 + extension.len() + 16 + 12)?;
    let target = directory.join(format!("{stem}.{extension}"));
    validate_output_path(&target)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirrors_relative_structure_under_output_dir() {
        let source = Path::new("in/sub/dir/photo.png");
        let path = output_path(source, Some(Path::new("out")), Path::new("in"), "webp").unwrap();
        assert_eq!(path, PathBuf::from("out/sub/dir/photo.webp"));
    }

    #[test]
    fn writes_in_place_when_output_dir_is_none() {
        let source = Path::new("in/sub/photo.png");
        let path = output_path(source, None, Path::new("in"), "webp").unwrap();
        assert_eq!(path, PathBuf::from("in/sub/photo.webp"));
    }

    #[test]
    fn flattens_sources_outside_the_input_root() {
        let source = Path::new("elsewhere/photo.png");
        let path = output_path(source, Some(Path::new("out")), Path::new("in"), "jpg").unwrap();
        assert_eq!(path, PathBuf::from("out/photo.jpg"));
    }

    #[test]
    fn root_level_file_stays_at_output_root() {
        let source = Path::new("in/photo.png");
        let path = output_path(source, Some(Path::new("out")), Path::new("in"), "gif").unwrap();
        assert_eq!(path, PathBuf::from("out/photo.gif"));
    }

    #[test]
    fn long_stem_is_truncated_within_component_budget() {
        let name = format!("{}.png", "a".repeat(300));
        let source = Path::new("in").join(&name);
        let path = output_path(&source, Some(Path::new("out")), Path::new("in"), "webp").unwrap();
        let file_name = path.file_name().unwrap().to_string_lossy();
        assert!(
            file_name.encode_utf16().count() <= 255 - (16 + 12),
            "目标名应给临时与冲突后缀留足预算：{file_name}"
        );
        assert!(file_name.ends_with(".webp"));
    }

    #[cfg(windows)]
    #[test]
    fn deep_output_dir_over_max_path_is_rejected() {
        let name = format!("{}.png", "a".repeat(300));
        let source = Path::new("in").join(&name);
        let output = format!("{}/out", "d".repeat(252));
        assert!(
            output_path(&source, Some(Path::new(&output)), Path::new("in"), "webp").is_err(),
            "超长相对输出路径应报错"
        );
    }
}
