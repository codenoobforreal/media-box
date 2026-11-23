use crate::codecs;
use crate::format::{IMAGE_EXTENSIONS, TargetFormat};
use crate::output_path::output_path;
use anyhow::{Context, Result};
use image::codecs::avif::AvifEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ColorType, DynamicImage, ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat};
use media_box_core::media_scan::{ScannedFile, collect_files};
use media_box_core::output_path::collision_free;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

/// AVIF 编码速度档：1 最慢、质量最好，10 最快。取 ravif/cavif 的默认档 4。
const AVIF_SPEED: u8 = 4;

/// 批量转换选项。
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct ConvertOptions {
    pub inputs: Vec<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub format: TargetFormat,
    pub quality: u8,
    pub depth: u8,
    pub jobs: usize,
    pub overwrite: bool,
    pub dry_run: bool,
    /// PNG 输出时启用最高压缩（无损）。
    pub optimize: bool,
}

/// 单个文件的转换结果。
#[derive(Debug, Clone, PartialEq)]
pub enum ConvertOutcome {
    /// 转换成功并已落盘。
    Success {
        source: PathBuf,
        target: PathBuf,
        bytes: u64,
    },
    /// 目标已存在且未开启覆盖，跳过。
    Skipped { source: PathBuf, target: PathBuf },
    /// --dry-run 下计划转换，未写盘。
    Planned { source: PathBuf, target: PathBuf },
    /// 转换失败，附带错误首条信息。
    Failed {
        source: PathBuf,
        target: PathBuf,
        error: String,
    },
}

impl ConvertOutcome {
    pub fn source(&self) -> &Path {
        match self {
            Self::Success { source, .. }
            | Self::Skipped { source, .. }
            | Self::Planned { source, .. }
            | Self::Failed { source, .. } => source,
        }
    }

    pub fn target(&self) -> &Path {
        match self {
            Self::Success { target, .. }
            | Self::Skipped { target, .. }
            | Self::Planned { target, .. }
            | Self::Failed { target, .. } => target,
        }
    }
}

/// 收集所有起始路径下的图片。
///
/// - 显式传入的文件一律接受（由调用方负责其可解码性）；
/// - 目录内容按图片扩展名筛选，depth 为递归层级，0 表示只看当前目录；
/// - 符号链接跳过，重复路径去重。
fn collect_images(inputs: &[impl AsRef<Path>], depth: u8) -> Result<Vec<ScannedFile>> {
    collect_files(inputs, IMAGE_EXTENSIONS, Some(depth))
}

/// 运行批量转换，返回逐文件结果（按源路径排序，保证输出稳定）。
pub fn run(options: ConvertOptions) -> Result<Vec<ConvertOutcome>> {
    if !options.format.can_write() {
        anyhow::bail!("目标格式 {} 不能作为输出", options.format.extension());
    }

    let images = collect_images(&options.inputs, options.depth)?;
    if images.is_empty() {
        return Ok(Vec::new());
    }

    let extension = options.format.extension();

    if options.dry_run {
        return Ok(plan_dry_run(&images, &options, extension));
    }

    Ok(run_tasks(
        &plan_tasks(&images, &options, extension),
        &options,
    ))
}

/// 为 --dry-run 生成计划结果，不写盘。
fn plan_dry_run(
    images: &[ScannedFile],
    options: &ConvertOptions,
    extension: &str,
) -> Vec<ConvertOutcome> {
    images
        .iter()
        .map(|image| match resolve_target(image, options, extension) {
            Ok(target) => ConvertOutcome::Planned {
                source: image.path.clone(),
                target,
            },
            Err(error) => ConvertOutcome::Failed {
                source: image.path.clone(),
                target: PathBuf::new(),
                error,
            },
        })
        .collect()
}

/// 为整批预留输出路径，保证同批同名文件互不覆盖。
fn plan_tasks(images: &[ScannedFile], options: &ConvertOptions, extension: &str) -> Vec<Task> {
    let mut taken = HashSet::new();
    let mut tasks = Vec::with_capacity(images.len());
    for image in images {
        match resolve_target(image, options, extension) {
            Ok(target) => tasks.push(Task::Convert {
                source: image.path.clone(),
                target: collision_free(&target, &mut taken),
            }),
            Err(error) => tasks.push(Task::Failed {
                source: image.path.clone(),
                error,
            }),
        }
    }
    tasks
}

/// 用工作池并发执行任务，收集并按源路径排序结果。
fn run_tasks(tasks: &[Task], options: &ConvertOptions) -> Vec<ConvertOutcome> {
    let (task_tx, task_rx) = mpsc::channel::<usize>();
    let (result_tx, result_rx) = mpsc::channel::<ConvertOutcome>();
    let task_rx = Arc::new(Mutex::new(task_rx));

    let workers = options.jobs.max(1);
    thread::scope(|scope| {
        for _ in 0..workers {
            let task_rx = Arc::clone(&task_rx);
            let result_tx = result_tx.clone();
            scope.spawn(move || {
                loop {
                    let index = {
                        let receiver = task_rx.lock().expect("任务队列锁中毒");
                        match receiver.recv() {
                            Ok(index) => index,
                            Err(mpsc::RecvError) => return,
                        }
                    };
                    let outcome = convert_task(index, tasks, options);
                    if result_tx.send(outcome).is_err() {
                        return;
                    }
                }
            });
        }

        for (index, _) in tasks.iter().enumerate() {
            let _ = task_tx.send(index);
        }
        drop(task_tx);
    });
    drop(result_tx);

    let mut outcomes: Vec<ConvertOutcome> = result_rx.into_iter().collect();
    outcomes.sort_by(|left, right| left.source().cmp(right.source()));
    outcomes
}

/// 一个待处理任务：路径已解析并预留，或已确定失败。
enum Task {
    Convert { source: PathBuf, target: PathBuf },
    Failed { source: PathBuf, error: String },
}

fn convert_task(index: usize, tasks: &[Task], options: &ConvertOptions) -> ConvertOutcome {
    match &tasks[index] {
        Task::Failed { source, error } => ConvertOutcome::Failed {
            source: source.clone(),
            target: PathBuf::new(),
            error: error.clone(),
        },
        Task::Convert { source, target } => {
            if !options.overwrite && target.exists() {
                return ConvertOutcome::Skipped {
                    source: source.clone(),
                    target: target.clone(),
                };
            }

            match convert_one(source, target, options) {
                Ok(bytes) => ConvertOutcome::Success {
                    source: source.clone(),
                    target: target.clone(),
                    bytes,
                },
                Err(error) => ConvertOutcome::Failed {
                    source: source.clone(),
                    target: target.clone(),
                    error: format!("{error:#}"),
                },
            }
        }
    }
}

fn resolve_target(
    source: &ScannedFile,
    options: &ConvertOptions,
    extension: &str,
) -> std::result::Result<PathBuf, String> {
    output_path(
        &source.path,
        options.output_dir.as_deref(),
        &source.root,
        extension,
    )
    .map_err(|error| error.to_string())
}

/// 解码并编码单个文件，原子写盘，返回输出字节数。
fn convert_one(source: &Path, target: &Path, options: &ConvertOptions) -> Result<u64> {
    let image = codecs::decode_input(source)?;

    let buffer = encode_to_buffer(&image, options.format, options.quality, options.optimize)
        .with_context(|| format!("编码图片 {}", source.display()))?;

    write_atomic(target, &buffer).with_context(|| format!("写入输出 {}", target.display()))
}

/// 把动态图像编码为内存字节。
///
/// JPEG/AVIF 使用可配置质量的编码器；PNG 在 optimize 时启用最高压缩；
/// JXL 走 zune-jpegxl；其余格式走通用写出。
fn encode_to_buffer(
    image: &DynamicImage,
    format: TargetFormat,
    quality: u8,
    optimize: bool,
) -> Result<Vec<u8>> {
    let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
    match format {
        TargetFormat::Standard(ImageFormat::Jpeg) => {
            JpegEncoder::new_with_quality(&mut cursor, quality).encode_image(image)?;
        }
        TargetFormat::Standard(ImageFormat::Avif) => {
            let (width, height) = image.dimensions();
            let rgba = image.to_rgba8();
            AvifEncoder::new_with_speed_quality(&mut cursor, AVIF_SPEED, quality).write_image(
                rgba.as_raw(),
                width,
                height,
                ExtendedColorType::Rgba8,
            )?;
        }
        TargetFormat::Standard(ImageFormat::Png) if optimize => {
            let (width, height) = image.dimensions();
            let (color, data) = png_pixel_buffer(image);
            PngEncoder::new_with_quality(
                &mut cursor,
                CompressionType::Best,
                FilterType::Adaptive,
            )
            .write_image(&data, width, height, color)?;
        }
        TargetFormat::Standard(format) => {
            image.write_to(&mut cursor, format)?;
        }
        TargetFormat::Jxl => {
            #[cfg(feature = "jxl")]
            {
                return encode_jxl(image, quality);
            }
            #[cfg(not(feature = "jxl"))]
            anyhow::bail!("目标格式 jxl 在当前编译特性下不可写");
        }
        TargetFormat::DecodeOnly(_) => {
            anyhow::bail!(
                "目标格式 {} 仅支持解码，不能作为输出",
                format.extension()
            );
        }
    }
    Ok(cursor.into_inner())
}

/// 用 zune-jpegxl 把图像无损编码为 JXL 字节（8bit 或 16bit 源均保持位深）。
#[cfg(feature = "jxl")]
fn encode_jxl(image: &DynamicImage, quality: u8) -> Result<Vec<u8>> {
    use zune_core::options::EncoderOptions;
    use zune_jpegxl::JxlSimpleEncoder;

    let (width, height) = image.dimensions();
    let (colorspace, depth, pixels) = jxl_pixel_layout(image);
    // 文件级已并发，编码器内固定单线程，避免线程过度订阅。
    let options = EncoderOptions::new(width as usize, height as usize, colorspace, depth)
        .set_quality(quality)
        .set_num_threads(1);

    let encoder = JxlSimpleEncoder::new(&pixels, options);
    let mut buffer = Vec::new();
    encoder
        .encode(&mut buffer)
        .map_err(|error| anyhow::anyhow!("JXL 编码失败：{error}"))?;
    Ok(buffer)
}

/// 整理 JXL 编码所需的（色彩空间、位深、原始像素字节）。
///
/// 8bit 源保持 8bit；16bit 源保持 16bit（按本机字节序展开为字节）；
/// 浮点源归一化到 8bit。
#[cfg(feature = "jxl")]
fn jxl_pixel_layout(image: &DynamicImage) -> (zune_core::colorspace::ColorSpace, zune_core::bit_depth::BitDepth, Vec<u8>) {
    use image::ColorType;
    use zune_core::bit_depth::BitDepth;
    use zune_core::colorspace::ColorSpace;

    match image.color() {
        ColorType::L8 => (ColorSpace::Luma, BitDepth::Eight, image.to_luma8().as_raw().to_vec()),
        ColorType::La8 => (ColorSpace::LumaA, BitDepth::Eight, image.to_luma_alpha8().as_raw().to_vec()),
        ColorType::Rgb8 => (ColorSpace::RGB, BitDepth::Eight, image.to_rgb8().as_raw().to_vec()),
        ColorType::Rgba8 => (ColorSpace::RGBA, BitDepth::Eight, image.to_rgba8().as_raw().to_vec()),
        ColorType::L16 => (ColorSpace::Luma, BitDepth::Sixteen, u16_native_bytes(image.to_luma16().as_raw())),
        ColorType::La16 => (ColorSpace::LumaA, BitDepth::Sixteen, u16_native_bytes(image.to_luma_alpha16().as_raw())),
        ColorType::Rgb16 => (ColorSpace::RGB, BitDepth::Sixteen, u16_native_bytes(image.to_rgb16().as_raw())),
        ColorType::Rgba16 => (ColorSpace::RGBA, BitDepth::Sixteen, u16_native_bytes(image.to_rgba16().as_raw())),
        other if other.has_alpha() => (ColorSpace::RGBA, BitDepth::Eight, image.to_rgba8().as_raw().to_vec()),
        _ => (ColorSpace::RGB, BitDepth::Eight, image.to_rgb8().as_raw().to_vec()),
    }
}

/// 取图像用于 PNG 编码的（颜色类型，原始像素字节），保留原始位深与通道。
fn png_pixel_buffer(image: &DynamicImage) -> (ExtendedColorType, Vec<u8>) {
    match image.color() {
        ColorType::L8 => (ExtendedColorType::L8, image.to_luma8().as_raw().to_vec()),
        ColorType::La8 => (ExtendedColorType::La8, image.to_luma_alpha8().as_raw().to_vec()),
        ColorType::Rgb8 => (ExtendedColorType::Rgb8, image.to_rgb8().as_raw().to_vec()),
        ColorType::Rgba8 => (ExtendedColorType::Rgba8, image.to_rgba8().as_raw().to_vec()),
        ColorType::L16 => (ExtendedColorType::L16, u16_native_bytes(image.to_luma16().as_raw())),
        ColorType::La16 => (ExtendedColorType::La16, u16_native_bytes(image.to_luma_alpha16().as_raw())),
        ColorType::Rgb16 => (ExtendedColorType::Rgb16, u16_native_bytes(image.to_rgb16().as_raw())),
        ColorType::Rgba16 => (ExtendedColorType::Rgba16, u16_native_bytes(image.to_rgba16().as_raw())),
        other if other.has_alpha() => (ExtendedColorType::Rgba8, image.to_rgba8().as_raw().to_vec()),
        _ => (ExtendedColorType::Rgb8, image.to_rgb8().as_raw().to_vec()),
    }
}

/// 把 16bit 像素（本机字节序的 u16 切片）展开为字节切片。
fn u16_native_bytes(values: &[u16]) -> Vec<u8> {
    values.iter().copied().flat_map(u16::to_ne_bytes).collect()
}

/// 先写同目录临时文件再重命名，避免失败留下半截输出。
fn write_atomic(target: &Path, bytes: &[u8]) -> Result<u64> {
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| format!("创建输出目录 {}", parent.display()))?;

    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    let temp = parent.join(format!(".{name}.{}.tmp", std::process::id()));

    if let Err(error) = fs::write(&temp, bytes) {
        let _ = fs::remove_file(&temp);
        return Err(error).with_context(|| format!("写入临时文件 {}", temp.display()));
    }

    match fs::rename(&temp, target) {
        Ok(()) => Ok(bytes.len() as u64),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error).with_context(|| format!("替换输出 {}", target.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageReader;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp-tests")
            .join(format!("image-convert-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_png(path: &Path, width: u32, height: u32) {
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            width,
            height,
            image::Rgb([10, 20, 30]),
        ));
        image.save_with_format(path, ImageFormat::Png).unwrap();
    }

    fn options(inputs: Vec<PathBuf>, output_dir: Option<PathBuf>) -> ConvertOptions {
        ConvertOptions {
            inputs,
            output_dir,
            format: "webp".parse().unwrap(),
            quality: 85,
            depth: 0,
            jobs: 1,
            overwrite: false,
            dry_run: false,
            optimize: false,
        }
    }

    #[test]
    fn collect_images_respects_depth() {
        let root = temp_dir("depth");
        let _ = fs::create_dir_all(root.join("sub/deep"));
        write_png(&root.join("a.png"), 1, 1);
        write_png(&root.join("sub/b.png"), 1, 1);
        write_png(&root.join("sub/deep/c.png"), 1, 1);

        let names = |depth: u8| {
            let mut paths: Vec<String> = collect_images(std::slice::from_ref(&root), depth)
                .unwrap()
                .into_iter()
                .map(|image| image.path.to_string_lossy().into_owned())
                .collect();
            paths.sort();
            paths
        };

        assert_eq!(names(0).len(), 1);
        assert_eq!(names(1).len(), 2);
        assert_eq!(names(2).len(), 3);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn explicit_files_are_accepted_and_dir_scan_is_filtered() {
        let root = temp_dir("filter");
        let _ = fs::create_dir_all(root.join("sub"));
        write_png(&root.join("sub/keep.png"), 1, 1);
        fs::write(root.join("sub/ignore.txt"), b"x").unwrap();
        write_png(&root.join("explicit.png"), 1, 1);

        let collected = collect_images(&[root.join("sub"), root.join("explicit.png")], 0).unwrap();
        let paths: Vec<String> = collected
            .iter()
            .map(|image| image.path.to_string_lossy().into_owned())
            .collect();
        assert!(paths.iter().any(|path| path.ends_with("keep.png")));
        assert!(!paths.iter().any(|path| path.ends_with("ignore.txt")));
        assert!(paths.iter().any(|path| path.ends_with("explicit.png")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_converts_png_to_webp_and_decodes_back() {
        let root = temp_dir("convert");
        let source = root.join("photo.png");
        write_png(&source, 4, 3);
        let out = root.join("out");

        let outcomes = run(options(vec![source.clone()], Some(out.clone()))).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert!(matches!(outcomes[0], ConvertOutcome::Success { .. }));

        let target = outcomes[0].target().to_path_buf();
        assert!(target.exists());
        let decoded = ImageReader::open(&target).unwrap().decode().unwrap();
        assert_eq!(decoded.dimensions(), (4, 3));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_skips_existing_unless_overwrite() {
        let root = temp_dir("skip");
        let source = root.join("photo.png");
        write_png(&source, 2, 2);
        let out = root.join("out");
        let _ = fs::create_dir_all(&out);
        write_png(&out.join("photo.webp"), 9, 9);

        let outcomes = run(options(vec![source.clone()], Some(out.clone()))).unwrap();
        assert!(matches!(outcomes[0], ConvertOutcome::Skipped { .. }));

        let mut overwrite = options(vec![source.clone()], Some(out.clone()));
        overwrite.overwrite = true;
        let outcomes = run(overwrite).unwrap();
        assert!(matches!(outcomes[0], ConvertOutcome::Success { .. }));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_dry_run_plans_without_writing() {
        let root = temp_dir("dry");
        let source = root.join("photo.png");
        write_png(&source, 2, 2);
        let out = root.join("out");

        let mut dry = options(vec![source.clone()], Some(out.clone()));
        dry.dry_run = true;
        let outcomes = run(dry).unwrap();
        assert!(matches!(outcomes[0], ConvertOutcome::Planned { .. }));
        assert!(!outcomes[0].target().exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_reports_decode_failure_without_stopping() {
        let root = temp_dir("fail");
        let bad = root.join("bad.png");
        fs::write(&bad, b"not an image").unwrap();
        let good = root.join("good.png");
        write_png(&good, 2, 2);
        let out = root.join("out");

        let outcomes = run(options(vec![bad, good], Some(out.clone()))).unwrap();
        assert_eq!(outcomes.len(), 2);
        assert!(
            outcomes
                .iter()
                .any(|outcome| matches!(outcome, ConvertOutcome::Failed { .. }))
        );
        assert!(
            outcomes
                .iter()
                .any(|outcome| matches!(outcome, ConvertOutcome::Success { .. }))
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_resolves_in_batch_collisions() {
        let root = temp_dir("collide");
        let a = root.join("a");
        let b = root.join("b");
        let _ = fs::create_dir_all(&a);
        let _ = fs::create_dir_all(&b);
        write_png(&a.join("photo.png"), 2, 2);
        write_png(&b.join("photo.png"), 2, 2);
        let out = root.join("out");

        let outcomes = run(options(
            vec![a.join("photo.png"), b.join("photo.png")],
            Some(out.clone()),
        ))
        .unwrap();
        assert_eq!(outcomes.len(), 2);
        let targets: Vec<String> = outcomes
            .iter()
            .map(|outcome| outcome.target().to_string_lossy().into_owned())
            .collect();
        assert!(targets.contains(&out.join("photo.webp").to_string_lossy().into_owned()));
        assert!(targets.contains(&out.join("photo-1.webp").to_string_lossy().into_owned()));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn optimize_produces_valid_png() {
        let root = temp_dir("optimize");
        let source = root.join("photo.png");
        write_png(&source, 4, 3);

        let mut opts = options(vec![source], Some(root.join("out")));
        opts.format = "png".parse().unwrap();
        opts.optimize = true;
        let outcomes = run(opts).unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(
            matches!(outcomes[0], ConvertOutcome::Success { .. }),
            "PNG 优化编码应成功：{:?}",
            outcomes[0]
        );
        let decoded = ImageReader::open(outcomes[0].target())
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(decoded.dimensions(), (4, 3));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(feature = "jxl")]
    fn encodes_jxl_and_decodes_back() {
        codecs::register_input_decoders();

        let root = temp_dir("jxl-encode");
        let source = root.join("photo.png");
        write_png(&source, 4, 3);

        let mut opts = options(vec![source], Some(root.join("out")));
        opts.format = "jxl".parse().unwrap();
        let outcomes = run(opts).unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(
            matches!(outcomes[0], ConvertOutcome::Success { .. }),
            "JXL 编码应成功：{:?}",
            outcomes[0]
        );
        let decoded = ImageReader::open(outcomes[0].target())
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(decoded.dimensions(), (4, 3));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[cfg(feature = "jxl")]
    fn decodes_jxl_sample_generated_by_zune() {
        // 让测试进程注册扩展解码 hook。
        codecs::register_input_decoders();

        // 用 zune-jpegxl 生成无损 JXL 样本（jxl 特性依赖）。
        let (width, height) = (4u32, 3u32);
        let mut pixels = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[(x * 60) as u8, (y * 80) as u8, 128]);
            }
        }
        let encoder = zune_jpegxl::JxlSimpleEncoder::new(
            &pixels,
            zune_core::options::EncoderOptions::new(
                width as usize,
                height as usize,
                zune_core::colorspace::ColorSpace::RGB,
                zune_core::bit_depth::BitDepth::Eight,
            ),
        );
        let mut encoded = Vec::new();
        encoder.encode(&mut encoded).expect("JXL 样本编码失败");

        let root = temp_dir("jxl");
        let source = root.join("sample.jxl");
        fs::write(&source, encoded).unwrap();

        let mut opts = options(vec![source], Some(root.join("out")));
        opts.format = "png".parse().unwrap();
        let outcomes = run(opts).unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(
            matches!(outcomes[0], ConvertOutcome::Success { .. }),
            "JXL 解码应成功：{:?}",
            outcomes[0]
        );
        let decoded = ImageReader::open(outcomes[0].target())
            .unwrap()
            .decode()
            .unwrap();
        assert_eq!(decoded.dimensions(), (width, height));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[ignore = "user-run: 需要真实 HEIC 样本（如 iPhone 照片）。把样本放到 crates/image-convert/tests/samples/photo.heic 后运行 cargo test -p image-convert -- --ignored"]
    fn decodes_real_heic_sample() {
        let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/samples/photo.heic");
        assert!(
            sample.exists(),
            "缺少样本文件 tests/samples/photo.heic，请先放入一张 HEIC 照片"
        );

        let root = temp_dir("heic");
        let mut opts = options(vec![sample], Some(root.join("out")));
        opts.format = "png".parse().unwrap();
        let outcomes = run(opts).unwrap();

        assert_eq!(outcomes.len(), 1);
        assert!(
            matches!(outcomes[0], ConvertOutcome::Success { .. }),
            "HEIC 解码应成功：{:?}",
            outcomes[0]
        );

        let _ = fs::remove_dir_all(&root);
    }
}
