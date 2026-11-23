use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// 输出路径推导错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutputPathError {
    #[error("源文件名包含非 UTF-8 字符：{0}")]
    NonUtf8Source(PathBuf),
}

/// 由源文件推导目标输出路径。
///
/// - `output_dir` 为 `Some` 时，把源文件相对 `input_root` 的目录结构镜像到 `output_dir` 下；
/// - `output_dir` 为 `None` 时，输出到源文件所在目录（就地转换）；
/// - 源文件不在 `input_root` 下时，直接平铺到目标目录。
///
/// 文件名统一改为 `<stem>.<extension>`。
pub fn output_path(
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

    Ok(directory.join(format!("{stem}.{extension}")))
}

/// 处理同批次内的输出冲突：`desired` 已被占用时追加 `-1`、`-2` 后缀。
///
/// 只解决批次内冲突，不检查磁盘上是否已存在；是否跳过已存在文件由调用方决定。
#[must_use]
pub fn collision_free(desired: &Path, taken: &mut HashSet<PathBuf>) -> PathBuf {
    if taken.insert(desired.to_path_buf()) {
        return desired.to_path_buf();
    }

    let stem = desired.file_stem().unwrap_or(OsStr::new(""));
    let extension = desired.extension().unwrap_or(OsStr::new(""));
    let mut suffix = 1u32;
    loop {
        let candidate = desired.with_file_name(format!(
            "{}-{suffix}.{}",
            stem.display(),
            extension.display()
        ));
        if taken.insert(candidate.clone()) {
            return candidate;
        }
        suffix += 1;
    }
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
        let path = output_path(source, None, Path::new("in"), "avif").unwrap();
        assert_eq!(path, PathBuf::from("in/sub/photo.avif"));
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
    fn collision_free_keeps_path_when_unused() {
        let mut taken = HashSet::new();
        let desired = Path::new("out/photo.webp");
        assert_eq!(collision_free(desired, &mut taken), *desired);
        assert!(taken.contains(desired));
    }

    #[test]
    fn collision_free_appends_increasing_suffixes() {
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
}
