use media_box_core::cli;
use media_box_core::compressor::Compressor;
use media_box_core::interrupter;

fn main() -> anyhow::Result<()> {
    let args = cli::parse();
    let canceller = interrupter::install()?;
    Compressor::new(args).run(&canceller)
}
