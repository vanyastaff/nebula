use std::fs;

use anyhow::{Context, bail};

use crate::config::{self, CliConfig};

/// Execute the `config show` command.
pub async fn show() -> anyhow::Result<()> {
    let config = CliConfig::load().await?;
    let toml = toml::to_string_pretty(&config).expect("config serialization");
    println!("{toml}");

    if let Some(path) = config::find_config_file() {
        eprintln!("# loaded from: {}", path.display());
    } else {
        eprintln!("# no config file found, showing defaults");
    }
    Ok(())
}

/// Execute the `config init` command.
pub fn init(global: bool) -> anyhow::Result<()> {
    let path = if global {
        let dir = config::global_config_dir().context("could not determine home directory")?;
        if !dir.exists() {
            fs::create_dir_all(&dir)
                .with_context(|| format!("failed to create {}", dir.display()))?;
        }
        dir.join("config.toml")
    } else {
        "nebula.toml".into()
    };

    if path.exists() {
        bail!("{} already exists", path.display());
    }

    fs::write(&path, CliConfig::default_toml())
        .with_context(|| format!("failed to write {}", path.display()))?;

    println!("Created {}", path.display());
    Ok(())
}
