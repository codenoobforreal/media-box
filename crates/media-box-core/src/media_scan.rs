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

/// 收集所有起始路径下的视频文件。
///
/// - `Some(0)`：只看当前目录；`Some(n)`：最多递归 n 层；`None`：不限层级。
pub fn collect_videos(paths: &[impl AsRef<Path>], max_depth: Option<u8>) -> Result<Vec<PathBuf>> {
    let mut dir_roots = Vec::new();
    let mut file_roots = Vec::new();

    for path in paths {
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
            file_roots.push(absolute);
        }
    }

    dir_roots.sort_by_key(|root| cmp::Reverse(root.components().count()));

    let mut collector = VideoCollector::default();

    for root in &dir_roots {
        collector.collect_dir(root, max_depth)?;
    }
    for path in file_roots {
        collector.accept_file(path);
    }

    Ok(collector.videos)
}

#[derive(Default)]
struct VideoCollector {
    videos: Vec<PathBuf>,
    seen: HashSet<PathBuf>,
    /// 目录 → 已用过的剩余深度；`None` 表示无限。
    visited_dirs: HashMap<PathBuf, Option<u8>>,
}

impl VideoCollector {
    fn collect_dir(&mut self, root: &Path, max_depth: Option<u8>) -> Result<()> {
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
                } else if file_type.is_file() && is_video_by_ext(&path) {
                    self.accept_file(path);
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

    fn accept_file(&mut self, path: PathBuf) {
        if is_video_by_ext(&path) && self.seen.insert(path.clone()) {
            self.videos.push(path);
        }
    }
}

fn is_video_by_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            VIDEO_EXTENSIONS
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
