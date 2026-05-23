use std::path::PathBuf;

use clap::{Parser, Subcommand};
use rawhid_host_core::{
    config::{load_config, write_default_config, ConfigError, ConfigPaths},
    ActiveAppProvider, AppConfig, HidConfig, HidDeviceManager, ProbeResult, Runner,
    SystemActiveAppProvider,
};
use thiserror::Error;
use tracing_subscriber::{filter::EnvFilter, fmt};

#[derive(Debug, Parser)]
#[command(name = "rawhid-host")]
#[command(about = "Switch ZMK layers from the active host application over Raw HID")]
struct Cli {
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start watching the active application and send layer packets.
    Run,
    /// Write a sample TOML configuration.
    InitConfig {
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        force: bool,
    },
    /// Print the configuration path that would be used.
    ConfigPath,
    /// List Raw HID candidates and HELLO results.
    ListDevices,
}

fn main() -> Result<(), CliError> {
    init_logging();
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Run) {
        Command::Run => run(cli.config),
        Command::InitConfig { output, force } => init_config(cli.config, output, force),
        Command::ConfigPath => config_path(cli.config),
        Command::ListDevices => list_devices(cli.config),
    }
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();
}

fn run(config_path: Option<PathBuf>) -> Result<(), CliError> {
    let (config, loaded_from) = load_config(config_path)?;
    print_config_source(&loaded_from);

    let app_provider = SystemActiveAppProvider;
    // Fail fast on unsupported platforms so the user gets a clear message.
    let _ = app_provider.active_app()?;

    let hid = HidDeviceManager::real(config.hid.clone())?;
    let mut runner = Runner::new(config, app_provider, hid);
    runner.run_forever();
}

fn init_config(
    global_config: Option<PathBuf>,
    output: Option<PathBuf>,
    force: bool,
) -> Result<(), CliError> {
    let path = match output {
        Some(output) => output,
        None => ConfigPaths::discover(global_config)
            .selected_path()
            .ok_or(CliError::NoConfigPath)?,
    };
    write_default_config(&path, force)?;
    println!("{}", path.display());
    Ok(())
}

fn config_path(config_path: Option<PathBuf>) -> Result<(), CliError> {
    let paths = ConfigPaths::discover(config_path);
    let path = paths.selected_path().ok_or(CliError::NoConfigPath)?;
    println!("{}", path.display());
    Ok(())
}

fn list_devices(config_path: Option<PathBuf>) -> Result<(), CliError> {
    let (config, loaded_from) = load_config(config_path)?;
    print_config_source(&loaded_from);

    let mut manager = HidDeviceManager::real(config.hid)?;
    let results = manager.probe()?;
    if results.is_empty() {
        println!("No HID candidates found.");
        return Ok(());
    }

    for result in results {
        print_probe_result(result);
    }

    Ok(())
}

fn print_config_source(path: &Option<PathBuf>) {
    match path {
        Some(path) => eprintln!("Using config: {}", path.display()),
        None => eprintln!("Using default config; no config file found."),
    }
}

fn print_probe_result(result: ProbeResult) {
    let status = if result.hello_ok {
        "HELLO ok"
    } else {
        "HELLO failed"
    };
    println!(
        "{} vid={:04x} pid={:04x} usage_page={:04x} usage={:04x} path={}",
        status,
        result.device.vendor_id,
        result.device.product_id,
        result.device.usage_page,
        result.device.usage,
        result.device.path
    );
    if let Some(product) = result.device.product {
        println!("  product: {}", product);
    }
    if let Some(manufacturer) = result.device.manufacturer {
        println!("  manufacturer: {}", manufacturer);
    }
    if let Some(error) = result.error {
        println!("  error: {}", error);
    }
}

#[derive(Debug, Error)]
enum CliError {
    #[error("{0}")]
    Config(#[from] ConfigError),
    #[error("{0}")]
    ActiveApp(#[from] rawhid_host_core::active_app::ActiveAppError),
    #[error("{0}")]
    Hid(#[from] rawhid_host_core::hid::HidError),
    #[error("could not determine a config path")]
    NoConfigPath,
}

#[allow(dead_code)]
fn _keep_types_public_for_tauri(_: AppConfig, _: HidConfig) {}
