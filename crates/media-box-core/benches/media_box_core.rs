//! media-box-core 的进程内基准：日志解析、进度节流、场景采样、统计、
//! 滤镜图拼接与 CRF 搜索状态机。
//!
//! 硬约束：**不启动 ffmpeg / ffprobe 或任何子进程**。所有被测代码只做纯计算
//! 或本地目录扫描；依赖外部二进制的阶段（探测、编码、VMAF 测量）不属于基准范围。
//!
//! 运行：`cargo bench -p media-box-core`（墙钟）；
//! `cargo bench -p media-box-core --features hotpath-alloc -- --test --exact <id>`
//! 配合 `HOTPATH_CRITERION_ITERATIONS` 输出固定次数的剖析报告。

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hotpath::criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use media_box_core::crf_search::{CrfSearcher, SampleMeasurer, SearchJournal};
use media_box_core::encoder::{BitrateReduceSettings, FrameRateReduce, ResolutionDownScaling};
use media_box_core::film_grain::representative_mean;
use media_box_core::media_scan::collect_videos;
use media_box_core::progress::{FfmpegProgressLineParser, FfmpegProgressSample};
use media_box_core::progress::{MIN_DELTA_PERCENT, ProgressEmitter};
use media_box_core::resolution::Resolution;
use media_box_core::scene_detect::{SceneCutDetectionRecord, parse_scdet_log_line};
use media_box_core::scene_sampler::SceneSampler;
use media_box_core::stats::f64_median;
use media_box_core::value::{ConstantRateFactor, FrameRatePerSecond, VmafScore};
use media_box_core::vmaf::{VmafMeasureMode, filter_graph, parse_score};
use media_box_core::ydif_probe::parse_ydif_value_from_line;
use media_box_core::ydif_stats::YdifValueSeries;
use media_box_core::ydif_stats::film_grain_strength;

/// 简单的线性同余伪随机数：基准数据要确定，不引入 rand 依赖。
fn lcg(seed: u64) -> impl FnMut() -> f64 {
    let mut state = seed;
    move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as f64 / (u64::MAX >> 33) as f64
    }
}

fn tuned(
    mut group: hotpath::criterion::BenchmarkGroup<'_>,
) -> hotpath::criterion::BenchmarkGroup<'_> {
    group
        .sample_size(50)
        .warm_up_time(Duration::from_millis(400))
        .measurement_time(Duration::from_millis(1_500));
    group
}

/// 仓库内的真实 ffmpeg -progress 输出样本（与 progress.rs 的解析契约对应）。
const PROGRESS_SAMPLE: &str =
    include_str!("../../../ffmpeg_examples/ffmpeg progress pipe example.txt");

fn bench_progress(c: &mut Criterion) {
    let lines: Vec<&str> = PROGRESS_SAMPLE.lines().collect();
    let total_bytes = PROGRESS_SAMPLE.len() as u64;

    let mut group = tuned(c.benchmark_group("progress"));
    group.throughput(Throughput::Bytes(total_bytes));

    group.bench_function("parse", |b| {
        b.iter(|| {
            let mut parser = FfmpegProgressLineParser::default();
            for line in &lines {
                black_box(parser.push_line(line));
            }
            black_box(parser.total_size_bytes())
        })
    });

    // 真实节奏：样本时间均匀推进，经过时间按 8 倍速折算。
    let total = Duration::from_secs(600);
    let samples: Vec<FfmpegProgressSample> = (1..=600)
        .map(|step| FfmpegProgressSample::new(step * 1_000_000))
        .collect();
    let elapsed_per_step = Duration::from_millis(125);

    group.bench_function("emit", |b| {
        b.iter(|| {
            let mut emitter = ProgressEmitter::new(None, total, MIN_DELTA_PERCENT);
            for sample in &samples {
                black_box(emitter.update(*sample, elapsed_per_step));
            }
        })
    });

    group.finish();
}

fn bench_log_parse(c: &mut Criterion) {
    // scdet 日志：真实行与噪声行各半。
    let mut scdet_lines = Vec::with_capacity(2_000);
    for index in 0..1_000u32 {
        scdet_lines.push(format!(
            "[scdet @ 000001] lavfi.scd.score: {}, lavfi.scd.time: {}",
            0.1 + f64::from(index % 80) / 100.0,
            f64::from(index) * 1.8
        ));
        scdet_lines.push(format!("frame= {} fps= 45 q=-1.0 size=256KiB", index * 3));
    }

    let mut ydif_lines = Vec::with_capacity(2_000);
    for index in 0..1_000u32 {
        ydif_lines.push(format!("frame:{index} pts:{index} pts_time:{}.5", index));
        ydif_lines.push(format!(
            "lavfi.signalstats.YDIF={}",
            2.0 + f64::from(index % 100) / 50.0
        ));
    }

    // libvmaf 的 -v info stderr：正文 + 结尾一行最终分数 + 少量收尾行。
    let mut vmaf_lines: Vec<String> = (0..2_000u32)
        .map(|index| format!("VMAF frame {index} score: 90.0{index:02}"))
        .collect();
    vmaf_lines.push("VMAF score: 92.3456".to_owned());
    vmaf_lines.push("Parsed_libvmaf_0: all frames processed".to_owned());
    vmaf_lines.push("frame= 1234 fps= 45".to_owned());

    // scdet / ydif 逐行扫 2000 行，按行数计吞吐量。
    let mut group = tuned(c.benchmark_group("log-parse"));
    group.throughput(Throughput::Elements(scdet_lines.len() as u64));
    group.bench_function("scdet", |b| {
        b.iter(|| {
            for line in &scdet_lines {
                black_box(parse_scdet_log_line(line));
            }
        })
    });
    group.bench_function("ydif", |b| {
        b.iter(|| {
            for line in &ydif_lines {
                black_box(parse_ydif_value_from_line(line));
            }
        })
    });
    group.finish();

    // parse_score 从尾部反向扫描，通常只读末尾几行：单独成组且不设吞吐量，
    // 避免套用 scdet 的 2000 elements 严重高估。
    let mut group = tuned(c.benchmark_group("vmaf-score"));
    group.bench_function("parse", |b| b.iter(|| parse_score(black_box(&vmaf_lines))));
    group.finish();
}

fn bench_sampler(c: &mut Criterion) {
    // 两小时的视频、每 3.6s 一个切点：切点间隔大于 1s 合并阈值，因此各自独立；
    // 场景 3.6s 长于 2s 下限，过滤后仍有大量有效场景，覆盖排序取 3 的高成本路径。
    let records: Vec<SceneCutDetectionRecord> = (0..2_000)
        .map(|index| {
            SceneCutDetectionRecord::new(
                0.1 + f64::from(index % 80) / 100.0,
                f64::from(index) * 3.6,
            )
        })
        .collect();
    let total = Duration::from_secs(7_200);
    let sampler = SceneSampler::default();

    let mut group = tuned(c.benchmark_group("sampler"));
    group.throughput(Throughput::Elements(records.len() as u64));
    group.bench_function("plan", |b| {
        b.iter(|| black_box(sampler.plan_samples(black_box(&records), total)))
    });
    group.finish();

    // 回退路径只看总时长、不遍历切点，单独成组且不设吞吐量。
    let mut group = tuned(c.benchmark_group("sampler/fallback"));
    group.bench_function("no-cuts", |b| {
        b.iter(|| black_box(sampler.fallback_samples(total)))
    });
    group.finish();
}

fn bench_stats(c: &mut Criterion) {
    let mut random = lcg(0x5EED);
    let noisy: Vec<f64> = (0..600).map(|_| 4.0 + 8.0 * random()).collect();
    let large: Vec<f64> = (0..10_001).map(|_| random() * 1000.0).collect();

    let mut group = tuned(c.benchmark_group("stats"));

    group.bench_function("median/3", |b| {
        b.iter_batched(
            || vec![3.0, 1.0, 2.0],
            |mut data: Vec<f64>| f64_median(&mut data),
            BatchSize::SmallInput,
        )
    });

    group.bench_function("median/101", |b| {
        let data: Vec<f64> = (0..101).map(|_| random()).collect();
        b.iter_batched(
            || data.clone(),
            |mut data: Vec<f64>| f64_median(&mut data),
            BatchSize::SmallInput,
        )
    });

    group.bench_function("median/10001", |b| {
        b.iter_batched(
            || large.clone(),
            |mut data: Vec<f64>| f64_median(&mut data),
            BatchSize::SmallInput,
        )
    });

    let series = YdifValueSeries::from_values(noisy.clone());
    group.bench_function("ydif/confidence", |b| {
        b.iter(|| black_box(series.confidence()))
    });

    let series_pair = [
        YdifValueSeries::from_values(vec![2.0; 600]),
        YdifValueSeries::from_values(vec![6.0; 600]),
        YdifValueSeries::from_values(noisy),
    ];
    group.bench_function("film-grain/representative", |b| {
        b.iter(|| black_box(representative_mean(&series_pair)))
    });

    group.bench_function("film-grain/strength", |b| {
        b.iter(|| black_box(film_grain_strength(7.99)))
    });

    group.finish();
}

fn reduced_settings() -> BitrateReduceSettings {
    BitrateReduceSettings::new(
        ResolutionDownScaling::new(Some(Resolution::Fhd), Resolution::Uhd),
        FrameRateReduce::new(
            Some(FrameRatePerSecond::new(30.0).unwrap()),
            FrameRatePerSecond::new(60.0).unwrap(),
        ),
    )
}

fn unchanged_settings() -> BitrateReduceSettings {
    BitrateReduceSettings::new(
        ResolutionDownScaling::new(None, Resolution::Fhd),
        FrameRateReduce::new(None, FrameRatePerSecond::new(24.0).unwrap()),
    )
}

fn bench_filter(c: &mut Criterion) {
    let reduced = reduced_settings();
    let unchanged = unchanged_settings();

    let mut group = tuned(c.benchmark_group("filter"));

    group.bench_function("video-filter/reduced", |b| {
        b.iter(|| black_box(reduced.video_filter()))
    });
    group.bench_function("video-filter/unchanged", |b| {
        b.iter(|| black_box(unchanged.video_filter()))
    });
    group.bench_function("is-reduced", |b| b.iter(|| black_box(reduced.is_reduced())));

    group.bench_function("vmaf-graph/same-spec", |b| {
        b.iter(|| black_box(filter_graph(VmafMeasureMode::SameSpec, reduced)))
    });
    group.bench_function("vmaf-graph/full-spec", |b| {
        b.iter(|| black_box(filter_graph(VmafMeasureMode::FullSpec, reduced)))
    });

    group.finish();
}

fn bench_value(c: &mut Criterion) {
    let mut group = tuned(c.benchmark_group("value"));

    group.bench_function("crf/new", |b| {
        b.iter(|| black_box(ConstantRateFactor::new(18.25)))
    });
    group.bench_function("crf/parse", |b| {
        b.iter(|| black_box("18.25".parse::<ConstantRateFactor>()))
    });
    group.bench_function("resolution/parse", |b| {
        b.iter(|| black_box("1920x1080".parse::<Resolution>()))
    });

    group.finish();
}

/// 基准用的临时目录树：根 + 4 层子目录，每个目录 4 个子目录、3 个混合扩展名文件，
/// 共 341 个目录、1023 个物理文件。放在 crate 的 `.tmp-tests/` 下（与测试约定一致），
/// Drop 时自动删除。
const SCAN_TREE_BRANCHING: usize = 4;

struct ScanTree {
    root: PathBuf,
    /// 全树：1 + 4 + 16 + 64 + 256 = 341 个目录 × 3 文件 = 1023。
    file_count: usize,
    /// `Some(1)` 只覆盖根与 4 个一级子目录：5 × 3 = 15 个物理文件。
    depth_one_file_count: usize,
}

impl ScanTree {
    fn build() -> Self {
        use std::fs;

        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("bench-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let extensions = ["mp4", "mkv", "txt"];
        let mut file_count = 0usize;

        fn walk(
            dir: &Path,
            depth: u8,
            extensions: &[&str; 3],
            file_count: &mut usize,
        ) -> Result<(), std::io::Error> {
            fs::create_dir_all(dir)?;
            for (index, extension) in extensions.iter().enumerate() {
                fs::write(
                    dir.join(format!("file-{index}.{extension}")),
                    b"benchmark payload",
                )?;
                *file_count += 1;
            }
            if depth > 0 {
                for sub in 0..SCAN_TREE_BRANCHING as u8 {
                    walk(
                        &dir.join(format!("sub-{sub}")),
                        depth - 1,
                        extensions,
                        file_count,
                    )?;
                }
            }
            Ok(())
        }

        walk(&root, 4, &extensions, &mut file_count).expect("构建扫描基准目录树失败");
        let depth_one_file_count = extensions.len() * (1 + SCAN_TREE_BRANCHING);

        Self {
            root,
            file_count,
            depth_one_file_count,
        }
    }
}

impl Drop for ScanTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn bench_scan(c: &mut Criterion) {
    let tree = ScanTree::build();

    // 吞吐量按每个基准实际扫描的文件数分别设置：Criterion 在注册时捕获 group 的吞吐量，
    // 因此这里在每次 bench_function 之前切换，两个基准不会互相套用。
    let mut group = tuned(c.benchmark_group("scan"));
    group.measurement_time(Duration::from_secs(3));

    group.throughput(Throughput::Elements(tree.file_count as u64));
    group.bench_function("collect/unlimited", |b| {
        b.iter(|| black_box(collect_videos(std::slice::from_ref(&tree.root), None)));
    });

    group.throughput(Throughput::Elements(tree.depth_one_file_count as u64));
    group.bench_function("collect/depth-1", |b| {
        b.iter(|| black_box(collect_videos(std::slice::from_ref(&tree.root), Some(1))));
    });

    group.finish();
}

/// 合成测量曲线：`S(c) = 100 - slope·(c-10)`，`F(c) = S(c) - gap`。
/// 每次测量是纯计算——真实的编码 + VMAF 成本由测量次数预算测试覆盖。
struct LinearCurve {
    slope: f64,
    gap: f64,
    distinct: bool,
}

impl SampleMeasurer for LinearCurve {
    fn measure(
        &mut self,
        crf: ConstantRateFactor,
        mode: VmafMeasureMode,
    ) -> anyhow::Result<Option<f64>> {
        let same_spec = 100.0 - self.slope * (crf.as_f64() - 10.0);
        Ok(Some(match mode {
            VmafMeasureMode::SameSpec => same_spec,
            VmafMeasureMode::FullSpec => same_spec - self.gap,
        }))
    }

    fn full_spec_is_distinct(&self) -> bool {
        self.distinct
    }
}

struct SilentJournal;

impl SearchJournal for SilentJournal {
    fn round(&mut self, _round: u8, _crf: ConstantRateFactor, _score: f64) {}
    fn note(&mut self, _message: &str) {}
    fn warning(&mut self, _message: &str) {}
}

fn run_search(target: f64, slope: f64, gap: f64, distinct: bool) -> f64 {
    CrfSearcher::new(VmafScore::new(target).expect("目标合法"), Resolution::Fhd)
        .search(
            &mut LinearCurve {
                slope,
                gap,
                distinct,
            },
            &mut SilentJournal,
        )
        .expect("搜索成功")
        .as_f64()
}

fn bench_crf_search(c: &mut Criterion) {
    let mut group = tuned(c.benchmark_group("crf-search"));

    group.bench_function("dual-spec/target-88", |b| {
        b.iter(|| black_box(run_search(88.0, 0.8, 1.0, true)))
    });
    group.bench_function("dual-spec/target-70", |b| {
        b.iter(|| black_box(run_search(70.0, 0.8, 1.0, true)))
    });
    group.bench_function("dual-spec/steep", |b| {
        b.iter(|| black_box(run_search(88.0, 2.0, 1.0, true)))
    });
    group.bench_function("single-spec/target-88", |b| {
        b.iter(|| black_box(run_search(88.0, 0.8, 1.0, false)))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_progress,
    bench_log_parse,
    bench_sampler,
    bench_stats,
    bench_filter,
    bench_value,
    bench_scan,
    bench_crf_search
);
criterion_main!(benches);
