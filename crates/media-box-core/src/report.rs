//! 流水线阶段定义与成功 / 失败 / 警告报告模型。

use crate::byte_size::ByteSize;
use crate::format::format_duration_fixed;
use crate::progress::ProgressSnapshot;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// 单个视频的流水线阶段，枚举顺序即执行顺序。
#[expect(
    clippy::exhaustive_enums,
    reason = "阶段集合与 UI 严格同源，新增阶段应强制所有渲染与调度处同步更新"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// 读取元数据（分辨率、帧率、时长、体积）。
    Metadata,
    /// 场景切点检测与样本规划。
    SceneSampling,
    /// 颗粒强度（YDIF）分析。
    FilmGrain,
    /// CRF 搜索与 VMAF 测量。
    CrfSearch,
    /// 按选定 CRF 正式编码。
    Encode,
}

impl Stage {
    /// 全部阶段，按执行顺序排列。
    pub const ALL: [Stage; 5] = [
        Stage::Metadata,
        Stage::SceneSampling,
        Stage::FilmGrain,
        Stage::CrfSearch,
        Stage::Encode,
    ];

    /// 阶段在终端界面上显示的名称。
    pub fn label(self) -> &'static str {
        match self {
            Self::Metadata => "读取元数据",
            Self::SceneSampling => "场景采样",
            Self::FilmGrain => "颗粒强度分析",
            Self::CrfSearch => "CRF 搜索",
            Self::Encode => "正式编码",
        }
    }
}

/// 单个视频的压缩结果摘要：体积变化、耗时与实际执行的 ffmpeg 命令。
#[derive(Debug, Clone)]
pub struct SuccessReport {
    src_size: ByteSize,
    output_size: ByteSize,
    src_duration: Duration,
    compress_duration: Duration,
    ffmpeg_command: String,
}

impl SuccessReport {
    /// 构造成功报告。
    pub fn new(
        src_size: ByteSize,
        output_size: ByteSize,
        src_duration: Duration,
        compress_duration: Duration,
        ffmpeg_command: String,
    ) -> Self {
        Self {
            src_size,
            output_size,
            src_duration,
            compress_duration,
            ffmpeg_command,
        }
    }

    fn output_src_size_percentage(&self) -> f64 {
        let src = self.src_size.as_u64();

        if src == 0 {
            return 100.0;
        }

        #[expect(
            clippy::as_conversions,
            clippy::cast_precision_loss,
            reason = "仅用于展示百分比，u64 超过 2^53 的体积在 f64 中的误差不可见"
        )]
        let percentage = self.output_size.as_u64() as f64 / src as f64 * 100.0;
        percentage
    }

    fn compress_src_duration_percentage(&self) -> f64 {
        if self.src_duration.is_zero() {
            return 100.0;
        }

        self.compress_duration.div_duration_f64(self.src_duration) * 100.0
    }
}

impl fmt::Display for SuccessReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.ffmpeg_command)?;
        writeln!(
            f,
            "{} → {}  |  {}  |  {} → {}  |  {}",
            self.src_size,
            self.output_size,
            ratio_arrow(self.output_src_size_percentage()),
            format_duration_fixed(self.src_duration),
            format_duration_fixed(self.compress_duration),
            ratio_arrow(self.compress_src_duration_percentage()),
        )
    }
}

/// 流水线事件：阶段推进、进度、结果与汇总，由 Reporter 发送给渲染方。
#[expect(
    clippy::exhaustive_enums,
    reason = "事件集合由本库与终端渲染层共同约定，新增事件应强制渲染层同步处理"
)]
#[derive(Debug)]
pub enum Event {
    /// 开始处理某个文件。
    JobStarted(PathBuf),
    /// 切换阶段，`detail` 是阶段行上的附加说明。
    Stage {
        /// 当前阶段。
        stage: Stage,
        /// 附加说明，如「CRF 32.00 · 样本 2/3」。
        detail: Option<String>,
    },
    /// 阶段内的临时说明，只影响当前阶段行。
    Note(String),
    /// 一次进度快照。
    Progress(ProgressSnapshot),
    /// 单个文件处理完成，`report` 为 `None` 表示未产生新文件（如已跳过）。
    JobCompleted {
        /// 源文件路径。
        path: PathBuf,
        /// 成功摘要。
        report: Option<SuccessReport>,
    },
    /// 单个文件处理失败。
    JobFailed {
        /// 源文件路径。
        path: PathBuf,
        /// 用户可读的错误文本。
        error: String,
    },
    /// 一条不中断处理的警告。
    Warning(String),
    /// 用户取消了整个批次。
    Cancelled,
    /// 批次结束。
    Finished,
}

/// 事件发送方：把压缩过程中的状态变化发给 UI。
#[derive(Debug, Clone)]
pub struct Reporter {
    sender: Sender<Event>,
}

impl Reporter {
    /// 用事件通道构造 Reporter。
    pub fn new(sender: Sender<Event>) -> Self {
        Self { sender }
    }

    /// 通知开始处理某个文件。
    pub fn job_started(&self, path: &Path) {
        self.send(Event::JobStarted(path.to_path_buf()));
    }

    /// 切换当前阶段（不带附加说明）。
    pub fn stage(&self, stage: Stage) {
        self.send(Event::Stage {
            stage,
            detail: None,
        });
    }

    /// 切换当前阶段并附带说明文字。
    pub fn stage_detail(&self, stage: Stage, detail: impl Into<String>) {
        self.send(Event::Stage {
            stage,
            detail: Some(detail.into()),
        });
    }

    /// 阶段内的临时说明（例如“区间已达标”），只影响当前阶段行的附加文字。
    pub fn note(&self, message: impl Into<String>) {
        self.send(Event::Note(message.into()));
    }

    /// 上报一次进度快照。
    pub fn progress(&self, snapshot: ProgressSnapshot) {
        self.send(Event::Progress(snapshot));
    }

    /// 上报一条警告（不中断处理）。
    pub fn warning(&self, message: impl Into<String>) {
        self.send(Event::Warning(message.into()));
    }

    /// 通知某个文件处理完成。
    pub fn job_completed(&self, path: PathBuf, report: Option<SuccessReport>) {
        self.send(Event::JobCompleted { path, report });
    }

    /// 通知某个文件处理失败。
    pub fn job_failed(&self, path: PathBuf, error: impl Into<String>) {
        self.send(Event::JobFailed {
            path,
            error: error.into(),
        });
    }

    /// 通知用户取消了批次。
    pub fn cancelled(&self) {
        self.send(Event::Cancelled);
    }

    /// 发送任意事件；接收端已关闭时静默丢弃。
    pub fn send(&self, event: Event) {
        let _ = self.sender.send(event);
    }
}

/// 箭头 + 变化幅度：节省 → "↓ 54.8%"，增大 → "↑ 12.0%"，不变 → "=  0.0%"。
fn ratio_arrow(current_pct: f64) -> String {
    let saved = 100.0 - current_pct;
    let (symbol, percentage) = if saved > 0.0 {
        ("↓", saved)
    } else if saved < 0.0 {
        ("↑", -saved)
    } else {
        ("=", 0.0)
    };

    format!("{symbol}{percentage:>5.1}%")
}
