# media-box

Rust 编写的媒体处理工作区，包含两个独立的命令行工具：

- **media-box**：按「感知质量」而不是码率压缩视频。给定目标 VMAF 分数，在场景样本上搜索刚好达标的 CRF，再用它正式编码整片。编码器固定为 SVT-AV1（`libsvtav1`）。
- **image-convert**：纯 Rust 的图片批量格式转换工具，不依赖外部编码器，支持 png/jpg/jpeg/gif/webp/bmp 常用格式互转。

## 环境要求

- Rust 2024 edition（rustc ≥ 1.95：使用 `cfg_select!` 与 let-chains）
- `image-convert` 无额外要求

仅 `media-box` 还需要：

- `ffmpeg`、`ffprobe` 在 `PATH` 中
- ffmpeg 编译时启用 `--enable-libsvtav1` 与 `--enable-libvmaf`
- libvmaf ≥ 3.2（提供 `vmaf_v1.0.16_3d0h`、`vmaf_v1.0.16_1d5h_2160` 模型）

## 构建

```bash
cargo build --release
```

产物：

- `target/release/media-box`
- `target/release/image-convert`

## media-box

### 快速开始

```bash
media-box video-compress -i D:\videos\a.mkv
```

默认目标 VMAF 91、preset 4、输出到源视频所在目录。正式编码前的 CRF 搜索会对每个场景样本反复「编码 + 测 VMAF」，因此单片耗时明显高于单纯转码；4K 长片尤其明显。

```bash
media-box video-compress -i <输入路径...> [选项]
```

`-i` 可重复指定，目录会按扩展名递归收集视频（不跟随符号链接，绝对的重复路径会去重）。

### 参数

| 参数 | 说明 | 默认 |
|---|---|---|
| `-i, --input <INPUT>` | 源视频文件或目录，可重复 | 必填 |
| `-r, --resolution <WxH>` | 输出最大分辨率（`x`/`X`/`×` 均可），超出则按 lanczos 缩放 | 跟随源 |
| `-o, --output <DIR>` | 输出目录，不存在会自动创建 | 源视频所在目录 |
| `-d, --depth <N>` | 目录递归深度，0 表示只处理该目录本身 | 0 |
| `-f, --fps <FPS>` | 输出帧率上限 | 跟随源 |
| `-p, --preset <0-13>` | SVT-AV1 preset，数值越大越快、体积越大 | 4 |
| `-v, --vmaf <SCORE>` | 目标 VMAF 分数（v1 体系，1-100） | 91 |

### 示例

```bash
# 单文件，输出到源视频所在目录
media-box video-compress -i D:\videos\a.mkv

# 批量目录，限制 1080p / 30fps，preset 6，目标 VMAF 93
media-box video-compress -i D:\videos -r 1920x1080 -f 30 -p 6 -v 93

# 递归两层，输出到指定目录
media-box video-compress -i D:\videos -d 2 -o D:\out
```

### 怎么调

| 想要 | 调整 |
|---|---|
| 更小的文件 | 降 `-v`（如 88~90），或升 `-p` |
| 更好的画质 | 升 `-v`（如 93~95） |
| 编码更快 | 降 `-p`（如 2~3），或加 `-r` 降分辨率 |
| 4K 太慢 | `-r 1920x1080`：搜索和正式编码都会明显变快 |

`-v` 差 1 分通常肉眼难辨，但体积可能差 5%~10%；建议先拿一小段素材试 91 与 93 再决定。

### 输出

- 文件名：`<源文件名>-<yymmddHHMMSS>.mp4`，时间戳为 UTC。
- 位置：`-o` 指定目录或源视频所在目录。
- 成功后在总结页打印源/输出体积、体积变化百分比、源时长与压缩耗时、完整 ffmpeg 命令。

### 中断与失败处理

- `Ctrl+C` 一次：取消当前阶段（含正在运行的 ffmpeg），结束整批；再按一次强制退出（退出码 1）。
- 结束页按「成功 / 失败 / 警告 / 已取消或未处理」分类汇总。
- 元数据读取失败：该文件计为失败，继续下一个。
- 场景检测失败：退化为固定位置采样，不阻塞编码。
- VMAF 测量失败：告警并沿用当前最优 CRF，继续正式编码。
- 正式编码失败或取消：删除半成品输出文件。

### 已知限制

- 只处理扩展名白名单内的视频（mp4/mkv/mov/avi/webm 等），不跟随符号链接。
- CRF 搜索对每个样本都做真实编码 + 测量，4K 长片耗时明显。
- 输出时间戳为 UTC，未引入本地时区依赖。
- 依赖 ffmpeg 的 `libvmaf` 模型名，libvmaf 版本过低会测量失败（仅告警并退回当前 CRF）。

## image-convert

### 快速开始

```bash
image-convert -i D:\images -f webp
```

目标格式用 `-f` 指定；`-i` 可重复，目录按图片扩展名递归收集。默认就地转换，输出到源文件所在目录。

```bash
image-convert -i <输入路径...> -f <目标格式> [选项]
```

### 参数

| 参数 | 说明 | 默认 |
|---|---|---|
| `-i, --input <INPUT>` | 源图片文件或目录，可重复 | 转换时必填 |
| `-o, --output <DIR>` | 输出目录，会镜像源相对 `-i` 的目录结构 | 源所在目录 |
| `-f, --format <FORMAT>` | 目标格式，见下表 | 转换时必填 |
| `-q, --quality <1-100>` | JPEG 有损质量，无损格式忽略 | 85 |
| `-d, --depth <N>` | 目录递归深度，0 表示只看当前目录 | 0 |
| `-j, --jobs <N>` | 并发转换任务数 | CPU 逻辑核数 |
| `--overwrite` | 允许覆盖已存在的输出，默认跳过 | 关 |
| `--dry-run` | 只列出将转换的文件，不写盘 | 关 |
| `--optimize` | PNG 输出启用最高无损压缩 | 关 |
| `--meta <keep/strip/list>` | 源元数据保留、剔除或只列出摘要 | keep |
| `--list-formats` | 打印输出格式清单后退出 | — |

`--list-formats` 与 `--meta list` 只读取信息，不需要 `-f`；其余场景必须显式给出 `-f`。

目标格式：

| 类别 | 格式 |
|---|---|
| 可输出 | png、jpg/jpeg、gif、webp、bmp |

JPEG 使用 `-q`；其余格式为无损，`-q` 不生效。

### 示例

```bash
# 目录转 WebP，就地输出
image-convert -i D:\images -f webp

# 指定输出目录，递归两层，并发 8，JPEG 质量 75
image-convert -i D:\images -f jpg -o D:\out -d 2 -j 8 -q 75

# 只列出，不写盘
image-convert -i D:\images -f png --dry-run

# 覆盖已有输出，PNG 最高压缩
image-convert -i D:\images -f png --overwrite --optimize

# 查看能力矩阵 / 元数据摘要
image-convert --list-formats
image-convert -i D:\images -f png --meta list
```

### 输出

- 文件名：`<源文件名>.<目标扩展名>`。
- 位置：`-o` 指定目录时镜像源相对 `-i` 的目录结构；未指定时输出到源文件所在目录。
- 同批次内同名文件自动追加 `-1`、`-2` 后缀，互不覆盖；磁盘上已存在的文件默认跳过，除非 `--overwrite`。
- 结束打印逐文件结果与汇总（成功 / 跳过 / 失败 / 计划 / 输出字节数）。

### 已知限制

- 个人工具定位，只支持常用格式；用 `--list-formats` 查看完整清单。
- WebP 使用纯 Rust 无损编码，暂不支持有损编码，`-q` 对其无效。
- 元数据保留依赖编码器支持：png / jpg / webp 保留 EXIF/ICC，gif / bmp 不支持；`--meta keep` 读失败只在 stderr 告警。

## 开发与测试

```bash
cargo test                      # 全部
cargo test --test crf_search    # 只跑搜索策略的行为测试
cargo test --test performance   # 只跑性能预算测试（次数/规模上界）
cargo test --lib                # 只跑模块内单元测试
cargo bench -p media-box-core   # 进程内基准（Criterion）
cargo clippy --all-targets      # 建议与测试一起跑
cargo fmt --check
```

`media-box` 的搜索与解析逻辑用合成曲线和固定文本驱动；`image-convert` 为纯 Rust，端到端测试直接使用仓库内自带的图片样本（`crates/image-convert/tests/fixtures/`）。二者测试都不需要外部二进制，也**不联网**；测试临时文件只写在项目内的 `.tmp-tests/`，结束自动删除。

**性能测量不依赖 ffmpeg/ffprobe**：基准（`crates/media-box-core/benches/media_box_core.rs`）只测日志解析、进度节流、场景规划、统计、滤镜拼接、目录扫描和 CRF 搜索状态机等纯计算路径；需要编码与 VMAF 测量的阶段由合成测量器驱动，不会启动任何外部进程。性能预算测试（`tests/performance.rs`、`tests/crf_search.rs`）约束发射次数、样本数量与测量次数上界，防止成本失控。

## 参考文档

- SVT-AV1 官方文档：<https://gitlab.com/AOMediaCodec/SVT-AV1/-/blob/master/Docs>
- HandBrake 文档：<https://handbrake.fr/docs/en/1.11.0/>
- FFmpeg 文档：<https://ffmpeg.org/documentation.html>

## 许可证

MIT。
