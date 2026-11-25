//! media-box 入口：解析参数并启动压缩流水线。

mod cli;

use media_box_core::compressor::Compressor;
use media_box_core::interrupter;

fn main() -> anyhow::Result<()> {
    let options = cli::parse();
    let canceller = interrupter::install()?;
    Compressor::new(options).run(&canceller)
}
