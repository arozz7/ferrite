use anyhow::Result;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("Ferrite starting");

    ferrite_tui::run().map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}
