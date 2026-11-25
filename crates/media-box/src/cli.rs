//! media-box 的命令行定义：子命令、参数表与解析。

use clap::{Arg, ArgAction, ArgMatches, Command, command, value_parser};
use media_box_core::compressor::CompressOptions;
use media_box_core::resolution::Resolution;
use media_box_core::value::{EncoderPreset, FrameRatePerSecond, VmafScore};
use std::path::PathBuf;

/// 顶层命令：`media-box <子命令>`。
pub(crate) fn command() -> Command {
    command!()
        .version(env!("CARGO_PKG_VERSION"))
        .propagate_version(true)
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(video_compress_command())
}

/// `video-compress` 的参数表。
///
/// 这里的每个 `-x` 都必须和 README 的参数表一一对应，改参数请同时改两处。
///
/// | 短 | 长 | 值 | 默认 | 说明 |
/// |---|---|---|---|---|
/// | `-i` | `--input` | 路径，可重复 | 必填 | 源文件或目录 |
/// | `-r` | `--resolution` | `宽x高` | 跟随源 | 输出最大分辨率 |
/// | `-o` | `--output` | 目录 | 源目录 | 输出目录 |
/// | `-d` | `--depth` | `0..=255` | 0 | 目录递归深度 |
/// | `-f` | `--fps` | 浮点 | 跟随源 | 输出帧率上限 |
/// | `-p` | `--preset` | `0..=13` | 4 | SVT-AV1 preset |
/// | `-v` | `--vmaf` | `1..=100` | 91 | 目标 VMAF |
fn video_compress_command() -> Command {
    Command::new("video-compress")
        .about("压缩指定路径下的视频")
        .arg(
            Arg::new("input")
                .short('i')
                .long("input")
                .value_name("INPUT")
                .help("源视频路径，可以是文件或目录，可重复指定多个")
                .value_parser(value_parser!(PathBuf))
                // Append：每个 -i 收一个值，重复出现即累积。
                .action(ArgAction::Append)
                .required(true),
        )
        .arg(
            Arg::new("resolution")
                .short('r')
                .long("resolution")
                .value_name("RESOLUTION")
                .help("输出视频的最大分辨率，格式为“宽x高”（如 1920x1080）；源分辨率更高时按此缩放，默认跟随源视频")
                .value_parser(value_parser!(Resolution)),
        )
        .arg(
            Arg::new("output")
                .short('o')
                .long("output")
                .value_name("OUTPUT")
                .help("输出目录；未指定时输出到源视频所在目录")
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(
            Arg::new("depth")
                .short('d')
                .long("depth")
                .value_name("DEPTH")
                .help("目录扫描的最大递归深度；未指定时只处理指定目录本身（等价于 0），上限 255")
                .value_parser(value_parser!(u8)),
        )
        .arg(
            Arg::new("fps")
                .short('f')
                .long("fps")
                .value_name("FPS")
                .help("输出帧率上限；源帧率更高时会被限制，默认跟随源视频")
                .value_parser(value_parser!(FrameRatePerSecond)),
        )
        .arg(
            Arg::new("preset")
                .short('p')
                .long("preset")
                .value_name("PRESET")
                .help("编码 preset，范围 0-13，默认 4；越大越快且体积越大")
                .value_parser(value_parser!(EncoderPreset)),
        )
        .arg(
            Arg::new("vmaf")
                .short('v')
                .long("vmaf")
                .value_name("VMAF")
                .help("目标 VMAF 分数（v1 分数体系），默认 91")
                .value_parser(value_parser!(VmafScore)),
        )
}

pub(crate) fn parse() -> CompressOptions {
    let matches = command().get_matches();

    match matches.subcommand() {
        Some(("video-compress", sub_matches)) => parse_video_compress(sub_matches),
        _ => unreachable!("subcommand_required(true) 保证进入子命令"),
    }
}

fn parse_video_compress(matches: &ArgMatches) -> CompressOptions {
    // clap 的 required(true) 已保证至少一个输入。
    let inputs = matches
        .get_many::<PathBuf>("input")
        .map_or_default(|values| values.cloned().collect());

    CompressOptions::new(
        inputs,
        matches.get_one::<Resolution>("resolution").copied(),
        matches.get_one::<PathBuf>("output").cloned(),
        matches.get_one::<u8>("depth").copied().unwrap_or_default(),
        matches.get_one::<FrameRatePerSecond>("fps").copied(),
        matches
            .get_one::<EncoderPreset>("preset")
            .copied()
            .unwrap_or_default(),
        matches
            .get_one::<VmafScore>("vmaf")
            .copied()
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_from(args: &[&str]) -> CompressOptions {
        let matches = command().try_get_matches_from(args).expect("参数应合法");

        match matches.subcommand() {
            Some(("video-compress", sub_matches)) => parse_video_compress(sub_matches),
            _ => panic!("缺少子命令"),
        }
    }

    #[test]
    fn fills_defaults() {
        let args = parse_from(&["media-box", "video-compress", "-i", "."]);
        assert_eq!(args.depth, 0);
        assert_eq!(args.preset, EncoderPreset::default());
        assert_eq!(args.vmaf_score, VmafScore::default());
        assert_eq!(args.resolution, None);
        assert_eq!(args.output_dir, None);
        assert_eq!(args.fps, None);
    }

    #[test]
    fn every_documented_flag_is_wired() {
        let args = parse_from(&[
            "media-box",
            "video-compress",
            "-i",
            ".",
            "-r",
            "1280x720",
            "-o",
            "out",
            "-d",
            "2",
            "-f",
            "30",
            "-p",
            "6",
            "-v",
            "93",
        ]);

        assert_eq!(args.resolution, Some(Resolution::Hd));
        assert_eq!(args.output_dir, Some(PathBuf::from("out")));
        assert_eq!(args.depth, 2);
        assert_eq!(
            args.fps.map(media_box_core::FrameRatePerSecond::as_f64),
            Some(30.0)
        );
        assert_eq!(args.preset.as_u8(), 6);
        assert_eq!(args.vmaf_score.as_f64(), 93.0);
    }

    #[test]
    fn input_is_required_and_repeatable() {
        assert!(
            command()
                .try_get_matches_from(["media-box", "video-compress"])
                .is_err()
        );

        let args = parse_from(&["media-box", "video-compress", "-i", "a.mkv", "-i", "b.mkv"]);
        assert_eq!(args.inputs.len(), 2);
    }

    #[test]
    fn parses_resolution_and_keeps_unit_invariants() {
        let args = parse_from(&["media-box", "video-compress", "-i", ".", "-r", "1920x1080"]);
        assert_eq!(args.resolution, Some(Resolution::Fhd));

        assert!(
            command()
                .try_get_matches_from(["media-box", "video-compress", "-i", ".", "-r", "0x0"])
                .is_err()
        );
        assert!(
            command()
                .try_get_matches_from(["media-box", "video-compress", "-i", ".", "-v", "200"])
                .is_err()
        );
    }

    #[test]
    fn help_exposes_every_documented_flag() {
        // 与 README「参数」表一一对应：改参数时若漏掉帮助文本会在此失败。
        let help = video_compress_command().render_help().to_string();
        for flag in [
            "--input",
            "--resolution",
            "--output",
            "--depth",
            "--fps",
            "--preset",
            "--vmaf",
        ] {
            assert!(help.contains(flag), "帮助文本缺少 {flag}：{help}");
        }
    }

    #[test]
    fn unknown_subcommand_is_rejected() {
        assert!(
            command()
                .try_get_matches_from(["media-box", "nope"])
                .is_err()
        );
    }
}
