//! 流水线阶段定义与成功 / 失败 / 警告报告模型。

use crate::byte_size::ByteSize;
use crate::format::format_duration_fixed;
use crate::progress::ProgressSnapshot;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// 单个视频的流水线阶段，枚举顺序即执行顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Metadata,
    SceneSampling,
    FilmGrain,
    CrfSearch,
    Encode,
}

impl Stage {
    pub const ALL: [Stage; 5] = [
        Stage::Metadata,
        Stage::SceneSampling,
        Stage::FilmGrain,
        Stage::CrfSearch,
        Stage::Encode,
    ];

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

    fn output_src_size_percentage(&self) -> f32 {
        let src = self.src_size.as_u64();

        if src == 0 {
            return 100.0;
        }

        self.output_size.as_u64() as f32 / src as f32 * 100.0
    }

    fn compress_src_duration_percentage(&self) -> f32 {
        if self.src_duration.is_zero() {
            return 100.0;
        }

        self.compress_duration.div_duration_f32(self.src_duration) * 100.0
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
            ratio_arrow(self.output_src_size_percentage() as f64),
            format_duration_fixed(self.src_duration),
            format_duration_fixed(self.compress_duration),
            ratio_arrow(self.compress_src_duration_percentage() as f64),
        )
    }
}

/// 流水线事件：阶段推进、进度、结果与汇总，由 Reporter 发送给渲染方。
pub enum Event {
    JobStarted(PathBuf),
    Stage {
        stage: Stage,
        detail: Option<String>,
    },
    Note(String),
    Progress(ProgressSnapshot),
    JobCompleted {
        path: PathBuf,
        report: Option<SuccessReport>,
    },
    JobFailed {
        path: PathBuf,
        error: String,
    },
    Warning(String),
    Cancelled,
    Finished,
}

/// 事件发送方：把压缩过程中的状态变化发给 UI。
#[derive(Clone)]
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

    pub fn job_completed(&self, path: PathBuf, report: Option<SuccessReport>) {
        self.send(Event::JobCompleted { path, report });
    }

    pub fn job_failed(&self, path: PathBuf, error: impl Into<String>) {
        self.send(Event::JobFailed {
            path,
            error: error.into(),
        });
    }

    pub fn cancelled(&self) {
        self.send(Event::Cancelled);
    }

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
