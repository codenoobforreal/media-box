//! 终端仪表盘渲染：阶段进度、当前文件与结束汇总。

use anyhow::{Result, anyhow};
use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate, size};
use std::fmt::Write as _;
use std::io::{BufWriter, IsTerminal, Write, stdout};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::format::format_duration_fixed;
use crate::progress::ProgressSnapshot;
use crate::report::{Event, Reporter, Stage, SuccessReport};

/// 界面刷新间隔，同时决定转圈动画的帧率。
const RENDER_INTERVAL: Duration = Duration::from_millis(120);

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

const MIN_RULE_WIDTH: u16 = 24;
const MAX_RULE_WIDTH: u16 = 120;

/// 终端仪表盘：持有事件发送端与渲染线程句柄。
#[derive(Debug)]
pub struct Ui {
    reporter: Reporter,
    renderer: JoinHandle<()>,
}

impl Ui {
    /// 启动渲染线程，返回可发送事件的界面句柄。
    pub fn install(queue: &[PathBuf]) -> Self {
        let (sender, receiver) = channel();
        let renderer = spawn_renderer(receiver, queue.to_vec());

        Self {
            reporter: Reporter::new(sender),
            renderer,
        }
    }

    /// 取事件发送端。
    pub fn reporter(&self) -> &Reporter {
        &self.reporter
    }

    /// 通知渲染线程收尾，并等待总结页输出完成。
    ///
    /// # Errors
    ///
    /// 渲染线程 panic 时返回错误。
    pub fn shutdown(self) -> Result<()> {
        self.reporter.send(Event::Finished);
        drop(self.reporter);

        self.renderer
            .join()
            .map_err(|_| anyhow!("终端界面线程异常退出"))
    }
}

fn spawn_renderer(receiver: Receiver<Event>, queue: Vec<PathBuf>) -> JoinHandle<()> {
    thread::spawn(move || {
        // 管道 / 重定向下不写光标控制序列，避免把日志文件刷成一片转义符；
        // 只在结束时打印总结。
        let interactive = stdout().is_terminal();
        let mut context = RenderContext::new(queue, interactive);
        let mut writer = BufWriter::new(stdout());
        let mut frame = 0usize;

        loop {
            match receiver.recv_timeout(RENDER_INTERVAL) {
                // 批次结束与发送端断开都意味着没有后续事件，渲染最后一张总结页即可。
                Ok(Event::Finished) | Err(RecvTimeoutError::Disconnected) => {
                    context.finished = true;
                    context.render(&mut writer, SPINNER[0]);
                    break;
                }
                Ok(event) => {
                    frame += 1;
                    context.apply(event);
                    context.render(&mut writer, SPINNER[frame % SPINNER.len()]);
                }
                Err(RecvTimeoutError::Timeout) => {
                    frame += 1;
                    context.render(&mut writer, SPINNER[frame % SPINNER.len()]);
                }
            }
        }

        let _ = writer.flush();
    })
}

#[derive(Debug)]
struct RenderContext {
    queue: Vec<PathBuf>,
    total: usize,
    success: usize,
    failed: usize,
    current_path: Option<PathBuf>,
    stage: Option<Stage>,
    detail: Option<String>,
    progress: Option<ProgressSnapshot>,
    success_reports: Vec<SuccessReport>,
    failures: Vec<(PathBuf, String)>,
    warnings: Vec<String>,
    cancelled: bool,
    finished: bool,
    started_at: Instant,
    /// stdout 是否为终端；非终端只输出最终总结，不写光标控制序列。
    interactive: bool,
}

impl RenderContext {
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "渲染上下文的构造器，interactive 表示 stdout 是否为终端"
    )]
    fn new(queue: Vec<PathBuf>, interactive: bool) -> Self {
        let total = queue.len();

        Self {
            queue,
            total,
            success: 0,
            failed: 0,
            current_path: None,
            stage: None,
            detail: None,
            progress: None,
            success_reports: Vec::new(),
            failures: Vec::new(),
            warnings: Vec::new(),
            cancelled: false,
            finished: false,
            started_at: Instant::now(),
            interactive,
        }
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::JobStarted(path) => self.begin_job(path),
            Event::Stage { stage, detail } => {
                self.stage = Some(stage);
                self.detail = detail;
                self.progress = None;
            }
            Event::Note(note) => self.detail = Some(note),
            Event::Progress(snapshot) => self.progress = Some(snapshot),
            Event::JobCompleted { path, report } => {
                self.finish_job(&path);
                if let Some(report) = report {
                    self.success_reports.push(report);
                }
                self.success += 1;
            }
            Event::JobFailed { path, error } => {
                self.finish_job(&path);
                self.failures.push((path, error));
                self.failed += 1;
            }
            Event::Warning(message) => self.warnings.push(message),
            Event::Cancelled => self.cancelled = true,
            Event::Finished => self.finished = true,
        }
    }

    fn begin_job(&mut self, path: PathBuf) {
        self.current_path = Some(path);
        self.stage = None;
        self.detail = None;
        self.progress = None;
    }

    fn finish_job(&mut self, path: &Path) {
        if let Some(index) = self
            .queue
            .iter()
            .position(|queued| queued.as_path() == path)
        {
            self.queue.swap_remove(index);
        }

        self.current_path = None;
        self.stage = None;
        self.detail = None;
        self.progress = None;
    }

    fn done(&self) -> usize {
        self.success + self.failed
    }

    fn render<W: Write>(&self, writer: &mut W, spinner: &str) {
        if self.interactive {
            let _ = queue!(
                writer,
                BeginSynchronizedUpdate,
                MoveTo(0, 0),
                Clear(ClearType::FromCursorDown),
            );
        }

        if self.finished {
            self.render_summary(writer);
        } else if self.interactive {
            self.render_dashboard(writer, spinner);
        }

        if self.interactive {
            let _ = queue!(writer, EndSynchronizedUpdate);
        }
        let _ = writer.flush();
    }

    fn render_dashboard<W: Write>(&self, writer: &mut W, spinner: &str) {
        let rule = "─".repeat(usize::from(rule_width()));
        let header = format!(
            "media-box  {}/{}  {}",
            self.done(),
            self.total,
            self.current_name()
        );

        styled(writer, Color::Cyan, true, &header);
        styled(writer, Color::DarkGrey, false, &rule);

        for stage in Stage::ALL {
            self.render_stage_row(writer, stage, spinner);
        }

        if let Some(progress) = &self.progress {
            styled(writer, Color::Yellow, false, &format!("  {progress}"));
        }

        styled(writer, Color::DarkGrey, false, &rule);

        let pending = self
            .queue
            .len()
            .saturating_sub(usize::from(self.current_path.is_some()));
        let mut footer = format!(
            "成功 {} · 失败 {} · 待处理 {pending} · 已用 {}",
            self.success,
            self.failed,
            format_duration_fixed(self.started_at.elapsed())
        );

        if !self.warnings.is_empty() {
            let _ = write!(footer, " · 警告 {}", self.warnings.len());
        }

        if self.cancelled {
            footer.push_str(" · 已取消");
        }

        styled(writer, Color::DarkGrey, false, &footer);

        if !self.cancelled {
            styled(
                writer,
                Color::DarkGrey,
                false,
                "按 Ctrl+C 取消；再按一次强制退出",
            );
        }
    }

    fn render_stage_row<W: Write>(&self, writer: &mut W, stage: Stage, spinner: &str) {
        let active = self.stage == Some(stage);
        let completed = self.stage.is_some_and(|current| stage < current);
        let (marker, color, bold) = if active {
            (spinner, Color::Yellow, true)
        } else if completed {
            ("✓", Color::Green, false)
        } else {
            ("·", Color::DarkGrey, false)
        };

        let mut text = format!("{marker} {}", stage.label());

        if active && let Some(detail) = &self.detail {
            text.push_str("  · ");
            text.push_str(detail);
        }

        let _ = write!(writer, "  ");
        styled(writer, color, bold, &text);
    }

    fn render_summary<W: Write>(&self, writer: &mut W) {
        let rule = "─".repeat(usize::from(rule_width()));
        let header = format!(
            "media-box · 完成  · 已用 {}",
            format_duration_fixed(self.started_at.elapsed())
        );

        styled(writer, Color::Cyan, true, &header);
        styled(writer, Color::DarkGrey, false, &rule);
        styled(
            writer,
            Color::DarkGrey,
            false,
            &format!(
                "成功 {} · 失败 {} · 未处理 {}",
                self.success,
                self.failed,
                self.queue.len()
            ),
        );
        let _ = writeln!(writer);

        if !self.success_reports.is_empty() {
            styled(writer, Color::Green, true, "=== 成功 ===");

            for report in &self.success_reports {
                let _ = writeln!(writer, "{report}");
            }

            let _ = writeln!(writer);
        }

        if !self.failures.is_empty() {
            styled(writer, Color::Red, true, "=== 失败 ===");

            for (path, error) in &self.failures {
                let _ = writeln!(writer, "{}: {error}", path.display());
            }

            let _ = writeln!(writer);
        }

        if !self.warnings.is_empty() {
            styled(writer, Color::Yellow, true, "=== 警告 ===");

            for warning in &self.warnings {
                let _ = writeln!(writer, "{warning}");
            }

            let _ = writeln!(writer);
        }

        if !self.queue.is_empty() {
            styled(writer, Color::DarkGrey, true, "=== 已取消/未处理 ===");

            for path in &self.queue {
                let _ = writeln!(writer, "{}", path.display());
            }
        }
    }

    fn current_name(&self) -> String {
        self.current_path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(
                || String::from("等待中"),
                |name| name.to_string_lossy().into_owned(),
            )
    }
}

#[expect(
    clippy::fn_params_excessive_bools,
    reason = "渲染辅助函数，bold 只是排版开关，抽出枚举反而更绕"
)]
fn styled<W: Write>(writer: &mut W, color: Color, bold: bool, text: &str) {
    let _ = queue!(writer, SetForegroundColor(color));

    if bold {
        let _ = queue!(writer, SetAttribute(Attribute::Bold));
    }

    let _ = writeln!(writer, "{text}");
    let _ = queue!(writer, SetAttribute(Attribute::Reset), ResetColor);
}

fn rule_width() -> u16 {
    size().map_or(MAX_RULE_WIDTH, |(width, _)| {
        width.clamp(MIN_RULE_WIDTH, MAX_RULE_WIDTH)
    })
}
