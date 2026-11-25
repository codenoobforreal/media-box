//! 按扩展名递归收集媒体文件：跳过符号链接、绝对路径去重。

use anyhow::{Context, Result};
use std::cmp;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{self, Path, PathBuf};

/// 支持的视频扩展名（小写）。
const VIDEO_EXTENSIONS: [&str; 25] = [
    "3g2", "3gp", "asf", "avi", "dv", "f4v", "flv", "m2ts", "m2v", "m4v", "mkv", "mod", "mov",
    "mp4", "mpeg", "mpg", "mts", "ogv", "qt", "rm", "rmvb", "ts", "vob", "webm", "wmv",
];

/// 扫描到的一个候选文件及其所属输入根目录。
///
/// root 用于输出目录结构镜像：目录扫描得到文件的根是扫描起点目录，
/// 显式传入文件的根是其所在目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedFile {
    /// 文件的绝对路径。
    pub path: PathBuf,
    /// 文件所属的输入根目录，用于镜像输出目录结构。
    pub root: PathBuf,
}

/// 收集所有起始路径下的视频文件。
///
/// - Some(0)：只看当前目录；Some(n)：最多递归 n 层；None：不限层级。
///
/// # Errors
///
/// 解析输入绝对路径或读取目录失败时返回错误。
pub fn collect_videos(paths: &[impl AsRef<Path>], max_depth: Option<u8>) -> Result<Vec<PathBuf>> {
    Ok(collect_files(paths, &VIDEO_EXTENSIONS, max_depth)?
        .into_iter()
        .map(|file| file.path)
        .collect())
}

/// 按扩展名收集所有起始路径下的文件。
///
/// - 显式传入的文件一律接受（扩展名与可解码性由调用方负责）；
/// - 目录内容按 extensions（小写）筛选；
/// - 符号链接跳过，重复路径去重；
/// - Some(0)：只看当前目录；Some(n)：最多递归 n 层；None：不限层级。
///
/// # Errors
///
/// 解析输入绝对路径或读取目录失败时返回错误。
pub fn collect_files(
    inputs: &[impl AsRef<Path>],
    extensions: &[&str],
    max_depth: Option<u8>,
) -> Result<Vec<ScannedFile>> {
    let mut dir_roots = Vec::new();
    let mut file_roots = Vec::new();

    for path in inputs {
        let absolute = path::absolute(path.as_ref())
            .with_context(|| format!("解析输入路径 {}", path.as_ref().display()))?;
        let metadata = fs::symlink_metadata(&absolute)
            .with_context(|| format!("读取输入路径 {}", absolute.display()))?;

        if metadata.is_symlink() {
            continue;
        }

        if metadata.is_dir() {
            dir_roots.push(absolute);
        } else if metadata.is_file() {
            let root = absolute
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
            file_roots.push(ScannedFile {
                path: absolute,
                root,
            });
        }
    }

    // 深的目录根先扫，保证嵌套输入的深度语义更精确。
    dir_roots.sort_by_key(|root| cmp::Reverse(root.components().count()));

    let mut collector = Collector::default();

    for root in &dir_roots {
        collector.collect_dir(root, extensions, max_depth)?;
    }
    for file in file_roots {
        collector.accept_file(file);
    }

    // 目录遍历顺序由文件系统决定；按路径排序让同一输入的批次顺序稳定。
    collector
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));

    Ok(collector.files)
}

#[derive(Default)]
struct Collector {
    files: Vec<ScannedFile>,
    seen: HashSet<PathBuf>,
    /// 目录 → 已用过的剩余深度；None 表示无限。
    visited_dirs: HashMap<PathBuf, Option<u8>>,
}

impl Collector {
    fn collect_dir(
        &mut self,
        root: &Path,
        extensions: &[&str],
        max_depth: Option<u8>,
    ) -> Result<()> {
        let mut stack = vec![(root.to_path_buf(), max_depth)];

        while let Some((dir, depth)) = stack.pop() {
            if !self.claim_dir(&dir, depth) {
                continue;
            }

            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let file_type = entry.file_type()?;
                let path = entry.path();

                if file_type.is_dir() {
                    if depth != Some(0) {
                        stack.push((path, depth.map(|remaining| remaining - 1)));
                    }
                } else if file_type.is_file() && matches_any_ext(&path, extensions) {
                    self.accept_file(ScannedFile {
                        path,
                        root: root.to_path_buf(),
                    });
                }
            }
        }

        Ok(())
    }

    fn claim_dir(&mut self, dir: &Path, depth: Option<u8>) -> bool {
        match self.visited_dirs.get(dir).copied() {
            Some(previous) => {
                if depth_covers(previous, depth) {
                    return false;
                }
                self.visited_dirs
                    .insert(dir.to_path_buf(), merge_depth(previous, depth));
            }
            None => {
                self.visited_dirs.insert(dir.to_path_buf(), depth);
            }
        }

        true
    }

    fn accept_file(&mut self, file: ScannedFile) {
        if self.seen.insert(file.path.clone()) {
            self.files.push(file);
        }
    }
}

/// 路径扩展名（忽略大小写）是否命中给定清单。
fn matches_any_ext(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extensions
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

fn depth_covers(previous: Option<u8>, current: Option<u8>) -> bool {
    match previous {
        None => true,
        Some(_) if current.is_none() => false,
        Some(previous) => current.is_some_and(|remaining| previous >= remaining),
    }
}

fn merge_depth(left: Option<u8>, right: Option<u8>) -> Option<u8> {
    match (left, right) {
        (None, _) | (_, None) => None,
        (Some(left), Some(right)) => Some(left.max(right)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_tree(label: &str) -> PathBuf {
        // 副作用收敛在 crate 根下的临时目录：系统临时目录在 DSH 工作区之外。
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("media-box-scan-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("a.png"), b"x").unwrap();
        fs::write(root.join("sub/b.png"), b"x").unwrap();
        fs::write(root.join("sub/ignore.txt"), b"x").unwrap();
        root
    }

    #[test]
    fn collect_files_filters_by_extension_and_records_roots() {
        let root = temp_tree("files");

        let files = collect_files(&[&root], &["png"], Some(1)).unwrap();
        let paths: Vec<String> = files
            .iter()
            .map(|file| file.path.to_string_lossy().into_owned())
            .collect();

        assert_eq!(paths.len(), 2);
        assert!(paths.iter().any(|path| path.ends_with("a.png")));
        assert!(paths.iter().any(|path| path.ends_with("b.png")));
        // 目录扫描得到的 root 是扫描起点。
        assert!(
            files
                .iter()
                .all(|file| file.root == root || file.root == fs::canonicalize(&root).unwrap())
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn explicit_files_keep_parent_as_root_and_skip_extension_check() {
        let root = temp_tree("explicit");
        // temp_tree 只在 sub/ 下建了非图片文件：显式传入它验证“无条件接受”。
        let explicit = root.join("sub/ignore.txt");

        let files = collect_files(&[&explicit], &["png"], Some(0)).unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, explicit);
        // 显式文件的 root 是其所在目录。
        assert_eq!(files[0].root, explicit.parent().unwrap());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn duplicate_inputs_are_deduplicated() {
        let root = temp_tree("dedup");

        let files = collect_files(&[&root, &root], &["png"], Some(0)).unwrap();

        assert_eq!(files.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn symlink_inputs_are_skipped() {
        let root = temp_tree("symlink");
        let link = root.join("link.png");
        // Windows 无开发者模式/管理员权限时创建符号链接会失败：跳过该环境。
        let created = {
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_file(root.join("a.png"), &link).is_ok()
            }
            #[cfg(not(windows))]
            {
                std::os::unix::fs::symlink(root.join("a.png"), &link).is_ok()
            }
        };
        if !created {
            let _ = fs::remove_dir_all(&root);
            return;
        }

        let files = collect_files(&[&root], &["png"], Some(0)).unwrap();

        assert_eq!(files.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }
}
