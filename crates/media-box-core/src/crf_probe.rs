//! CRF 搜索的测量实现：用 ffmpeg + libvmaf 在场景样本上测分，并适配搜索日志。

use crate::crf_search::{SampleMeasurer, SearchJournal};
use crate::encoder::{Av1EncodeArgs, BitrateReduceSettings};
use crate::ffmpeg_args::FfmpegCommand;
use crate::interrupter::{Cancelled, Canceller};
use crate::process;
use crate::report::{Reporter, Stage};
use crate::scene_sampler::SceneSample;
use crate::stats::f64_median;
use crate::value::{ConstantRateFactor, EncoderPreset, FilmGrain};
use crate::vmaf::{self, VmafMeasureMode};
use anyhow::{Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 构造 ProbeContext 的入参集合，避免构造函数参数过多。
pub struct ProbeContextParams<'a> {
    pub input: &'a Path,
    pub base_output: &'a Path,
    pub settings: BitrateReduceSettings,
    pub preset: EncoderPreset,
    pub film_grain: FilmGrain,
    pub samples: &'a [SceneSample],
    pub canceller: &'a Canceller,
    pub reporter: &'a Reporter,
}

/// 一次 CRF 搜索所需的全部输入与状态。
pub struct ProbeContext<'a> {
    input: &'a Path,
    base_output: &'a Path,
    settings: BitrateReduceSettings,
    preset: EncoderPreset,
    film_grain: FilmGrain,
    samples: &'a [SceneSample],
    canceller: &'a Canceller,
    reporter: &'a Reporter,
}

impl<'a> ProbeContext<'a> {
    /// 由入参集合构造上下文。
    pub fn new(params: ProbeContextParams<'a>) -> Self {
        let ProbeContextParams {
            input,
            base_output,
            settings,
            preset,
            film_grain,
            samples,
            canceller,
            reporter,
        } = params;

        Self {
            input,
            base_output,
            settings,
            preset,
            film_grain,
            samples,
            canceller,
            reporter,
        }
    }
}

/// 组装 VMAF 测量命令：第一路是已编码样本，第二路是源片同一时间点。
///
/// 两路都不加 `-export_side_data +film_grain`：测量目标是颗粒合成后的最终观感，
/// 让两路按各自码流正常合成颗粒，口径一致。该选项只作用于紧随其后的那一路输入，
/// 单侧设置会让参考帧与失真帧的颗粒状态不一致。
///
/// 纯组装、不启动进程，便于用 `get_args()` 做静态断言。
fn build_vmaf_command(
    input: &Path,
    distorted: &Path,
    sample: &SceneSample,
    filter: &str,
) -> Command {
    FfmpegCommand::new()
        .no_stats()
        .no_audio_subtitle_data()
        .input(distorted)
        .seek(sample.start())
        .input(input)
        .duration(sample.duration())
        .lavfi(filter)
        .null_output()
        .build()
}

/// 采样测分器：把每个场景样本编码后用 libvmaf 测分，实现 SampleMeasurer。
pub struct SampleProbe<'a> {
    context: ProbeContext<'a>,
    filters: [String; 2],
    sample_filter: Option<String>,
    temp_stem: String,
    temp_extension: String,
}

impl<'a> SampleProbe<'a> {
    /// 由上下文构造测分器，预生成同规格 / 全规格两套滤镜图。
    pub fn new(context: ProbeContext<'a>) -> Self {
        let settings = context.settings;
        let temp_stem = context
            .base_output
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("distorted")
            .to_owned();
        let temp_extension = context
            .base_output
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("mp4")
            .to_owned();

        Self {
            filters: [
                vmaf::filter_graph(VmafMeasureMode::SameSpec, settings),
                vmaf::filter_graph(VmafMeasureMode::FullSpec, settings),
            ],
            sample_filter: settings.video_filter(),
            temp_stem,
            temp_extension,
            context,
        }
    }

    fn filter(&self, mode: VmafMeasureMode) -> &str {
        match mode {
            VmafMeasureMode::SameSpec => &self.filters[0],
            VmafMeasureMode::FullSpec => &self.filters[1],
        }
    }

    fn temp_output_path(&self, crf: ConstantRateFactor, index: usize) -> PathBuf {
        let name = format!(
            "{stem}_p{pid}_s{sample}c{crf:.2}.{extension}",
            stem = self.temp_stem,
            pid = std::process::id(),
            sample = index + 1,
            crf = crf.as_f64(),
            extension = self.temp_extension,
        );

        // 放系统临时目录：强退（process::exit）时 Drop 不执行，
        // 留在输出目录会污染用户产物；pid 避免多进程同名互踩。
        std::env::temp_dir().join(name)
    }

    /// 所有样本 VMAF 的中位数；单样本失败只跳过，不中断整轮搜索。
    fn median_vmaf(&self, crf: ConstantRateFactor, mode: VmafMeasureMode) -> Result<Option<f64>> {
        let total = self.context.samples.len();
        let mut scores = Vec::with_capacity(total);

        for (index, sample) in self.context.samples.iter().enumerate() {
            self.context.canceller.check()?;

            let position_seconds = sample.start().as_secs();
            let distorted = TempSampleFile::new(self.temp_output_path(crf, index));

            self.context.reporter.stage_detail(
                Stage::CrfSearch,
                format!(
                    "CRF {:.2} · 样本 {}/{total} · 编码",
                    crf.as_f64(),
                    index + 1
                ),
            );

            if let Err(error) = self.encode_sample(crf, sample, distorted.path()) {
                if Cancelled::is_in(&error) {
                    return Err(error);
                }

                self.context.reporter.warning(format!(
                    "样本编码失败（{position_seconds}s），已跳过：{error:#}"
                ));
                continue;
            }

            self.context.reporter.stage_detail(
                Stage::CrfSearch,
                format!(
                    "CRF {:.2} · 样本 {}/{total} · VMAF 测量",
                    crf.as_f64(),
                    index + 1
                ),
            );

            match self.measure_vmaf(sample, distorted.path(), mode) {
                Ok(Some(score)) => scores.push(score),
                Ok(None) => self
                    .context
                    .reporter
                    .warning(format!("样本 {position_seconds}s 未产生 VMAF 分数，已跳过")),
                Err(error) if Cancelled::is_in(&error) => return Err(error),
                Err(error) => self.context.reporter.warning(format!(
                    "样本 {position_seconds}s 的 VMAF 测量失败，已跳过：{error:#}"
                )),
            }
        }

        Ok(f64_median(&mut scores))
    }

    fn encode_sample(
        &self,
        crf: ConstantRateFactor,
        sample: &SceneSample,
        output: &Path,
    ) -> Result<()> {
        let mut command = Command::new("ffmpeg");
        command
            .args(["-hide_banner", "-v", "error", "-an", "-sn", "-dn"])
            .arg("-ss")
            .arg(sample.start().as_secs_f64().to_string())
            .arg("-i")
            .arg(self.context.input)
            .arg("-t")
            .arg(sample.duration().as_secs_f64().to_string());

        Av1EncodeArgs::new(self.context.preset, crf, self.context.film_grain).apply(&mut command);

        if let Some(filter) = &self.sample_filter {
            command.arg("-vf").arg(filter);
        }
        command.arg("-y").arg(output);

        let output = process::run(command, self.context.canceller, |_, _| {})?;

        if output.success() {
            Ok(())
        } else {
            bail!("样本编码失败：{}", output.failure_message())
        }
    }

    fn measure_vmaf(
        &self,
        sample: &SceneSample,
        distorted: &Path,
        mode: VmafMeasureMode,
    ) -> Result<Option<f64>> {
        let command = build_vmaf_command(self.context.input, distorted, sample, self.filter(mode));

        let output = process::run(command, self.context.canceller, |_, _| {})?;

        if !output.success() {
            bail!("VMAF 测量失败：{}", output.failure_message());
        }

        Ok(vmaf::parse_score(output.stderr_lines()))
    }
}

impl SampleMeasurer for SampleProbe<'_> {
    fn measure(&mut self, crf: ConstantRateFactor, mode: VmafMeasureMode) -> Result<Option<f64>> {
        match self.median_vmaf(crf, mode) {
            Ok(score) => Ok(score),
            Err(error) if Cancelled::is_in(&error) => Err(error),
            Err(error) => {
                self.context.reporter.warning(format!(
                    "CRF {:.2} 的{} VMAF 测量失败：{error:#}",
                    crf.as_f64(),
                    mode.label()
                ));
                Ok(None)
            }
        }
    }

    fn full_spec_is_distinct(&self) -> bool {
        self.context.settings.is_reduced()
    }
}

/// 搜索日志适配器：把轮次、备注与警告转发给 Reporter。
pub struct Journal<'a> {
    reporter: &'a Reporter,
}

impl<'a> Journal<'a> {
    /// 用 Reporter 构造日志适配器。
    pub fn new(reporter: &'a Reporter) -> Self {
        Self { reporter }
    }
}

impl SearchJournal for Journal<'_> {
    fn round(&mut self, round: u8, crf: ConstantRateFactor, score: f64) {
        self.reporter.stage_detail(
            Stage::CrfSearch,
            format!("第 {round} 轮 · CRF {:.2} · VMAF {score:.2}", crf.as_f64()),
        );
    }

    fn note(&mut self, message: &str) {
        self.reporter.note(message);
    }

    fn warning(&mut self, message: &str) {
        self.reporter.warning(message);
    }
}

/// 搜索过程中编码出的临时样本文件，Drop 时删除。
struct TempSampleFile {
    path: PathBuf,
}

impl TempSampleFile {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempSampleFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn vmaf_command_composes_grain_in_both_inputs() {
        let sample = SceneSample::new(Duration::from_secs(10), Duration::from_secs(5));
        let command = build_vmaf_command(
            Path::new("source.mkv"),
            Path::new("sample.mkv"),
            &sample,
            "[0:v]setpts=PTS-STARTPTS[dist];[1:v]setpts=PTS-STARTPTS[ref]",
        );
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        // 完整参数序列锁定：-ss 必须夹在两个 -i 之间，-t 在 lavfi 之前；
        // 两路都不导出 film_grain side data（衡量颗粒合成后的最终观感）。
        assert_eq!(
            args,
            [
                "-hide_banner",
                "-nostats",
                "-an",
                "-sn",
                "-dn",
                "-i",
                "sample.mkv",
                "-ss",
                "10",
                "-i",
                "source.mkv",
                "-t",
                "5",
                "-lavfi",
                "[0:v]setpts=PTS-STARTPTS[dist];[1:v]setpts=PTS-STARTPTS[ref]",
                "-f",
                "null",
                "-",
            ]
        );
    }
}
