#[cfg(feature = "file")]
use nebula_log::{Config, Format, Rolling, WriterConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "file")]
    {
        let config = Config {
            format: Format::Json,
            writer: WriterConfig::File {
                path: "logs/app.log".into(),
                rolling: Some(Rolling::Daily),
                non_blocking: true,
            },
            ..Config::default()
        };

        let _guard = nebula_log::init_with(config)?;

        tracing::info!("Logging to file with daily rotation");

        // Simulate application
        for i in 0..10 {
            tracing::info!(iteration = i, "Processing batch");
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[cfg(not(feature = "file"))]
    println!("File logging requires the 'file' feature");

    Ok(())
}
