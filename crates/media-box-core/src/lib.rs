//! media-box 生态核心库：媒体扫描、输出路径、VMAF 目标搜索与 SVT-AV1 编码。库不依赖 clap，由二进制层映射 CLI 参数。

pub mod byte_size;
#[cfg(feature = "app")]
pub mod compressor;
pub mod crf_probe;
pub mod crf_search;
pub mod display_width;
pub mod encode_job;
pub mod encoder;
pub(crate) mod ffmpeg_args;
pub mod film_grain;
pub mod format;
pub mod interrupter;
pub mod media_scan;
pub mod output_path;
pub mod path_budget;
pub mod probe;
pub mod process;
pub mod progress;
pub mod report;
pub mod resolution;
pub mod scene_detect;
pub mod scene_sampler;
pub mod stats;
#[cfg(feature = "app")]
pub mod ui;
pub mod value;
pub mod vmaf;
pub mod ydif_probe;
pub mod ydif_stats;

pub use value::{
    ConstantRateFactor, EncoderPreset, FilmGrain, FrameRatePerSecond, Quality, VmafScore,
};
