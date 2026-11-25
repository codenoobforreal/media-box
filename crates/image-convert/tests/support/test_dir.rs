//! 测试用临时目录：把测试副作用统一收在仓库内的 `.tmp-tests/` 下，离开作用域自动删除。
//!
//! 不写入系统临时目录：既避免并行测试互相覆盖，也保证副作用不超出工作区，
//! 便于在文件系统权限受限的环境（如 DSH 沙箱）下运行。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// 进程内自增序号：与 `name`、进程号一起保证并行测试的目录互不冲突。
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// 工作区内的临时测试目录；离开作用域时自动删除。
#[derive(Debug)]
pub(crate) struct TestDir {
    path: PathBuf,
}

impl TestDir {
    /// 在仓库内的 `.tmp-tests/` 下创建唯一临时目录。
    pub(crate) fn new(name: &str) -> Self {
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("image-convert-{name}-{}-{seq}", std::process::id()));

        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("创建临时测试目录失败");

        Self { path: dir }
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl std::ops::Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
