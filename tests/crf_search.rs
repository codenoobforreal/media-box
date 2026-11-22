//! CRF 搜索的行为契约：给定一条测量曲线，返回的 CRF 应让**全规格** VMAF 落在目标附近。
//!
//! 刻意不约束轮次、候选顺序、标定公式——那些是实现细节，调整算法不应该改测试。

use anyhow::Result;
use media_box::crf_search::{CrfSearcher, SampleMeasurer, SearchJournal};
use media_box::interrupter::Cancelled;
use media_box::resolution::Resolution;
use media_box::value::{ConstantRateFactor, VmafScore};
use media_box::vmaf::VmafMeasureMode;

/// 合成曲线：`S(c) = 100 - slope·(c-10)`，`F(c) = S(c) - gap(c)`。
struct Curve {
    slope: f64,
    gap: fn(f64) -> f64,
    /// 第 N 次测量开始返回失败（模拟样本反复编码失败）。
    fail_from: Option<usize>,
    calls: Vec<ConstantRateFactor>,
}

impl Curve {
    fn new(slope: f64, gap: fn(f64) -> f64) -> Self {
        Self {
            slope,
            gap,
            fail_from: None,
            calls: Vec::new(),
        }
    }

    fn same_spec(&self, crf: ConstantRateFactor) -> f64 {
        100.0 - self.slope * (crf.as_f64() - 10.0)
    }

    fn full_spec(&self, crf: ConstantRateFactor) -> f64 {
        self.same_spec(crf) - (self.gap)(crf.as_f64())
    }
}

impl SampleMeasurer for Curve {
    fn measure(&mut self, crf: ConstantRateFactor, mode: VmafMeasureMode) -> Result<Option<f64>> {
        self.calls.push(crf);

        if self
            .fail_from
            .is_some_and(|limit| self.calls.len() >= limit)
        {
            return Ok(None);
        }

        Ok(Some(match mode {
            VmafMeasureMode::SameSpec => self.same_spec(crf),
            VmafMeasureMode::FullSpec => self.full_spec(crf),
        }))
    }
}

struct CancellingCurve;

impl SampleMeasurer for CancellingCurve {
    fn measure(&mut self, _crf: ConstantRateFactor, _mode: VmafMeasureMode) -> Result<Option<f64>> {
        Err(anyhow::Error::new(Cancelled))
    }
}

#[derive(Default)]
struct SilentJournal;

impl SearchJournal for SilentJournal {
    fn round(&mut self, _round: u8, _crf: ConstantRateFactor, _score: f64) {}
    fn note(&mut self, _message: String) {}
    fn warning(&mut self, _message: String) {}
}

fn searcher(target: f64) -> CrfSearcher {
    CrfSearcher::new(
        VmafScore::new(target).expect("目标必须在 1..=100"),
        Resolution::Fhd,
    )
}

/// 用户视角的达标判据：不能欠（< T - 0.5），也不能白送体积（> T + 2）。
fn assert_meets_target(curve: &Curve, crf: ConstantRateFactor, target: f64) {
    let score = curve.full_spec(crf);

    assert!(
        score >= target - 0.5,
        "质量不达标：target={target} crf={crf:?} vmaf={score:.2}"
    );
    assert!(
        score <= target + 2.0,
        "过度牺牲体积：target={target} crf={crf:?} vmaf={score:.2}"
    );
}

#[test]
fn lands_on_target_for_reachable_targets() {
    for target in [70.0, 80.0, 88.0, 91.0] {
        let searcher = searcher(target);
        let mut curve = Curve::new(0.8, |_| 1.0);

        let result = searcher
            .search(&mut curve, &mut SilentJournal)
            .expect("搜索不应失败");

        assert_meets_target(&curve, result, target);
    }
}

#[test]
fn lands_on_target_when_the_spec_gap_curves_with_crf() {
    // 同规格与全规格的差值随 CRF 非线性变化：标定必有残差，收尾修正要能把它收回来。
    fn gap(crf: f64) -> f64 {
        0.5 + 2.5 * ((crf - 20.0).max(0.0) / 10.0).sqrt()
    }

    let searcher = searcher(88.0);
    let mut curve = Curve::new(0.8, gap);

    let result = searcher
        .search(&mut curve, &mut SilentJournal)
        .expect("搜索不应失败");
    let score = curve.full_spec(result);

    assert!(score >= 87.5, "crf={result:?} vmaf={score:.2}");
}

#[test]
fn returns_best_quality_when_the_target_is_out_of_reach() {
    // 连下界（20）都只有 92 分，目标 99 不可达。
    let searcher = searcher(99.0);
    let range = searcher.bounds();
    let mut curve = Curve::new(0.8, |_| 1.0);

    let result = searcher
        .search(&mut curve, &mut SilentJournal)
        .expect("不可达也应正常返回");

    assert_eq!(result, range.low, "应退到最优画质");
    assert!(curve.full_spec(result) < 99.0);
}

#[test]
fn returns_max_compression_when_the_target_is_already_met() {
    // 上界（50）就有 68 分，目标 60 只要求更低的画质。
    let searcher = searcher(60.0);
    let range = searcher.bounds();
    let mut curve = Curve::new(0.8, |_| 1.0);

    let result = searcher
        .search(&mut curve, &mut SilentJournal)
        .expect("搜索不应失败");

    assert_eq!(result, range.high, "应退到最高压缩率");
}

#[test]
fn upper_bound_must_meet_full_spec_not_just_same_spec() {
    // 上界（50）同规格 68 ≥ 目标 65，但全规格只有 63：
    // 双口径下不能因为同规格达标就直接返回最高压缩率 CRF。
    // 修复前此用例返回 50（全规格 63 < 64.5），修复后应搜索到全规格达标的点。
    let searcher = searcher(65.0);
    let mut curve = Curve::new(0.8, |_| 5.0);

    let result = searcher
        .search(&mut curve, &mut SilentJournal)
        .expect("搜索不应失败");

    assert_meets_target(&curve, result, 65.0);
}

#[test]
fn stays_inside_the_suggested_range_and_grid() {
    let searcher = searcher(88.0);
    let range = searcher.bounds();
    let mut curve = Curve::new(0.8, |_| 1.0);

    searcher
        .search(&mut curve, &mut SilentJournal)
        .expect("搜索不应失败");

    for crf in &curve.calls {
        assert!(
            (range.low..=range.high).contains(crf),
            "测量越界：{crf:?} 不在 [{:?}, {:?}]",
            range.low,
            range.high
        );
        assert!(
            (crf.as_f64() * 4.0).fract().abs() < 1e-9,
            "不在 0.25 网格：{crf:?}"
        );
    }
}

#[test]
fn keeps_the_measurement_budget_bounded() {
    let searcher = searcher(88.0);
    let mut curve = Curve::new(0.8, |_| 1.0);

    searcher
        .search(&mut curve, &mut SilentJournal)
        .expect("搜索不应失败");

    // 每次测量 = 一次样本编码 + 一次 VMAF，成本是用户能感知的；只给宽松上界，不锁轨迹。
    assert!(
        curve.calls.len() <= 16,
        "测量 {} 次，超出预算",
        curve.calls.len()
    );
}

#[test]
fn survives_measurement_failures() {
    let searcher = searcher(88.0);
    let range = searcher.bounds();
    let mut curve = Curve::new(0.8, |_| 1.0);
    curve.fail_from = Some(3);

    let result = searcher
        .search(&mut curve, &mut SilentJournal)
        .expect("测量失败不应变成硬错误");

    assert!((range.low..=range.high).contains(&result));
}

#[test]
fn propagates_cancellation() {
    let searcher = searcher(88.0);

    let error = searcher
        .search(&mut CancellingCurve, &mut SilentJournal)
        .expect_err("取消必须向上传播");

    assert!(Cancelled::is_in(&error));
}

/// 无降规格：全规格与同规格是同一次测量，搜索不应请求全规格口径。
struct SingleSpecCurve {
    slope: f64,
    calls: Vec<ConstantRateFactor>,
}

impl SampleMeasurer for SingleSpecCurve {
    fn measure(&mut self, crf: ConstantRateFactor, mode: VmafMeasureMode) -> Result<Option<f64>> {
        assert_eq!(
            mode,
            VmafMeasureMode::SameSpec,
            "单口径模式不应请求全规格测量"
        );
        self.calls.push(crf);

        Ok(Some(100.0 - self.slope * (crf.as_f64() - 10.0)))
    }

    fn full_spec_is_distinct(&self) -> bool {
        false
    }
}

#[test]
fn skips_dual_spec_work_when_nothing_is_reduced() {
    let searcher = searcher(88.0);
    let mut curve = SingleSpecCurve {
        slope: 0.8,
        calls: Vec::new(),
    };

    let result = searcher
        .search(&mut curve, &mut SilentJournal)
        .expect("搜索不应失败");

    // 命中窗口 [88, 89]：S(24) = 88.8。
    let score = 100.0 - 0.8 * (result.as_f64() - 10.0);
    assert!(
        (88.0..=89.0).contains(&score),
        "crf={result:?} vmaf={score}"
    );

    // 双口径下同样的曲线要测 10 次（含 4 次冗余的全规格），单口径只测 6 次。
    assert!(
        curve.calls.len() <= 7,
        "测量 {} 次，未跳过冗余的全规格测量",
        curve.calls.len()
    );
}
