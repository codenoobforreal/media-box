#[derive(Debug, Clone, Copy, PartialEq)]
struct Stats {
    mean: f64,
    standard_deviation: f64,
}

#[derive(Debug, PartialEq, Clone)]
pub struct YdifValueSeries(Vec<f64>);

impl YdifValueSeries {
    pub fn from_values(values: Vec<f64>) -> Self {
        Self(values)
    }

    fn stats(&self) -> Stats {
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

    pub fn confidence(&self) -> Confidence {
        let stats = self.stats();

        if Self::coefficient_of_variation(stats) <= Self::acceptable_coefficient_of_variation(stats)
        {
            Confidence::High
        } else {
            Confidence::Low
        }
    }

    pub fn has_computable_statistics(&self) -> bool {
        if self.0.is_empty() {
            return false;
        }

        let stats = self.stats();
        Self::coefficient_of_variation(stats).is_finite()
            && Self::acceptable_coefficient_of_variation(stats).is_finite()
    }

    pub fn mean(&self) -> f64 {
        self.stats().mean
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// CV ≤ 阈值：YDIF 波动稳定，可信。
    High,
    /// CV > 阈值：波动大，置信度不足。
    Low,
}

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
}
