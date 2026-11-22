use crate::byte_size::ByteSize;
use crate::crf_probe::{Journal, ProbeContext, ProbeContextParams, SampleProbe};
use crate::crf_search::CrfSearcher;
use crate::encode_job;
use crate::encoder::{
    Av1EncodeArgs, BitrateReduceSettings, FrameRateReduce, ResolutionDownScaling,
};
use crate::film_grain::FilmGrainCalculator;
use crate::interrupter::{Cancelled, Canceller};
use crate::media_scan::collect_videos;
use crate::output_path;
use crate::probe;
use crate::process;
use crate::report::{Reporter, Stage, SuccessReport};
use crate::resolution::Resolution;
use crate::scene_detect::SceneCutDetector;
use crate::scene_sampler::{SceneSample, SceneSampler};
use crate::ui::Ui;
use crate::value::{EncoderPreset, FrameRatePerSecond, VmafScore};
use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 一次压缩任务的配置。与 CLI 解耦：库不依赖 clap，由应用层把解析结果映射进来。
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct CompressOptions {
    pub inputs: Vec<PathBuf>,
    pub resolution: Option<Resolution>,
    pub output_dir: Option<PathBuf>,
    pub depth: u8,
    pub fps: Option<FrameRatePerSecond>,
    pub preset: EncoderPreset,
    pub vmaf_score: VmafScore,
}

impl CompressOptions {
    /// 构造压缩配置；各参数与 CLI 选项一一对应（映射关系见二进制 crate 的 `cli` 模块）。
    pub fn new(
        inputs: Vec<PathBuf>,
        resolution: Option<Resolution>,
        output_dir: Option<PathBuf>,
        depth: u8,
        fps: Option<FrameRatePerSecond>,
        preset: EncoderPreset,
        vmaf_score: VmafScore,
    ) -> Self {
        Self {
            inputs,
            resolution,
            output_dir,
            depth,
            fps,
            preset,
            vmaf_score,
        }
    }
}

pub struct Compressor {
    options: CompressOptions,
}

impl Compressor {
    pub fn new(options: CompressOptions) -> Self {
        Self { options }
    }

    pub fn run(&self, canceller: &Canceller) -> Result<()> {
        let mut queue = collect_videos(&self.options.inputs, Some(self.options.depth))?;

        if queue.is_empty() {
            bail!("在输入路径中未找到视频文件：\n{:?}", self.options.inputs);
        }

        let ui = Ui::install(&queue);
        let reporter = ui.reporter();

        while let Some(video_path) = queue.pop() {
            if canceller.is_cancelled() {
                reporter.cancelled();
                break;
            }

            reporter.job_started(&video_path);

            match self.compress_one(&video_path, canceller, reporter) {
                Ok(report) => reporter.job_completed(video_path, report),
                Err(error) if Cancelled::is_in(&error) => {
                    reporter.cancelled();
                    break;
                }
                Err(error) => reporter.job_failed(video_path, format!("{error:#}")),
            }
        }

        ui.shutdown()
    }

    fn compress_one(
        &self,
        video_path: &Path,
        canceller: &Canceller,
        reporter: &Reporter,
    ) -> Result<Option<SuccessReport>> {
        let started_at = Instant::now();

        reporter.stage(Stage::Metadata);
        let metadata = probe::read_metadata(video_path, canceller)?;

        let settings = BitrateReduceSettings::new(
            ResolutionDownScaling::new(self.options.resolution, metadata.resolution()),
            FrameRateReduce::new(self.options.fps, metadata.fps()),
        );

        let samples = self.scene_samples(video_path, metadata.duration(), canceller, reporter)?;

        reporter.stage(Stage::FilmGrain);
        let film_grain =
            FilmGrainCalculator::new(video_path, &samples, reporter).calculate(canceller)?;

        let output_base =
            output_path::output_base_path(video_path, self.options.output_dir.as_deref())?;

        reporter.stage(Stage::CrfSearch);
        let mut probe = SampleProbe::new(ProbeContext::new(ProbeContextParams {
            input: video_path,
            base_output: &output_base,
            settings,
            preset: self.options.preset,
            film_grain,
            samples: &samples,
            canceller,
            reporter,
        }));
        let mut journal = Journal::new(reporter);
        let crf = CrfSearcher::new(self.options.vmaf_score, settings.output_resolution())
            .search(&mut probe, &mut journal)?;

        reporter.stage_detail(Stage::Encode, format!("CRF {:.2}", crf.as_f64()));

        let output_path =
            output_path::final_output_path(&output_base, output_path::now_unix_seconds())?;
        let encode_args = Av1EncodeArgs::new(self.options.preset, crf, film_grain);
        let command = encode_job::build_command(video_path, &output_path, encode_args, settings);
        let command_string = process::display_command(&command);

        let output_size = encode_job::run(
            command,
            &output_path,
            canceller,
            reporter,
            started_at,
            metadata.duration(),
        )?;

        Ok(output_size.map(|bytes| {
            SuccessReport::new(
                metadata.size(),
                ByteSize::b(bytes),
                metadata.duration(),
                started_at.elapsed(),
                command_string,
            )
        }))
    }

    fn scene_samples(
        &self,
        video_path: &Path,
        duration: Duration,
        canceller: &Canceller,
        reporter: &Reporter,
    ) -> Result<Vec<SceneSample>> {
        reporter.stage(Stage::SceneSampling);
        let sampler = SceneSampler::default();
        sampler.validate()?;

        if !sampler.needs_scene_detection(duration) {
            return Ok(sampler.fallback_samples(duration));
        }

        let cuts = match SceneCutDetector::new(video_path).detect_scene_cuts(canceller) {
            Ok(cuts) => cuts,
            Err(error) if Cancelled::is_in(&error) => return Err(error),
            Err(_) => return Ok(sampler.fallback_samples(duration)),
        };

        Ok(sampler.plan_samples(&cuts, duration))
    }
}
