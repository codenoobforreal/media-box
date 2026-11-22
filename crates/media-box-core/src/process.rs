use crate::interrupter::{Cancelled, Canceller};
use anyhow::{Context, Result};
use std::io::{BufReader, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// 子进程输出的轮询间隔，决定取消的响应延迟。
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 保留的 stderr 行数上限；超出后丢弃最旧的行。
const MAX_STDERR_LINES: usize = 200;

/// 单行字节上限；超长行被截断，避免 read_until 让缓冲无限增长。
const MAX_LINE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

struct Line {
    stream: Stream,
    text: String,
}

#[derive(Debug)]
pub struct ProcessOutput {
    status: ExitStatus,
    stderr_lines: Vec<String>,
}

impl ProcessOutput {
    pub fn success(&self) -> bool {
        self.status.success()
    }

    pub fn stderr_lines(&self) -> &[String] {
        &self.stderr_lines
    }

    pub fn failure_message(&self) -> String {
        let mut error_lines: Vec<&str> = Vec::new();

        for line in &self.stderr_lines {
            if is_error_line(line) {
                error_lines.push(line);
            }
        }

        if !error_lines.is_empty() {
            // 只保留尾部若干行：真正的失败原因通常在最后。
            let tail = error_lines[error_lines.len().saturating_sub(20)..].join("\n");
            return tail;
        }

        if !self.stderr_lines.is_empty() {
            let tail = self.stderr_lines[self.stderr_lines.len().saturating_sub(20)..].join("\n");
            return tail;
        }

        match self.status.code() {
            Some(code) => format!("进程以退出码 {code} 结束，且没有任何输出"),
            None => String::from("进程被信号终止，且没有任何输出"),
        }
    }
}

/// ffmpeg 的错误行没有统一前缀，使用关键字判断。
pub fn is_error_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("error")
        || lower.contains("invalid")
        || lower.contains("failed")
        || lower.contains("unable")
}

pub fn display_command(command: &Command) -> String {
    std::iter::once(command.get_program().to_string_lossy().into_owned())
        .chain(
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned()),
        )
        .collect::<Vec<String>>()
        .join(" ")
}

/// 执行命令，逐行回调输出；检测到取消时杀掉子进程并返回 [`Cancelled`]。
pub fn run(
    mut command: Command,
    canceller: &Canceller,
    mut on_line: impl FnMut(Stream, &str),
) -> Result<ProcessOutput> {
    canceller.check()?;

    let command_string = display_command(&command);

    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child =
        spawn_child(&mut command).with_context(|| format!("启动子进程失败：{command_string}"))?;

    let (sender, receiver) = channel::<Line>();
    let stdout_reader = spawn_line_reader(Stream::Stdout, child.stdout.take(), sender.clone());
    let stderr_reader = spawn_line_reader(Stream::Stderr, child.stderr.take(), sender.clone());
    drop(sender);

    let mut stderr_lines = Vec::new();

    loop {
        if canceller.is_cancelled() {
            return Err(cancel(&mut child, receiver, stdout_reader, stderr_reader));
        }

        match receiver.recv_timeout(POLL_INTERVAL) {
            Ok(line) => {
                on_line(line.stream, &line.text);

                if line.stream == Stream::Stderr {
                    stderr_lines.push(line.text);
                    // 诊断只需要尾部：libvmaf 的 -v info 输出可达万行，
                    // 无上限会让失败信息膨胀到不可读。分批裁剪避免每次 push 都 O(n)。
                    if stderr_lines.len() > MAX_STDERR_LINES * 2 {
                        stderr_lines.drain(..MAX_STDERR_LINES);
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    if canceller.is_cancelled() {
        return Err(cancel(&mut child, receiver, stdout_reader, stderr_reader));
    }

    let status = child
        .wait()
        .with_context(|| format!("等待子进程结束失败：{command_string}"))?;

    join_quietly(stdout_reader);
    join_quietly(stderr_reader);

    Ok(ProcessOutput {
        status,
        stderr_lines,
    })
}

fn cancel(
    child: &mut Child,
    receiver: Receiver<Line>,
    stdout_reader: JoinHandle<()>,
    stderr_reader: JoinHandle<()>,
) -> anyhow::Error {
    terminate(child);
    drop(receiver);
    join_quietly(stdout_reader);
    join_quietly(stderr_reader);
    Cancelled.into()
}

fn spawn_line_reader(
    stream: Stream,
    reader: Option<impl Read + Send + 'static>,
    sender: Sender<Line>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let Some(reader) = reader else {
            return;
        };

        let mut reader = BufReader::new(reader);
        let mut buffer = Vec::with_capacity(256);
        let mut chunk = [0u8; 8 * 1024];

        loop {
            // 分块读取而不是 read_until：后者会把一整行读进缓冲，
            // 子进程写出永不换行的超长内容时内存无上限增长。
            let read = match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };

            for &byte in &chunk[..read] {
                if byte != b'\n' {
                    if buffer.len() < MAX_LINE_BYTES {
                        buffer.push(byte);
                    }
                    continue;
                }

                if !send_line(&mut buffer, stream, &sender) {
                    return;
                }
            }
        }

        // 末行可能没有换行符。
        if !buffer.is_empty() {
            let _ = send_line(&mut buffer, stream, &sender);
        }
    })
}

/// 发送一整行并清空缓冲；返回 false 表示通道已断开，调用方应停止读取。
fn send_line(buffer: &mut Vec<u8>, stream: Stream, sender: &Sender<Line>) -> bool {
    let line = String::from_utf8_lossy(buffer);
    let text = line.trim_end_matches(['\n', '\r']).to_owned();
    buffer.clear();

    sender.send(Line { stream, text }).is_ok()
}

fn terminate(child: &mut Child) {
    // 先杀整棵进程树，再回收直接子进程。
    //
    // 只 `child.kill()` 杀不掉 ffmpeg 遗留的子进程（某些 filter / 硬件解码后端）
    // 它们会继续持有 stdout/stderr 管道的写端：读线程卡在 `read_until` 上，EOF
    // 永远不来，`join_quietly` 在取消路径永久阻塞。杀树后所有写端关闭，读线程
    // 正常读到 EOF 退出，join 安全返回。
    kill_process_tree(child.id());

    let _ = child.kill();
    let _ = child.wait();
}

/// Unix 下让子进程成为新进程组组长，取消时才能终止整个进程组。
fn spawn_child(command: &mut Command) -> std::io::Result<Child> {
    cfg_select! {
        unix => {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
            command.spawn()
        },
        _ => command.spawn(),
    }
}

fn kill_process_tree(pid: u32) {
    cfg_select! {
           windows => {
               // `/T` 杀整棵树、`/F` 强制。进程已退出时返回非零，可忽略。
               let _ = Command::new("taskkill")
                   .args(["/F", "/T", "/PID"])
                   .arg(pid.to_string())
                   .output();
           },
           unix => {
               // process_group(0) 令子进程 pgid == pid；负 pid 表示整个进程组。
               // 系统 kill 不是 Cargo 依赖，不参与本项目编译。
               let _ = Command::new("kill")
                   .arg("-KILL")
                   .arg(format!("-{pid}"))
                   .output();
           },
           _ => {
               // 其它平台由 terminate 中的 child.kill() 兜底。
           },
    }
}

fn join_quietly(handle: JoinHandle<()>) {
    let _ = handle.join();
}
