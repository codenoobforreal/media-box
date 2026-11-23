use crate::format::{IMAGE_EXTENSIONS, TargetFormat};
use crate::output_path::{collision_free, output_path};
use anyhow::{Context, Result};
use image::codecs::avif::AvifEncoder;
use image::codecs::jpeg::JpegEncoder;
use image::{
    DynamicImage, ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat, ImageReader,
};
use std::collections::{HashMap, HashSet};
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

/// 待转换的源图片及其所属输入根目录（用于镜像输出结构）。
#[derive(Debug, Clone, PartialEq)]
struct SourceImage {
    path: PathBuf,
    root: PathBuf,
}

/// 收集所有起始路径下的图片。
///
/// - 显式传入的文件一律接受（由调用方负责其可解码性）；
/// - 目录内容按图片扩展名筛选，depth 为递归层级，0 表示只看当前目录；
/// - 符号链接跳过，重复路径去重。
fn collect_images(inputs: &[impl AsRef<Path>], depth: u8) -> Result<Vec<SourceImage>> {
    let mut roots = Vec::new();
    let mut files = Vec::new();

    for input in inputs {
        let absolute = std::path::absolute(input.as_ref())
            .with_context(|| format!("解析输入路径 {}", input.as_ref().display()))?;
        let metadata = fs::symlink_metadata(&absolute)
            .with_context(|| format!("读取输入路径 {}", absolute.display()))?;

        if metadata.is_symlink() {
            continue;
        }

        if metadata.is_dir() {
            roots.push(absolute);
        } else if metadata.is_file() {
            let root = absolute
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            files.push(SourceImage {
                path: absolute,
                root,
            });
        }
    }

    let mut collector = ImageCollector::default();
    for root in &roots {
        collector.collect_dir(root, depth)?;
    }
    for file in files {
        collector.accept_file(file);
    }

    Ok(collector.images)
}

/// 运行批量转换，返回逐文件结果（按源路径排序，保证输出稳定）。
pub fn run(options: ConvertOptions) -> Result<Vec<ConvertOutcome>> {
    let images = collect_images(&options.inputs, options.depth)?;
    if images.is_empty() {
        return Ok(Vec::new());
    }

    let extension = options.format.extension();

    if options.dry_run {
        return Ok(images
            .iter()
            .map(|image| match resolve_target(image, &options, extension) {
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
            .collect());
    }

    // 先在主线程为整批预留输出路径，保证同批同名文件互不覆盖。
    let mut taken = HashSet::new();
    let mut tasks = Vec::with_capacity(images.len());
    for image in &images {
        match resolve_target(image, &options, extension) {
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

    let (task_tx, task_rx) = mpsc::channel::<usize>();
    let (result_tx, result_rx) = mpsc::channel::<ConvertOutcome>();
    let task_rx = Arc::new(Mutex::new(task_rx));

    let workers = options.jobs.max(1);
    let tasks_ref: &[Task] = &tasks;
    let options_ref = &options;
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
                    let outcome = convert_task(index, tasks_ref, options_ref);
                    if result_tx.send(outcome).is_err() {
                        return;
                    }
                }
            });
        }

        for (index, _) in tasks_ref.iter().enumerate() {
            let _ = task_tx.send(index);
        }
        drop(task_tx);
    });
    drop(result_tx);

    let mut outcomes: Vec<ConvertOutcome> = result_rx.into_iter().collect();
    outcomes.sort_by(|left, right| left.source().cmp(right.source()));
    Ok(outcomes)
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
    source: &SourceImage,
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
    let image = ImageReader::open(source)
        .with_context(|| format!("打开源图片 {}", source.display()))?
        .decode()
        .with_context(|| format!("解码源图片 {}", source.display()))?;

    let buffer = encode_to_buffer(&image, options.format.as_image(), options.quality)
        .with_context(|| format!("编码图片 {}", source.display()))?;

    write_atomic(target, &buffer).with_context(|| format!("写入输出 {}", target.display()))
}

/// 把动态图像编码为内存字节。JPEG 与 AVIF 使用可配置质量的编码器，其余走通用写出。
fn encode_to_buffer(image: &DynamicImage, format: ImageFormat, quality: u8) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    match format {
        ImageFormat::Jpeg => {
            JpegEncoder::new_with_quality(&mut buffer, quality).encode_image(image)?;
        }
        ImageFormat::Avif => {
            let (width, height) = image.dimensions();
            let rgba = image.to_rgba8();
            AvifEncoder::new_with_speed_quality(&mut buffer, AVIF_SPEED, quality).write_image(
                rgba.as_raw(),
                width,
                height,
                ExtendedColorType::Rgba8,
            )?;
        }
        _ => {
            let mut cursor = std::io::Cursor::new(Vec::<u8>::new());
            image.write_to(&mut cursor, format)?;
            return Ok(cursor.into_inner());
        }
    }
    Ok(buffer)
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

fn is_image_by_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

#[derive(Default)]
struct ImageCollector {
    images: Vec<SourceImage>,
    seen: HashSet<PathBuf>,
    /// 目录 -> 已用过的剩余深度，取最大值以合并多次访问。
    visited_dirs: HashMap<PathBuf, u8>,
}

impl ImageCollector {
    fn collect_dir(&mut self, root: &Path, depth: u8) -> Result<()> {
        let mut stack = vec![(root.to_path_buf(), depth)];

        while let Some((dir, remaining)) = stack.pop() {
            if !self.claim_dir(&dir, remaining) {
                continue;
            }

            let entries =
                fs::read_dir(&dir).with_context(|| format!("读取目录 {}", dir.display()))?;
            for entry in entries {
                let entry = entry.with_context(|| format!("读取目录项 {}", dir.display()))?;
                let file_type = entry
                    .file_type()
                    .with_context(|| format!("读取目录项类型 {}", entry.path().display()))?;
                let path = entry.path();

                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    if remaining > 0 {
                        stack.push((path, remaining - 1));
                    }
                } else if file_type.is_file() && is_image_by_ext(&path) {
                    self.accept_file(SourceImage {
                        path,
                        root: root.to_path_buf(),
                    });
                }
            }
        }

        Ok(())
    }

    fn claim_dir(&mut self, dir: &Path, remaining: u8) -> bool {
        match self.visited_dirs.get(dir).copied() {
            Some(previous) if remaining <= previous => false,
            _ => {
                self.visited_dirs.insert(dir.to_path_buf(), remaining);
                true
            }
        }
    }

    fn accept_file(&mut self, image: SourceImage) {
        if self.seen.insert(image.path.clone()) {
            self.images.push(image);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
