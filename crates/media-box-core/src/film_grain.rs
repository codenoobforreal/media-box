use crate::interrupter::{Cancelled, Canceller};
use crate::report::Reporter;
use crate::scene_sampler::SceneSample;
use crate::stats::f64_median;
use crate::value::FilmGrain;
use crate::ydif_probe::YdifProbe;
use crate::ydif_stats::{Confidence, YdifValueSeries, film_grain_strength};
use anyhow::Result;
use std::path::Path;

pub struct FilmGrainCalculator<'a> {
    input_video_path: &'a Path,
    scene_samples: &'a [SceneSample],
    reporter: &'a Reporter,
}

impl<'a> FilmGrainCalculator<'a> {
    pub fn new(
        input_video_path: &'a Path,
        scene_samples: &'a [SceneSample],
        reporter: &'a Reporter,
    ) -> Self {
        Self {
            input_video_path,
            scene_samples,
            reporter,
        }
    }

    pub fn calculate(&self, canceller: &Canceller) -> Result<FilmGrain> {
        let mut series = Vec::with_capacity(self.scene_samples.len());

        for sample in self.scene_samples {
            match YdifProbe::new(
                self.input_video_path.to_path_buf(),
                sample.start(),
                sample.duration(),
            )
            .run(canceller)
            {
                Ok(values) if values.has_computable_statistics() => series.push(values),
                Ok(_) => self
                    .reporter
                    .warning("样本 YDIF 统计不可算，已跳过".to_owned()),
                Err(error) if Cancelled::is_in(&error) => return Err(error),
                Err(error) => self
                    .reporter
                    .warning(format!("样本 YDIF 探测失败，已跳过：{error:#}")),
            }
        }

        if series.is_empty() {
            self.reporter
                .warning("颗粒分析无有效样本，film-grain 退回默认值 8".to_owned());
            return Ok(FilmGrain::default());
        }

        let high_confidence: Vec<&YdifValueSeries> = series
            .iter()
            .filter(|values| values.confidence() == Confidence::High)
            .collect();
        let minimum_required = series.len().div_ceil(2).max(1);

        let chosen: Vec<&YdifValueSeries> = if high_confidence.len() >= minimum_required {
            high_confidence
        } else {
            series.iter().collect()
        };

        let mut means: Vec<f64> = chosen.iter().map(|values| values.mean()).collect();

        let Some(median_of_means) = f64_median(&mut means) else {
            return Ok(FilmGrain::default());
        };

        Ok(FilmGrain::new(film_grain_strength(median_of_means))?)
    }
}
