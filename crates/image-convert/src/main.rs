mod cli;
mod convert;
mod format;
mod output_path;
mod report;

fn main() -> anyhow::Result<()> {
    let options: convert::ConvertOptions = cli::parse().into();

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
