use anyhow::Result;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("Ferrite starting");

    println!("Ferrite — autonomous storage diagnostics & data recovery");
    println!("Run `ferrite --help` for usage (TUI coming in Phase 7)");

    Ok(())
}
