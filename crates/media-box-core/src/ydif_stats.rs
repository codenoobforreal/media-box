//! YDIF 序列统计、置信度判定与 film-grain 强度映射。

#[derive(Debug, Clone, Copy, PartialEq)]
struct Stats {
    mean: f64,
    standard_deviation: f64,
}

/// 一条样本的 YDIF 值序列，提供统计、置信度判定与代表值映射。
#[derive(Debug, PartialEq, Clone)]
pub struct YdifValueSeries(Vec<f64>);

impl YdifValueSeries {
    /// 由探测到的 YDIF 值构造序列。
    pub fn from_values(values: Vec<f64>) -> Self {
        Self(values)
    }

    fn stats(&self) -> Stats {
        #[expect(
            clippy::as_conversions,
            clippy::cast_precision_loss,
            reason = "样本数远小于 2^53，转 f64 只为求均值"
        )]
        let count = self.0.len() as f64;
        let mean = self.0.iter().sum::<f64>() / count;
        let variance = self
            .0
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f64>()
            / count;

        Stats {
            mean,
            standard_deviation: variance.max(0.0).sqrt(),
        }
    }

    fn coefficient_of_variation(stats: Stats) -> f64 {
        stats.standard_deviation / stats.mean
    }

    fn acceptable_coefficient_of_variation(stats: Stats) -> f64 {
        ((stats.mean / 15.0).min(1.0) * 0.2 + 0.5).clamp(0.5, 0.7)
    }

    /// 依据变异系数判定该序列的置信度。
    pub fn confidence(&self) -> Confidence {
        let stats = self.stats();

        if Self::coefficient_of_variation(stats) <= Self::acceptable_coefficient_of_variation(stats)
        {
            Confidence::High
        } else {
            Confidence::Low
        }
    }

    /// 统计量是否可算（空序列或出现非有限值时为 false）。
    pub fn has_computable_statistics(&self) -> bool {
        if self.0.is_empty() {
            return false;
        }

        let stats = self.stats();
        Self::coefficient_of_variation(stats).is_finite()
            && Self::acceptable_coefficient_of_variation(stats).is_finite()
    }

    /// YDIF 均值。
    pub fn mean(&self) -> f64 {
        self.stats().mean
    }
}

/// YDIF 序列的置信度档位。
#[expect(
    clippy::exhaustive_enums,
    reason = "只有高/低两档，新增档位需同步修改代表值选择策略"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// CV ≤ 阈值：YDIF 波动稳定，可信。
    High,
    /// CV > 阈值：波动大，置信度不足。
    Low,
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
    crate::stats::f64_median(&mut means)
}

/// 把 YDIF 代表值映射为 film-grain 强度档位（0 / 4 / 8 / 12 / 15）。
pub fn film_grain_strength(representative_ydif: f64) -> u8 {
    match representative_ydif {
        value if value < 1.0 => 0,
        value if value < 3.0 => 4,
        value if value < 8.0 => 8,
        value if value < 15.0 => 12,
        _ => 15,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_stable_and_noisy_series() {
        assert_eq!(
            YdifValueSeries::from_values(vec![4.0; 8]).confidence(),
            Confidence::High
        );
        assert_eq!(
            YdifValueSeries::from_values(vec![1.0, 40.0, 1.0, 40.0]).confidence(),
            Confidence::Low
        );

        // 数值很大时方差不能失真（曾经用 Σx² 公式会得到 NaN）。
        let large = YdifValueSeries::from_values(vec![1e9, 1e9 + 1.0, 1e9 + 2.0]);
        assert!(large.has_computable_statistics());
    }

    #[test]
    fn maps_ydif_to_film_grain_strength() {
        assert_eq!(film_grain_strength(0.5), 0);
        assert_eq!(film_grain_strength(2.0), 4);
        assert_eq!(film_grain_strength(7.99), 8);
        assert_eq!(film_grain_strength(14.99), 12);
        assert_eq!(film_grain_strength(50.0), 15);
    }

    fn constant(value: f64) -> YdifValueSeries {
        // 恒定序列 CV=0，置信度必为 High。
        YdifValueSeries::from_values(vec![value; 8])
    }

    fn noisy(pairs: usize) -> YdifValueSeries {
        // 1/40 交替：CV 远超阈值，置信度必为 Low（见上方的置信度测试）。
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
