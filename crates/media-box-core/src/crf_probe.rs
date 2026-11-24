//! CRF 搜索的测量实现：用 ffmpeg + libvmaf 在场景样本上测分，并适配搜索日志。

use crate::crf_search::{SampleMeasurer, SearchJournal};
use crate::encoder::{Av1EncodeArgs, BitrateReduceSettings};
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
        let mut command = Command::new("ffmpeg");
        command
            .args([
                "-hide_banner",
                "-nostats",
                "-v",
                "info",
                "-export_side_data",
                "+film_grain",
                "-i",
            ])
            .arg(distorted)
            .arg("-ss")
            .arg(sample.start().as_secs_f64().to_string())
            .arg("-i")
            .arg(self.context.input)
            .arg("-t")
            .arg(sample.duration().as_secs_f64().to_string())
            .arg("-lavfi")
            .arg(self.filter(mode))
            .args(["-f", "null", "-"]);

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

    fn note(&mut self, message: String) {
        self.reporter.note(message);
    }

    fn warning(&mut self, message: String) {
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
