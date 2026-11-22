use crate::byte_size::ByteSize;
use crate::cli::CliArgs;
use crate::crf_probe::{Journal, ProbeContext, SampleProbe};
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
use crate::scene_detect::SceneCutDetector;
use crate::scene_sampler::{SceneSample, SceneSampler};
use crate::ui::Ui;
use anyhow::{Result, bail};
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Compressor {
    args: CliArgs,
}

impl Compressor {
    pub fn new(args: CliArgs) -> Self {
        Self { args }
    }

    pub fn run(&self, canceller: &Canceller) -> Result<()> {
        let mut queue = collect_videos(&self.args.inputs, Some(self.args.depth))?;

        if queue.is_empty() {
            bail!("在输入路径中未找到视频文件：\n{:?}", self.args.inputs);
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
            ResolutionDownScaling::new(self.args.resolution, metadata.resolution),
            FrameRateReduce::new(self.args.fps, metadata.fps),
        );

        let samples = self.scene_samples(video_path, metadata.duration, canceller, reporter)?;

        reporter.stage(Stage::FilmGrain);
        let film_grain =
            FilmGrainCalculator::new(video_path, &samples, reporter).calculate(canceller)?;

        let output_base =
            output_path::output_base_path(video_path, self.args.output_dir.as_deref())?;

        reporter.stage(Stage::CrfSearch);
        let mut probe = SampleProbe::new(ProbeContext {
            input: video_path,
            base_output: &output_base,
            settings,
            preset: self.args.preset,
            film_grain,
            samples: &samples,
            canceller,
            reporter,
        });
        let mut journal = Journal::new(reporter);
        let crf = CrfSearcher::new(self.args.vmaf_score, settings.output_resolution())
            .search(&mut probe, &mut journal)?;

        reporter.stage_detail(Stage::Encode, format!("CRF {:.2}", crf.as_f64()));

        let output_path =
            output_path::final_output_path(&output_base, output_path::now_unix_seconds())?;
        let encode_args = Av1EncodeArgs::new(self.args.preset, crf, film_grain);
        let command = encode_job::build_command(video_path, &output_path, encode_args, settings);
        let command_string = process::display_command(&command);

        let output_size = encode_job::run(
            command,
            &output_path,
            canceller,
            reporter,
            started_at,
            metadata.duration,
        )?;

        Ok(output_size.map(|bytes| {
            SuccessReport::new(
                metadata.size,
                ByteSize::b(bytes),
                metadata.duration,
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
