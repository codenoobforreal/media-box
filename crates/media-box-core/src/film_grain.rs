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

        let Some(median_of_means) = representative_mean(&series) else {
            self.reporter
                .warning("颗粒分析无有效样本，film-grain 退回默认值 8".to_owned());
            return Ok(FilmGrain::default());
        };

        Ok(FilmGrain::new(film_grain_strength(median_of_means))?)
    }
}

/// 从各样本的 YDIF 序列中选出代表均值：高置信样本不少于一半时只用高置信组，
/// 否则全部纳入（低置信样本聊胜于无）；返回所选均值的中位数，空输入返回 `None`。
///
/// 纯函数：只依赖传入序列，不触碰 ffmpeg，选片策略可直接测试。
pub fn representative_mean(series: &[YdifValueSeries]) -> Option<f64> {
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
    f64_median(&mut means)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn constant(value: f64) -> YdifValueSeries {
        // 恒定序列 CV=0，置信度必为 High。
        YdifValueSeries::from_values(vec![value; 8])
    }

    fn noisy(pairs: usize) -> YdifValueSeries {
        // 1/40 交替：CV 远超阈值，置信度必为 Low（见 ydif_stats 测试）。
        let mut values = Vec::with_capacity(pairs * 2);
        for _ in 0..pairs {
            values.push(1.0);
            values.push(40.0);
        }
        YdifValueSeries::from_values(values)
    }

    #[test]
    fn empty_series_has_no_representative() {
        assert_eq!(representative_mean(&[]), None);
    }

    #[test]
    fn uses_only_high_confidence_when_it_reaches_half() {
        // 2 高 1 低：高置信恰过半 → 低置信均值 20.5 不得进入中位数。
        let series = [constant(2.0), constant(6.0), noisy(2)];
        assert_eq!(representative_mean(&series), Some(4.0));
    }

    #[test]
    fn half_is_enough_even_with_a_single_high_series() {
        // 1 高 1 低：过半阈值 div_ceil(2)=1，只用高置信。
        let series = [constant(2.0), noisy(2)];
        assert_eq!(representative_mean(&series), Some(2.0));
    }

    #[test]
    fn falls_back_to_all_series_when_high_confidence_is_minority() {
        // 1 高 2 低：高置信不足一半 → 全部纳入，均值 [5.0, 20.5, 20.5] 中位数 20.5；
        // 若误只用高置信会得到 5.0。
        let series = [constant(5.0), noisy(2), noisy(1)];
        assert_eq!(representative_mean(&series), Some(20.5));
    }
}
