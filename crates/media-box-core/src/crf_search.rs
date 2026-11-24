//! CRF 搜索状态机：在单调曲线上用「二分 + 割线」混合逼近目标 VMAF。算法推导见根目录 AGENTS.md。

use crate::resolution::Resolution;
use crate::value::{ConstantRateFactor, VmafScore};
use crate::vmaf::VmafMeasureMode;
use anyhow::Result;

/// 搜索最多迭代的轮次。
///
/// 这是一道兜底：区间每轮至少收缩一格（0.25），且通常直接减半，
/// 从 30 宽收到 0.5 只需要 6 轮左右。正常曲线走不到这里，只有非单调/阶跃的
/// 病态曲线才可能用满。
const MAX_SEARCH_ROUNDS: u8 = 12;

/// 只向上放宽、不向下放宽——「画质略好于目标」可接受，「略差于目标」不可接受。
const TARGET_SCORE_TOLERANCE: f64 = 1.0;

/// 全规格核对允许的双向偏差。
const FINAL_CHECK_TOLERANCE: f64 = 0.5;

/// 区间宽度收缩到这个量级时停止：网格步长是 0.25，再分下去已无意义。
const MIN_SEARCH_SPAN: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrfRange {
    low: ConstantRateFactor,
    high: ConstantRateFactor,
}

impl CrfRange {
    pub fn new(low: ConstantRateFactor, high: ConstantRateFactor) -> Self {
        Self { low, high }
    }

    pub fn low(self) -> ConstantRateFactor {
        self.low
    }

    pub fn high(self) -> ConstantRateFactor {
        self.high
    }
}

/// 按输出分辨率给出建议区间。
pub fn suggest_crf_range(resolution: Resolution) -> CrfRange {
    let (low, high) = match resolution.pixels() {
        pixels if pixels >= Resolution::Uhd.pixels() => (24.0, 55.0),
        pixels if pixels >= Resolution::Qhd.pixels() => (22.0, 52.0),
        pixels if pixels >= Resolution::Fhd.pixels() => (20.0, 50.0),
        pixels if pixels >= Resolution::Hd.pixels() => (18.0, 45.0),
        _ => (16.0, 40.0),
    };

    CrfRange::new(
        ConstantRateFactor::snapped_clamped(low),
        ConstantRateFactor::snapped_clamped(high),
    )
}

pub trait SampleMeasurer {
    /// `Ok(None)` 表示本次测量失败但可以继续（例如样本编码失败），
    /// `Err` 表示必须中止（例如用户取消）。
    fn measure(&mut self, crf: ConstantRateFactor, mode: VmafMeasureMode) -> Result<Option<f64>>;
    /// 全规格与同规格是否是两次**不同**的测量。
    ///
    /// 没有降规格（分辨率、帧率都不动）时两者滤镜图相同，实现方返回 `false`
    /// 可让搜索省掉端点全规格、标定补测与收尾复核，测量次数约减 40%。
    ///
    /// 默认 `true`：保守假设两种口径不同。
    fn full_spec_is_distinct(&self) -> bool {
        true
    }
}

pub trait SearchJournal {
    fn round(&mut self, round: u8, crf: ConstantRateFactor, score: f64);
    fn note(&mut self, message: String);
    fn warning(&mut self, message: String);
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Calibration {
    /// (同规格分, 全规格分)
    points: Vec<(f64, f64)>,
}

impl Calibration {
    fn push(&mut self, same_spec: f64, full_spec: f64) {
        if self.points.len() < 3 {
            self.points.push((same_spec, full_spec));
        }
    }

    fn linear_target(&self, target: f64) -> Option<f64> {
        let (s1, f1) = *self.points.first()?;
        let (s2, f2) = *self.points.get(1)?;

        let s_diff = s2 - s1;
        if s_diff.abs() < 0.01 {
            return None; // 两点同分：竖直线，反解无意义
        }

        let slope = (f2 - f1) / s_diff;
        if slope.abs() < 1e-9 {
            return None; // 水平线：反推会得到 ±inf
        }

        let intercept = f1 - slope * s1;
        let s_target = (target - intercept) / slope;

        Some(s_target.clamp(s1.min(s2), s1.max(s2)))
    }

    fn quadratic_target(&self, target: f64) -> Option<f64> {
        if self.points.len() < 3 {
            return None;
        }

        // 拟合前按 S 排序：调用方压点的顺序是「端点、端点、中点」，不能直接用。
        let mut points: Vec<(f64, f64)> = self.points.clone();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        let [(x1, y1), (x2, y2), (x3, y3)] = [points[0], points[1], points[2]];

        // 范德蒙德行列式；三点共线时退化。
        let denominator = (x1 - x2) * (x1 - x3) * (x2 - x3);
        if denominator.abs() < 1e-6 {
            return None;
        }

        let a = (x3 * (y2 - y1) + x2 * (y1 - y3) + x1 * (y3 - y2)) / denominator;
        // 曲率过大 = 拟合不可信；二次项为 0 = 退化成直线，交给线性映射。
        if a.abs() > 0.05 || a.abs() < 1e-9 {
            return None;
        }

        let b = (x3 * x3 * (y1 - y2) + x2 * x2 * (y3 - y1) + x1 * x1 * (y2 - y3)) / denominator;
        let c = (x2 * x3 * (x2 - x3) * y1 + x3 * x1 * (x3 - x1) * y2 + x1 * x2 * (x1 - x2) * y3)
            / denominator;

        // 单调性校验：顶点必须在区间之外。
        let vertex = -b / (2.0 * a);
        if (a > 0.0 && vertex > x1) || (a < 0.0 && vertex < x3) {
            return None;
        }

        // 解 a·S² + b·S + (c - T) = 0，取区间内的根。
        let (qa, qb, qc) = (a, b, c - target);
        let discriminant = qb * qb - 4.0 * qa * qc;
        if discriminant < 0.0 {
            return None;
        }

        let sqrt_discriminant = discriminant.sqrt();
        let root = [
            (-qb - sqrt_discriminant) / (2.0 * qa),
            (-qb + sqrt_discriminant) / (2.0 * qa),
        ]
        .into_iter()
        .find(|candidate| (x1 - 0.01..=x3 + 0.01).contains(candidate))?;

        Some(root.clamp(x1, x3))
    }
}

/// CRF 搜索状态机。
///
/// 不变式：`S(low) ≥ S*`、`S(high) < S*`（`S*` 是标定后的同规格目标）。
#[derive(Debug, Clone, PartialEq)]
pub struct CrfSearcher {
    target_score: VmafScore,
    initial: CrfRange,
    low: ConstantRateFactor,
    high: ConstantRateFactor,
    /// 已测过的 (CRF, 同规格分)，按时间顺序；尾部两点用来估斜率。
    measurements: Vec<(ConstantRateFactor, f64)>,
    calibration: Calibration,
}

impl CrfSearcher {
    pub fn new(target_score: VmafScore, resolution: Resolution) -> Self {
        let range = suggest_crf_range(resolution);

        Self {
            target_score,
            initial: range,
            low: range.low(),
            high: range.high(),
            measurements: Vec::new(),
            calibration: Calibration::default(),
        }
    }

    pub fn bounds(&self) -> CrfRange {
        CrfRange::new(self.low, self.high)
    }

    pub fn search(
        mut self,
        measurer: &mut impl SampleMeasurer,
        journal: &mut impl SearchJournal,
    ) -> Result<ConstantRateFactor> {
        let target = self.target_score.as_f64();
        let mut round = 0u8;
        let dual_spec = measurer.full_spec_is_distinct();

        let Some(low_score) = measurer.measure(self.low, VmafMeasureMode::SameSpec)? else {
            journal.warning("下界 VMAF 无法测得，退回最优画质 CRF".to_owned());
            return Ok(self.low);
        };

        round += 1;
        journal.round(round, self.low, low_score);
        self.record(self.low, low_score);

        if low_score < target {
            journal.note("区间下界已无法达标，采用最优画质 CRF".to_owned());
            return Ok(self.low);
        }

        if dual_spec
            && let Some(full_score) = measurer.measure(self.low, VmafMeasureMode::FullSpec)?
        {
            self.calibration.push(low_score, full_score);

            if full_score < target {
                journal.note("区间下界全规格仍未达标，采用最优画质 CRF".to_owned());
                return Ok(self.low);
            }
        }

        let Some(high_score) = measurer.measure(self.high, VmafMeasureMode::SameSpec)? else {
            journal.warning("上界 VMAF 无法测得，退回最优画质 CRF".to_owned());
            return Ok(self.low);
        };

        round += 1;
        journal.round(round, self.high, high_score);
        self.record(self.high, high_score);

        // 只有单口径时同规格分才等于全规格分。
        // 双口径下同规格达标不代表全规格达标，必须等下面的全规格测量再判定，
        // 否则会返回未达标的最高压缩率 CRF。
        if !dual_spec && high_score >= target {
            journal.note("区间上界已满足目标 VMAF，采用最高压缩率 CRF".to_owned());
            return Ok(self.high);
        }

        if dual_spec
            && let Some(full_score) = measurer.measure(self.high, VmafMeasureMode::FullSpec)?
        {
            self.calibration.push(high_score, full_score);

            if full_score >= target {
                journal.note("区间上界全规格已满足目标 VMAF，采用最高压缩率 CRF".to_owned());
                return Ok(self.high);
            }
        }

        let mut same_spec_target = if dual_spec {
            self.calibration.linear_target(target).unwrap_or(target)
        } else {
            // S ≡ F：迭代目标就是用户目标本身，无需标定。
            target
        };
        let mut quadratic_ready = false;
        let mut hit_crf = None;

        loop {
            let next_crf = self.next_crf(same_spec_target);

            let Some(score) = measurer.measure(next_crf, VmafMeasureMode::SameSpec)? else {
                journal.warning("搜索过程中 VMAF 无法测得，退回当前最优 CRF".to_owned());
                return Ok(self.low);
            };

            round += 1;
            journal.round(round, next_crf, score);

            let target_window = same_spec_target..=same_spec_target + TARGET_SCORE_TOLERANCE;
            if target_window.contains(&score) {
                journal.note("命中目标质量区间，搜索结束".to_owned());
                // 记下命中点：让收尾修正用最新斜率，而不是几轮前的。
                self.record(next_crf, score);
                hit_crf = Some(next_crf);
                break;
            }

            self.update_bounds(next_crf, score, same_spec_target);

            // 补齐第 3 个标定点，凑满后尝试一次三点二次映射。
            // 用 `< 3` 之后：正常路径（两端点都成功）仍然只补一次，成本不变；
            // 退化路径会在后续轮次自动补齐（最坏多花 2 次全规格测量）。
            if dual_spec
                && !quadratic_ready
                && self.calibration.points.len() < 3
                && let Some(full_score) = measurer.measure(next_crf, VmafMeasureMode::FullSpec)?
            {
                self.calibration.push(score, full_score);

                if let Some(adjusted) = self.calibration.quadratic_target(target) {
                    same_spec_target = adjusted;
                    quadratic_ready = true;
                    self.rebracket(adjusted);
                }
            }

            if self.high.as_f64() - self.low.as_f64() <= MIN_SEARCH_SPAN {
                journal.note("搜索区间已收缩到 0.5 以内，搜索结束".to_owned());
                break;
            }

            if round >= MAX_SEARCH_ROUNDS {
                journal.note(format!("达到最大搜索轮次 {MAX_SEARCH_ROUNDS}，搜索结束"));
                break;
            }
        }

        if dual_spec {
            self.finalize(measurer, journal, hit_crf)
        } else {
            // 已测到的同规格分就是全规格分，不必再测一遍
            Ok(hit_crf.unwrap_or(self.low))
        }
    }

    /// 收尾：用全规格分数核对结果，必要时修正一次。
    fn finalize(
        &mut self,
        measurer: &mut impl SampleMeasurer,
        journal: &mut impl SearchJournal,
        hit_crf: Option<ConstantRateFactor>,
    ) -> Result<ConstantRateFactor> {
        let target = self.target_score.as_f64();
        let final_crf = hit_crf.unwrap_or(self.low);

        let Some(final_score) = measurer.measure(final_crf, VmafMeasureMode::FullSpec)? else {
            journal.warning("最终全规格校验未完成，沿用搜索结果".to_owned());
            return Ok(final_crf);
        };

        let delta = final_score - target;

        if delta.abs() <= FINAL_CHECK_TOLERANCE {
            journal.note(format!("最终全规格校验通过（VMAF {final_score:.2}）"));
            return Ok(final_crf);
        }

        // 只处理「偏低」：偏高说明画质好于目标，本来就安全，不动。
        if delta < -FINAL_CHECK_TOLERANCE
            && let Some(adjusted) = self.adjust_by_delta(final_crf, delta)
        {
            journal.note(format!(
                "全规格校验偏低，CRF 微调至 {:.2}",
                adjusted.as_f64()
            ));
            return Ok(adjusted);
        }

        Ok(final_crf)
    }

    fn next_crf(&self, same_spec_target: f64) -> ConstantRateFactor {
        let bisect_mid = (self.low.as_f64() + self.high.as_f64()) / 2.0;

        let Some((interpolated, previous_crf)) = self.interpolation(same_spec_target) else {
            return ConstantRateFactor::snapped_clamped(bisect_mid);
        };

        let max_step = ((self.high.as_f64() - self.low.as_f64()) * 0.5).clamp(0.5, 8.0);
        let mixed = 0.7 * bisect_mid + 0.3 * interpolated;
        let next = mixed.clamp(self.low.as_f64(), self.high.as_f64()).clamp(
            previous_crf.as_f64() - max_step,
            previous_crf.as_f64() + max_step,
        );
        let snapped = ConstantRateFactor::snapped_clamped(next);

        if snapped == previous_crf || {
            snapped == self.low || snapped == self.high || snapped < self.low || snapped > self.high
        } {
            return self.fallback_candidate(previous_crf);
        }

        snapped
    }

    /// 兜底候选：在区间内挑一个未测过的点，保证搜索有进展。
    ///
    /// 二分中点可能恰好是上一次测过的 CRF，此时区间不会收缩，
    /// 循环会原地打转到轮次上限，每一轮都是一次真实的编码 + 测量。
    fn fallback_candidate(&self, previous_crf: ConstantRateFactor) -> ConstantRateFactor {
        let mid =
            ConstantRateFactor::snapped_clamped((self.low.as_f64() + self.high.as_f64()) / 2.0);
        let low = self.low;
        let high = self.high;

        for candidate in [mid, low, high] {
            if candidate != previous_crf && candidate >= low && candidate <= high {
                return candidate;
            }
        }

        // 区间已窄到没有其它网格点：交由收敛判据退出。
        mid
    }

    fn interpolation(&self, same_spec_target: f64) -> Option<(f64, ConstantRateFactor)> {
        let (previous_crf, previous_score) = *self.measurements.last()?;
        let (earlier_crf, earlier_score) = *self
            .measurements
            .get(self.measurements.len().checked_sub(2)?)?;

        let slope = trusted_slope(
            previous_crf.as_f64() - earlier_crf.as_f64(),
            previous_score - earlier_score,
        )?;

        // 瞄准 S* 而不是 T：循环的收敛量是标定后的目标。用 T 会系统性多压 δ 分
        // （δ = S* - T），再由窗口兜回来，白跑一轮。
        let gap = same_spec_target - previous_score;
        Some((previous_crf.as_f64() + gap / slope, previous_crf))
    }

    fn adjust_by_delta(
        &self,
        current: ConstantRateFactor,
        delta: f64,
    ) -> Option<ConstantRateFactor> {
        let (previous_crf, previous_score) = *self.measurements.last()?;
        let (earlier_crf, earlier_score) = *self
            .measurements
            .get(self.measurements.len().checked_sub(2)?)?;

        let vmaf_delta = previous_score - earlier_score;
        if vmaf_delta.abs() < 0.01 {
            return None; // 斜率估不出来
        }

        let crf_per_vmaf = (previous_crf.as_f64() - earlier_crf.as_f64()) / vmaf_delta;
        // 注意这里是减号：delta 为负（画质不足）时要把 CRF 往小调。
        let adjusted = current.as_f64() - delta * crf_per_vmaf * 0.8;

        Some(ConstantRateFactor::snapped_clamped(
            adjusted.clamp(self.low.as_f64() - 1.0, self.high.as_f64()),
        ))
    }

    fn update_bounds(&mut self, crf: ConstantRateFactor, score: f64, same_spec_target: f64) {
        if score >= same_spec_target {
            self.low = crf;
        } else {
            self.high = crf;
        }

        self.record(crf, score);
    }

    /// 换一个 `S*` 后重算区间：用已测历史重新划界，而不是沿用按旧目标划的界。
    fn rebracket(&mut self, same_spec_target: f64) {
        let mut low = self.initial.low;
        let mut high = self.initial.high;

        for &(crf, score) in &self.measurements {
            if score >= same_spec_target {
                low = low.max(crf);
            } else {
                high = high.min(crf);
            }
        }

        // 历史测量自相矛盾（噪声所致）时保持原界，交给收敛判据退出。
        if low <= high {
            self.low = low;
            self.high = high;
        }
    }

    fn record(&mut self, crf: ConstantRateFactor, score: f64) {
        if let Some(slot) = self
            .measurements
            .iter_mut()
            .find(|(known, _)| *known == crf)
        {
            // 同一 CRF 再次被测量：保留最新分数，旧的可能是过时的。
            slot.1 = score;
            return;
        }

        self.measurements.push((crf, score));
    }
}

fn trusted_slope(crf_delta: f64, vmaf_delta: f64) -> Option<f64> {
    if crf_delta.abs() < 0.01 || vmaf_delta.abs() < 0.01 {
        return None;
    }

    let slope = vmaf_delta / crf_delta;
    (slope.is_finite() && (-10.0..-0.1).contains(&slope)).then_some(slope)
}
