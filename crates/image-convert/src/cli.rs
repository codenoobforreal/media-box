//! image-convert 的命令行定义：参数表、解析与校验。

use crate::convert::ConvertOptions;
use crate::format::{TargetFormat, image_extensions};
use crate::metadata::MetadataMode;
use clap::{Arg, ArgAction, Command, command, value_parser};
use media_box_core::value::Quality;
use std::path::PathBuf;

/// 参数组合校验错误：可在调用方按类型分支，展示文本只在 report 层生成。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("缺少输入：用 -i 指定源图片文件或目录")]
    MissingInput,
    #[error("缺少目标格式：用 -f 指定输出格式")]
    MissingFormat,
}

/// 命令行参数。
#[non_exhaustive]
#[derive(Debug, PartialEq, Clone)]
pub struct CliArgs {
    pub inputs: Vec<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub format: TargetFormat,
    pub quality: Quality,
    pub depth: u8,
    pub jobs: usize,
    pub overwrite: bool,
    pub dry_run: bool,
    pub optimize: bool,
    pub metadata: MetadataMode,
    pub list_formats: bool,
    /// -f 是否由用户显式给出（--meta list 等模式可不给出）。
    pub format_explicit: bool,
}

impl From<CliArgs> for ConvertOptions {
    fn from(args: CliArgs) -> Self {
        Self {
            inputs: args.inputs,
            output_dir: args.output_dir,
            format: args.format,
            quality: args.quality.as_u8(),
            depth: args.depth,
            jobs: args.jobs,
            overwrite: args.overwrite,
            dry_run: args.dry_run,
            optimize: args.optimize,
            metadata: args.metadata,
        }
    }
}

/// `image-convert` 的参数表。
///
/// | 短 | 长 | 值 | 默认 | 说明 |
/// |---|---|---|---|---|
/// | `-i` | `--input` | 路径，可重复 | 必填 | 源图片文件或目录 |
/// | `-o` | `--output` | 目录 | 源所在目录 | 输出目录 |
/// | `-f` | `--format` | 目标格式 | 转换时必填 | 见 `--help` 的格式清单 |
/// | `-q` | `--quality` | `1..=100` | 85 | JPEG 有损质量 |
/// | `-d` | `--depth` | `0..=255` | 0 | 目录递归深度 |
/// | `-j` | `--jobs` | `N` | CPU 逻辑核数 | 并发任务数 |
/// | `--overwrite` | `--overwrite` | 开关 | 关 | 覆盖已存在输出 |
/// | `--dry-run` | `--dry-run` | 开关 | 关 | 只列出，不写盘 |
/// | `--optimize` | `--optimize` | 开关 | 关 | PNG 输出启用最高无损压缩 |
/// | `--meta` | `--meta` | keep/strip/list | keep | 源元数据保留、剔除或只列出摘要 |
pub fn command() -> Command {
    command!()
        .version(env!("CARGO_PKG_VERSION"))
        .about("批量转换图片格式（纯 Rust，无外部编码器依赖）")
        .arg(
            Arg::new("input")
                .short('i')
                .long("input")
                .value_name("INPUT")
                .help("源图片路径，可以是文件或目录，可重复指定多个")
                .value_parser(value_parser!(PathBuf))
                .action(ArgAction::Append)
                .required_unless_present("list_formats"),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .value_name("DIR")
                .help("输出目录；缺省时输出到源文件所在目录")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("format")
                .short('f')
                .long("format")
                .value_name("FORMAT")
                .help(format!(
                    "目标格式：{}，用 --list-formats 查看完整清单",
                    image_extensions().join("/")
                ))
                .value_parser(value_parser!(TargetFormat)),
        )
        .arg(
            Arg::new("list_formats")
                .long("list-formats")
                .help("打印输出格式清单后退出")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("quality")
                .short('q')
                .long("quality")
                .value_name("1..=100")
                .help("JPEG 有损质量，默认 85；无损格式忽略")
                .value_parser(value_parser!(Quality))
                .default_value("85"),
        )
        .arg(
            Arg::new("depth")
                .short('d')
                .long("depth")
                .value_name("0..=255")
                .help("目录递归深度，0 表示只看当前目录，默认 0")
                .value_parser(value_parser!(u8))
                .default_value("0"),
        )
        .arg(
            Arg::new("jobs")
                .short('j')
                .long("jobs")
                .value_name("N")
                .help("并发转换任务数，默认等于 CPU 逻辑核数")
                .value_parser(value_parser!(usize)),
        )
        .arg(
            Arg::new("overwrite")
                .long("overwrite")
                .help("允许覆盖已存在的输出文件，默认跳过")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("dry_run")
                .long("dry-run")
                .help("只列出将转换的文件，不写盘")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("optimize")
                .long("optimize")
                .help("PNG 输出时启用最高无损压缩（默认关闭）")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("metadata")
                .long("meta")
                .value_name("MODE")
                .help("元数据处理：keep（默认，尽力保留）、strip（不携带）、list（只列出摘要）")
                .value_parser(value_parser!(MetadataMode))
                .default_value("keep"),
        )
}

/// 默认并发数：CPU 逻辑核数，失败时退回 1。
fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |parallelism| parallelism.get())
        .max(1)
}

impl CliArgs {
    /// 由 clap 匹配结果构造命令行参数。
    pub fn from_matches(matches: &clap::ArgMatches) -> Self {
        Self {
            inputs: matches
                .get_many::<PathBuf>("input")
                .map(|values| values.cloned().collect())
                .unwrap_or_default(),
            output_dir: matches.get_one::<PathBuf>("output").cloned(),
            // --list-formats 模式下 -f 可省略，此时该字段不参与任何转换。
            format: matches
                .get_one::<TargetFormat>("format")
                .copied()
                .unwrap_or_default(),
            quality: *matches
                .get_one::<Quality>("quality")
                .expect("--quality 有默认值"),
            depth: *matches.get_one::<u8>("depth").expect("--depth 有默认值"),
            jobs: matches
                .get_one::<usize>("jobs")
                .copied()
                .unwrap_or_else(default_jobs),
            overwrite: matches.get_flag("overwrite"),
            dry_run: matches.get_flag("dry_run"),
            optimize: matches.get_flag("optimize"),
            metadata: *matches
                .get_one::<MetadataMode>("metadata")
                .expect("--meta 有默认值"),
            list_formats: matches.get_flag("list_formats"),
            format_explicit: matches.contains_id("format"),
        }
    }

    /// 校验参数组合，返回错误信息。
    ///
    /// --list-formats 与 --meta list 只读取信息，不需要 -f；其余转换必须显式给出 -f。
    pub fn validate(&self) -> std::result::Result<(), ValidationError> {
        if self.list_formats || self.metadata == MetadataMode::List {
            return Ok(());
        }
        if self.inputs.is_empty() {
            return Err(ValidationError::MissingInput);
        }
        if !self.format_explicit {
            return Err(ValidationError::MissingFormat);
        }
        Ok(())
    }
}

/// 解析命令行参数。
pub fn parse() -> CliArgs {
    CliArgs::from_matches(&command().get_matches())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_from(args: &[&str]) -> CliArgs {
        CliArgs::from_matches(&command().try_get_matches_from(args).unwrap())
    }

    #[test]
    fn fills_defaults() {
        let args = parse_from(&["image-convert", "-i", ".", "-f", "webp"]);
        assert_eq!(args.quality, Quality::default());
        assert_eq!(args.depth, 0);
        assert!(args.jobs >= 1);
        assert!(!args.overwrite);
        assert!(!args.dry_run);
        assert!(!args.optimize);
        assert_eq!(args.metadata, MetadataMode::Keep);
        assert_eq!(args.output_dir, None);
    }

    #[test]
    fn every_documented_flag_is_wired() {
        let args = parse_from(&[
            "image-convert",
            "-i",
            ".",
            "-o",
            "out",
            "-f",
            "webp",
            "-q",
            "70",
            "-d",
            "2",
            "-j",
            "4",
            "--overwrite",
            "--dry-run",
            "--optimize",
        ]);
        assert_eq!(args.output_dir, Some(PathBuf::from("out")));
        assert_eq!(args.format, "webp".parse::<TargetFormat>().unwrap());
        assert_eq!(args.quality.as_u8(), 70);
        assert_eq!(args.depth, 2);
        assert_eq!(args.jobs, 4);
        assert!(args.overwrite);
        assert!(args.dry_run);
        assert!(args.optimize);
    }

    #[test]
    fn input_is_required_and_repeatable() {
        // 没有 -i 时 clap 直接拒绝。
        assert!(
            command()
                .try_get_matches_from(["image-convert", "-f", "png"])
                .is_err()
        );

        // -f 的必填由 validate 保证；--meta list 模式可省略。
        let list = parse_from(&["image-convert", "-i", ".", "--meta", "list"]);
        assert!(list.validate().is_ok());

        let missing_format = parse_from(&["image-convert", "-i", "."]);
        assert!(missing_format.validate().is_err());

        let args = parse_from(&["image-convert", "-i", "a.png", "-i", "b.png", "-f", "jpg"]);
        assert_eq!(args.inputs.len(), 2);
        assert!(args.validate().is_ok());
    }

    #[test]
    fn format_help_lists_every_supported_extension() {
        // 帮助文案由格式表派生：新增格式却漏了文案会在此失败。
        let help = command().render_help().to_string();
        for extension in image_extensions() {
            assert!(help.contains(extension), "帮助缺少 {extension}：{help}");
        }
    }

    #[test]
    fn unknown_format_is_rejected() {
        assert!(
            command()
                .try_get_matches_from(["image-convert", "-i", ".", "-f", "nope"])
                .is_err()
        );
    }

    #[test]
    fn quality_out_of_range_is_rejected() {
        for value in ["0", "101", "255"] {
            assert!(
                command()
                    .try_get_matches_from(["image-convert", "-i", ".", "-f", "jpg", "-q", value])
                    .is_err()
            );
        }
    }

    #[test]
    fn depth_is_a_u8() {
        assert!(
            command()
                .try_get_matches_from(["image-convert", "-i", ".", "-f", "png", "-d", "256"])
                .is_err()
        );
        assert!(
            command()
                .try_get_matches_from(["image-convert", "-i", ".", "-f", "png", "-d", "255"])
                .is_ok()
        );
    }

    #[test]
    fn list_formats_needs_no_input_or_format() {
        let args = parse_from(&["image-convert", "--list-formats"]);
        assert!(args.list_formats);
        assert!(args.inputs.is_empty());
        // -f 缺省时字段取默认值，不参与转换。
        assert_eq!(args.format, TargetFormat::default());

        // 没有 --list-formats 时，-i 和 -f 仍然必填。
        assert!(
            command()
                .try_get_matches_from(["image-convert", "--dry-run"])
                .is_err()
        );
    }

    #[test]
    fn metadata_mode_is_parsed() {
        let keep = parse_from(&["image-convert", "-i", ".", "-f", "png"]);
        assert_eq!(keep.metadata, MetadataMode::Keep);

        let strip = parse_from(&["image-convert", "-i", ".", "-f", "png", "--meta", "strip"]);
        assert_eq!(strip.metadata, MetadataMode::Strip);

        let list = parse_from(&["image-convert", "-i", ".", "-f", "png", "--meta", "list"]);
        assert_eq!(list.metadata, MetadataMode::List);
    }

    #[test]
    fn invalid_metadata_mode_is_rejected() {
        assert!(
            command()
                .try_get_matches_from(["image-convert", "-i", ".", "-f", "png", "--meta", "drop"])
                .is_err()
        );
    }
}
