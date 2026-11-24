# ffmpeg_examples

解析代码所依赖的外部输出样本与字段说明。这里的文件不是给用户看的教程，而是给 Agent 核对**文本契约**用的：解析逻辑一旦改动，先对照这里的样本确认字段名、顺序与格式没有变。

## 文件与对应代码

| 文件 | 内容 | 对应代码 |
|---|---|---|
| `ffmpeg progress pipe example.txt` | `ffmpeg -progress pipe:1` 的原始输出（`out_time_ms` / `total_size` / `progress` 等键） | `media-box-core/src/progress.rs`：`FfmpegProgressLineParser` |
| `ffprobe params explain.md` | ffprobe 流 / 格式各字段含义 | `media-box-core/src/probe.rs`：`ProbeRaw::absorb` 读取的字段 |
| `metadata.jsonc` | `ffprobe ... -of json` 的完整输出样本 | 字段名与 `ffprobe params explain.md` 对照 |

## 注意

- 样本是某个历史素材的快照，字段顺序与实际输出可能不同；解析逻辑按**键名**而非行序匹配，不要因为样本顺序变化而改代码。
- 修改 `progress.rs` 的键名、或 `probe.rs` 的字段集合时，应同步更新本目录说明与对应样本。
