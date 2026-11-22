use media_box::cli;
use media_box::compressor::Compressor;
use media_box::interrupter;

fn main() -> anyhow::Result<()> {
    let args = cli::parse();
    let canceller = interrupter::install()?;
    Compressor::new(args).run(&canceller)
}
