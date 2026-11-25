//! 逐文件转换结果与批量汇总的文本输出。

use crate::convert::ConvertOutcome;
use std::io::{self, Write};

/// 批量转换的汇总统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    pub total: usize,
    pub succeeded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub planned: usize,
    pub bytes: u64,
}

impl Summary {
    /// 由逐文件结果汇总。
    #[must_use]
    pub(crate) fn from_outcomes(outcomes: &[ConvertOutcome]) -> Self {
        let mut summary = Self {
            total: outcomes.len(),
            ..Self::default()
        };
        for outcome in outcomes {
            match outcome {
                ConvertOutcome::Success { bytes, .. } => {
                    summary.succeeded += 1;
                    summary.bytes += bytes;
                }
                ConvertOutcome::Skipped { .. } => summary.skipped += 1,
                ConvertOutcome::Planned { .. } => summary.planned += 1,
                ConvertOutcome::Failed { .. } => summary.failed += 1,
            }
        }
        summary
    }

    /// 输出一行汇总。
    pub(crate) fn print(&self, writer: &mut impl Write) -> io::Result<()> {
        writeln!(
            writer,
            "总计 {} 个：成功 {}，跳过 {}，失败 {}，计划 {}，输出 {} 字节",
            self.total, self.succeeded, self.skipped, self.failed, self.planned, self.bytes
        )
    }
}

/// 逐文件打印结果，最后输出汇总。
pub(crate) fn report(outcomes: &[ConvertOutcome], writer: &mut impl Write) -> io::Result<()> {
    for outcome in outcomes {
        match outcome {
            ConvertOutcome::Success { bytes, .. } => {
                writeln!(
                    writer,
                    "成功 {} -> {}（{} 字节）",
                    outcome.source().display(),
                    outcome.target().display(),
                    bytes
                )?;
            }
            ConvertOutcome::Skipped { .. } => {
                writeln!(
                    writer,
                    "跳过 {}（已存在 {}）",
                    outcome.source().display(),
                    outcome.target().display()
                )?;
            }
            ConvertOutcome::Planned { .. } => {
                writeln!(
                    writer,
                    "计划 {} -> {}",
                    outcome.source().display(),
                    outcome.target().display()
                )?;
            }
            ConvertOutcome::Failed { error, .. } => {
                writeln!(writer, "失败 {}：{}", outcome.source().display(), error)?;
            }
        }
    }

    Summary::from_outcomes(outcomes).print(writer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn summarizes_all_variants() {
        let outcomes = vec![
            ConvertOutcome::Success {
                source: PathBuf::from("a.png"),
                target: PathBuf::from("a.webp"),
                bytes: 10,
            },
            ConvertOutcome::Success {
                source: PathBuf::from("b.png"),
                target: PathBuf::from("b.webp"),
                bytes: 20,
            },
            ConvertOutcome::Skipped {
                source: PathBuf::from("c.png"),
                target: PathBuf::from("c.webp"),
            },
            ConvertOutcome::Failed {
                source: PathBuf::from("d.png"),
                target: PathBuf::new(),
                error: "解码失败".to_string(),
            },
            ConvertOutcome::Planned {
                source: PathBuf::from("e.png"),
                target: PathBuf::from("e.webp"),
            },
        ];

        let summary = Summary::from_outcomes(&outcomes);
        assert_eq!(summary.total, 5);
        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.skipped, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.planned, 1);
        assert_eq!(summary.bytes, 30);
    }
}
