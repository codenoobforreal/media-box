mod cli;

use media_box_core::compressor::{CompressOptions, Compressor};
use media_box_core::interrupter;

fn main() -> anyhow::Result<()> {
    let options: CompressOptions = cli::parse().into();
    let canceller = interrupter::install()?;
    Compressor::new(options).run(&canceller)
}
