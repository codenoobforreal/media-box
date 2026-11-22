//! `-d/--depth` 的扫描行为：用真实临时目录验证"哪些文件会被处理"，

use media_box_core::media_scan::collect_videos;
use std::fs;
use std::path::{Path, PathBuf};

struct TempTree {
    root: PathBuf,
}

impl TempTree {
    fn new(label: &str) -> Self {
        // 副作用收敛在项目内（crate 根下的临时目录）：系统临时目录在
        // DSH 工作区之外，工作区策略下创建目录会被沙箱拒绝。
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("media-box-scan-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        for dir in ["", "sub", "sub/deep"] {
            fs::create_dir_all(root.join(dir)).expect("创建测试目录");
        }
        for file in ["a.mp4", "sub/b.mkv", "sub/deep/c.mov", "sub/skip.txt"] {
            fs::write(root.join(file), b"x").expect("创建测试文件");
        }

        Self { root }
    }

    fn collected(&self, depth: Option<u8>) -> Vec<String> {
        let mut names: Vec<String> = collect_videos(&[&self.root], depth)
            .expect("扫描不应失败")
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("文件必有名字")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn depth_controls_how_deep_the_scan_goes() {
    let tree = TempTree::new("depth");

    assert_eq!(tree.collected(Some(0)), ["a.mp4"]);
    assert_eq!(tree.collected(Some(1)), ["a.mp4", "b.mkv"]);
    assert_eq!(tree.collected(None), ["a.mp4", "b.mkv", "c.mov"]);
}
