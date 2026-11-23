mod cli;
mod codecs;
mod convert;
mod format;
mod output_path;
mod report;

fn main() -> anyhow::Result<()> {
    codecs::register_input_decoders();

    let args = cli::parse();
    if args.list_formats {
        format::print_capabilities(&mut std::io::stdout().lock())?;
        return Ok(());
    }

    let options: convert::ConvertOptions = args.into();

    if options.format.is_decode_only() {
        anyhow::bail!(
            "目标格式 {} 仅支持解码，暂不支持作为输出格式",
            options.format.extension()
        );
    }
    if !options.format.can_write() {
        anyhow::bail!(
            "目标格式 {} 在当前编译特性下不可写",
            options.format.extension()
        );
    }

    let outcomes = convert::run(options)?;
    let mut stdout = std::io::stdout().lock();
    report::report(&outcomes, &mut stdout)?;
    Ok(())
}
