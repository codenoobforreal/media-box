//! Ctrl+C 取消：区分「用户取消」与普通失败，支持二次强退。

use anyhow::Result;
use std::sync::Mutex;
#[cfg(feature = "app")]
use {anyhow::Context, std::process, std::sync::Arc};

/// 取消标记：把「用户取消」与普通失败区分开。
///
/// 各阶段检测到取消后返回该错误，上层据此结束整批任务，而不是把它算作失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("用户取消了当前操作")]
pub struct Cancelled;

impl Cancelled {
    /// 判断任意错误（含 `anyhow` 的上下文链）是否由取消引起。
    pub fn is_in(error: &anyhow::Error) -> bool {
        error.chain().any(|cause| cause.is::<Cancelled>())
    }
}

#[derive(Debug, Default)]
struct CancelState {
    cancelled: bool,
    second_press_armed: bool,
}

/// 取消状态句柄，可在信号线程与工作线程间共享。
#[derive(Debug, Default)]
pub struct Canceller {
    state: Mutex<CancelState>,
}

impl Canceller {
    /// 构造取消句柄（等价于 Default）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由 ctrlc handler 调用。返回 true 表示调用方应立即 `process::exit(1)`
    ///
    /// 把"是否强退"的决策留给调用方，而不是在这里 exit
    pub fn on_ctrl_c(&self) -> bool {
        // 锁中毒时仍能取到内部数据：信号处理线程里 panic 的代价比带着过期状态继续更高。
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if state.second_press_armed {
            return true;
        }

        state.second_press_armed = true;
        state.cancelled = true;
        false
    }

    /// 直接置位取消标记，供非 Ctrl+C 路径使用。
    pub fn cancel(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancelled = true;
    }

    /// 是否已被取消。
    pub fn is_cancelled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancelled
    }

    /// 已取消则返回 Cancelled，否则返回 Ok。
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }
}

/// 注册全局 Ctrl+C handler，返回共享的 Canceller
#[cfg(feature = "app")]
pub fn install() -> Result<Arc<Canceller>> {
    let canceller = Arc::new(Canceller::new());
    let for_handler = canceller.clone();

    ctrlc::set_handler(move || {
        if for_handler.on_ctrl_c() {
            process::exit(1);
        }
    })
    .context("设置 Ctrl-C 处理器失败")?;

    Ok(canceller)
}
