use crate::convert::ConvertOptions;
use crate::format::TargetFormat;
use clap::{Arg, ArgAction, Command, command, value_parser};
use std::path::PathBuf;
use std::str::FromStr;

/// 有损编码质量解析错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QualityError {
    #[error("质量值 {value} 超出范围，必须在 1 到 100 之间。")]
    OutOfRange { value: u8 },
    #[error("无法解析质量值 {0}")]
    ParseInt(#[from] std::num::ParseIntError),
}

/// 有损编码质量，取值 1..=100。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Quality(u8);

impl Quality {
    pub fn new(value: u8) -> Result<Self, QualityError> {
        if (1..=100).contains(&value) {
            Ok(Self(value))
        } else {
            Err(QualityError::OutOfRange { value })
        }
    }

    pub fn as_u8(self) -> u8 {
        self.0
    }
}

impl Default for Quality {
    fn default() -> Self {
        Self(85)
    }
}

impl FromStr for Quality {
    type Err = QualityError;

    fn from_str(text: &str) -> Result<Self, QualityError> {
        Self::new(text.parse()?)
    }
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
    pub list_formats: bool,
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
        }
    }
}

/// `image-convert` 的参数表。
///
/// | 短 | 长 | 值 | 默认 | 说明 |
/// |---|---|---|---|---|
/// | `-i` | `--input` | 路径，可重复 | 必填 | 源图片文件或目录 |
/// | `-o` | `--output` | 目录 | 源所在目录 | 输出目录 |
/// | `-f` | `--format` | 目标格式 | 必填 | 见 `--help` 的格式清单 |
/// | `-q` | `--quality` | `1..=100` | 85 | JPEG/AVIF 有损质量 |
/// | `-d` | `--depth` | `0..=255` | 0 | 目录递归深度 |
/// | `-j` | `--jobs` | `N` | CPU 逻辑核数 | 并发任务数 |
/// | `--overwrite` | `--overwrite` | 开关 | 关 | 覆盖已存在输出 |
/// | `--dry-run` | `--dry-run` | 开关 | 关 | 只列出，不写盘 |
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
                .help("目标格式：png/jpg/webp/avif/gif/tif/bmp/ico/tga/pnm/qoi/hdr/exr/ff/dds；heic/jxl/jp2 等扩展格式仅支持解码，用 --list-formats 查看能力矩阵")
                .value_parser(value_parser!(TargetFormat))
                .required_unless_present("list_formats"),
        )
        .arg(
            Arg::new("list_formats")
                .long("list-formats")
                .help("打印输入/输出格式能力矩阵后退出")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("quality")
                .short('q')
                .long("quality")
                .value_name("1..=100")
                .help("JPEG/AVIF 有损质量，默认 85；无损格式忽略")
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
            list_formats: matches.get_flag("list_formats"),
        }
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
            "avif",
            "-q",
            "70",
            "-d",
            "2",
            "-j",
            "4",
            "--overwrite",
            "--dry-run",
        ]);
        assert_eq!(args.output_dir, Some(PathBuf::from("out")));
        assert_eq!(args.format, "avif".parse::<TargetFormat>().unwrap());
        assert_eq!(args.quality.as_u8(), 70);
        assert_eq!(args.depth, 2);
        assert_eq!(args.jobs, 4);
        assert!(args.overwrite);
        assert!(args.dry_run);
    }

    #[test]
    fn input_is_required_and_repeatable() {
        assert!(
            command()
                .try_get_matches_from(["image-convert", "-f", "png"])
                .is_err()
        );
        assert!(
            command()
                .try_get_matches_from(["image-convert", "-i", "."])
                .is_err()
        );

        let args = parse_from(&["image-convert", "-i", "a.png", "-i", "b.png", "-f", "jpg"]);
        assert_eq!(args.inputs.len(), 2);
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
    fn decode_only_targets_parse() {
        for text in [
            "jxl", "heic", "heif", "jp2", "j2k", "j2c", "pcx", "sgi", "icns", "wbmp", "xpm", "ora",
            "otb", "xbm",
        ] {
            let args = parse_from(&["image-convert", "-i", ".", "-f", text]);
            assert!(args.format.is_decode_only(), "{text} 应解析为仅解码格式");
        }
    }
}
