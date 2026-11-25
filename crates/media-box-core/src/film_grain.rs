//! film-grain 强度计算：汇总各样本的 YDIF 代表值。

use crate::interrupter::{Cancelled, Canceller};
use crate::report::Reporter;
use crate::scene_sampler::SceneSample;
use crate::value::FilmGrain;
use crate::ydif_probe::YdifProbe;
use crate::ydif_stats::{film_grain_strength, representative_mean};
use anyhow::Result;
use std::path::Path;

/// film-grain 强度计算器：逐样本探测 YDIF 并汇总为代表值。
#[derive(Debug)]
pub struct FilmGrainCalculator<'a> {
    input_video_path: &'a Path,
    scene_samples: &'a [SceneSample],
    reporter: &'a Reporter,
}

impl<'a> FilmGrainCalculator<'a> {
    /// 用源视频、场景样本与事件上报方构造计算器。
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

    /// 逐样本探测并汇总 film-grain 强度；全部样本失效时退回默认值。
    ///
    /// # Errors
    ///
    /// 用户取消时返回带 [`Cancelled`] 的错误；其余样本级失败只记警告并跳过。
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

        let Some(median_of_means) = representative_mean(&series) else {
            self.reporter
                .warning("颗粒分析无有效样本，film-grain 退回默认值 8".to_owned());
            return Ok(FilmGrain::default());
        };

        Ok(FilmGrain::new(film_grain_strength(median_of_means))?)
    }
}
