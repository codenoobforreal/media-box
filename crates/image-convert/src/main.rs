//! image-convert 入口：解析参数，执行转换或信息查询。

mod cli;
mod codecs;
mod convert;
mod format;
mod metadata;
mod output_path;
mod report;

fn main() -> anyhow::Result<()> {
    let args = cli::parse();
    if args.list_formats {
        format::print_capabilities(&mut std::io::stdout().lock())?;
        return Ok(());
    }

    args.validate()?;

    let options: convert::ConvertOptions = args.into();

    if options.metadata == metadata::MetadataMode::List {
        convert::list_metadata(&options, &mut std::io::stdout().lock())?;
        return Ok(());
    }

    let outcomes = convert::run(options)?;
    let mut stdout = std::io::stdout().lock();
    report::report(&outcomes, &mut stdout)?;
    Ok(())
}
