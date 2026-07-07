use std::path::PathBuf;

use clap::{Parser, Subcommand};
use rawhid_host_core::{
    config::{load_config, write_default_config, ConfigError, ConfigPaths},
    ActiveAppProvider, AppConfig, EncoderBinding, HidConfig, HidDeviceManager, ProbeResult, Runner,
    SystemActiveAppProvider, CAPABILITY_CONFIG_RPC,
};
use thiserror::Error;
use tracing_subscriber::{filter::EnvFilter, fmt};

#[derive(Debug, Parser)]
#[command(name = "keylink-studio")]
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
    /// Probe Config RPC ENCODER GET_INFO on verified Host Link devices.
    ConfigGetInfo,
    /// Probe Config RPC ENCODER GET_BINDINGS on verified Host Link devices.
    ConfigGetBindings {
        #[arg(long)]
        layer_id: u32,
        #[arg(long)]
        encoder_id: u8,
    },
    /// Send Config RPC ENCODER SET_BINDINGS to verified Host Link devices.
    ConfigSetBindings {
        #[arg(long)]
        layer_id: u32,
        #[arg(long)]
        encoder_id: u8,
        #[arg(long)]
        cw_behavior_id: u16,
        #[arg(long)]
        cw_param1: u32,
        #[arg(long)]
        cw_param2: u32,
        #[arg(long)]
        ccw_behavior_id: u16,
        #[arg(long)]
        ccw_param1: u32,
        #[arg(long)]
        ccw_param2: u32,
    },
    /// Probe Config RPC ENCODER GET_DIRTY on verified Host Link devices.
    ConfigGetDirty,
    /// Send Config RPC ENCODER SAVE to verified Host Link devices.
    ConfigSave,
    /// Send Config RPC ENCODER DISCARD to verified Host Link devices.
    ConfigDiscard,
    /// Send Config RPC ENCODER CLEAR_OVERRIDE to verified Host Link devices.
    ConfigClearOverride {
        #[arg(long)]
        layer_id: u32,
        #[arg(long)]
        encoder_id: u8,
    },
}

fn main() -> Result<(), CliError> {
    init_logging();
    let cli = Cli::parse();

    match cli.command.unwrap_or(Command::Run) {
        Command::Run => run(cli.config),
        Command::InitConfig { output, force } => init_config(cli.config, output, force),
        Command::ConfigPath => config_path(cli.config),
        Command::ListDevices => list_devices(cli.config),
        Command::ConfigGetInfo => config_get_info(cli.config),
        Command::ConfigGetBindings {
            layer_id,
            encoder_id,
        } => config_get_bindings(cli.config, layer_id, encoder_id),
        Command::ConfigSetBindings {
            layer_id,
            encoder_id,
            cw_behavior_id,
            cw_param1,
            cw_param2,
            ccw_behavior_id,
            ccw_param1,
            ccw_param2,
        } => config_set_bindings(
            cli.config,
            layer_id,
            encoder_id,
            EncoderBinding {
                behavior_id: cw_behavior_id,
                param1: cw_param1,
                param2: cw_param2,
            },
            EncoderBinding {
                behavior_id: ccw_behavior_id,
                param1: ccw_param1,
                param2: ccw_param2,
            },
        ),
        Command::ConfigGetDirty => config_get_dirty(cli.config),
        Command::ConfigSave => config_save(cli.config),
        Command::ConfigDiscard => config_discard(cli.config),
        Command::ConfigClearOverride {
            layer_id,
            encoder_id,
        } => config_clear_override(cli.config, layer_id, encoder_id),
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

fn config_get_info(config_path: Option<PathBuf>) -> Result<(), CliError> {
    let (config, loaded_from) = load_config(config_path)?;
    print_config_source(&loaded_from);

    let mut manager = HidDeviceManager::real(config.hid)?;
    let results = manager.probe()?;
    let mut verified_count = 0usize;
    let mut config_rpc_count = 0usize;

    for result in results {
        if !result.verified {
            continue;
        }
        if result.device.capabilities & CAPABILITY_CONFIG_RPC != 0 {
            config_rpc_count += 1;
        }
        verified_count += 1;
        print_config_get_info_result(&mut manager, result)?;
    }

    if verified_count == 0 {
        println!("No verified Host Link devices found.");
    } else if config_rpc_count == 0 {
        println!("No verified devices advertise CONFIG_RPC.");
    }

    Ok(())
}

fn config_get_bindings(
    config_path: Option<PathBuf>,
    layer_id: u32,
    encoder_id: u8,
) -> Result<(), CliError> {
    let (config, loaded_from) = load_config(config_path)?;
    print_config_source(&loaded_from);

    let mut manager = HidDeviceManager::real(config.hid)?;
    let results = manager.probe()?;
    let mut verified_count = 0usize;
    let mut config_rpc_count = 0usize;

    for result in results {
        if !result.verified {
            continue;
        }
        if result.device.capabilities & CAPABILITY_CONFIG_RPC != 0 {
            config_rpc_count += 1;
        }
        verified_count += 1;
        print_config_get_bindings_result(&mut manager, result, layer_id, encoder_id)?;
    }

    if verified_count == 0 {
        println!("No verified Host Link devices found.");
    } else if config_rpc_count == 0 {
        println!("No verified devices advertise CONFIG_RPC.");
    }

    Ok(())
}

fn config_set_bindings(
    config_path: Option<PathBuf>,
    layer_id: u32,
    encoder_id: u8,
    cw_binding: EncoderBinding,
    ccw_binding: EncoderBinding,
) -> Result<(), CliError> {
    let (config, loaded_from) = load_config(config_path)?;
    print_config_source(&loaded_from);

    let mut manager = HidDeviceManager::real(config.hid)?;
    let results = manager.probe()?;
    let mut verified_count = 0usize;
    let mut config_rpc_count = 0usize;

    for result in results {
        if !result.verified {
            continue;
        }
        if result.device.capabilities & CAPABILITY_CONFIG_RPC != 0 {
            config_rpc_count += 1;
        }
        verified_count += 1;
        print_config_set_bindings_result(
            &mut manager,
            result,
            layer_id,
            encoder_id,
            cw_binding,
            ccw_binding,
        )?;
    }

    if verified_count == 0 {
        println!("No verified Host Link devices found.");
    } else if config_rpc_count == 0 {
        println!("No verified devices advertise CONFIG_RPC.");
    }

    Ok(())
}

fn config_get_dirty(config_path: Option<PathBuf>) -> Result<(), CliError> {
    let (config, loaded_from) = load_config(config_path)?;
    print_config_source(&loaded_from);

    let mut manager = HidDeviceManager::real(config.hid)?;
    let results = manager.probe()?;
    let mut verified_count = 0usize;
    let mut config_rpc_count = 0usize;

    for result in results {
        if !result.verified {
            continue;
        }
        if result.device.capabilities & CAPABILITY_CONFIG_RPC != 0 {
            config_rpc_count += 1;
        }
        verified_count += 1;
        print_config_get_dirty_result(&mut manager, result)?;
    }

    if verified_count == 0 {
        println!("No verified Host Link devices found.");
    } else if config_rpc_count == 0 {
        println!("No verified devices advertise CONFIG_RPC.");
    }

    Ok(())
}

fn config_save(config_path: Option<PathBuf>) -> Result<(), CliError> {
    let (config, loaded_from) = load_config(config_path)?;
    print_config_source(&loaded_from);

    let mut manager = HidDeviceManager::real(config.hid)?;
    let results = manager.probe()?;
    let mut verified_count = 0usize;
    let mut config_rpc_count = 0usize;

    for result in results {
        if !result.verified {
            continue;
        }
        if result.device.capabilities & CAPABILITY_CONFIG_RPC != 0 {
            config_rpc_count += 1;
        }
        verified_count += 1;
        print_config_save_result(&mut manager, result)?;
    }

    if verified_count == 0 {
        println!("No verified Host Link devices found.");
    } else if config_rpc_count == 0 {
        println!("No verified devices advertise CONFIG_RPC.");
    }

    Ok(())
}

fn config_discard(config_path: Option<PathBuf>) -> Result<(), CliError> {
    let (config, loaded_from) = load_config(config_path)?;
    print_config_source(&loaded_from);

    let mut manager = HidDeviceManager::real(config.hid)?;
    let results = manager.probe()?;
    let mut verified_count = 0usize;
    let mut config_rpc_count = 0usize;

    for result in results {
        if !result.verified {
            continue;
        }
        if result.device.capabilities & CAPABILITY_CONFIG_RPC != 0 {
            config_rpc_count += 1;
        }
        verified_count += 1;
        print_config_discard_result(&mut manager, result)?;
    }

    if verified_count == 0 {
        println!("No verified Host Link devices found.");
    } else if config_rpc_count == 0 {
        println!("No verified devices advertise CONFIG_RPC.");
    }

    Ok(())
}

fn config_clear_override(
    config_path: Option<PathBuf>,
    layer_id: u32,
    encoder_id: u8,
) -> Result<(), CliError> {
    let (config, loaded_from) = load_config(config_path)?;
    print_config_source(&loaded_from);

    let mut manager = HidDeviceManager::real(config.hid)?;
    let results = manager.probe()?;
    let mut verified_count = 0usize;
    let mut config_rpc_count = 0usize;

    for result in results {
        if !result.verified {
            continue;
        }
        if result.device.capabilities & CAPABILITY_CONFIG_RPC != 0 {
            config_rpc_count += 1;
        }
        verified_count += 1;
        print_config_clear_override_result(&mut manager, result, layer_id, encoder_id)?;
    }

    if verified_count == 0 {
        println!("No verified Host Link devices found.");
    } else if config_rpc_count == 0 {
        println!("No verified devices advertise CONFIG_RPC.");
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
    let status = if result.verified {
        "verified"
    } else {
        "not verified"
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

fn print_config_get_info_result(
    manager: &mut HidDeviceManager,
    result: ProbeResult,
) -> Result<(), CliError> {
    let device = result.device;
    println!(
        "verified vid={:04x} pid={:04x} usage_page={:04x} usage={:04x} path={}",
        device.vendor_id, device.product_id, device.usage_page, device.usage, device.path
    );
    if let Some(product) = &device.product {
        println!("  product: {product}");
    }
    if let Some(manufacturer) = &device.manufacturer {
        println!("  manufacturer: {manufacturer}");
    }
    if device.capabilities & CAPABILITY_CONFIG_RPC == 0 {
        println!("  CONFIG_RPC: unsupported");
        return Ok(());
    }

    println!("  CONFIG_RPC: supported");
    match manager.config_get_encoder_info(&device) {
        Ok(info) => {
            println!("  ENCODER GET_INFO: OK");
            println!("    layer_count: {}", info.layer_count);
            println!("    encoder_count: {}", info.encoder_count);
            println!("    capabilities: 0x{:02x}", info.capabilities);
        }
        Err(rawhid_host_core::hid::HidError::ConfigRpcStatus(status)) => {
            println!("  ENCODER GET_INFO: {status:?}");
        }
        Err(error) => {
            println!("  ENCODER GET_INFO: error: {error}");
        }
    }
    Ok(())
}

fn print_config_get_bindings_result(
    manager: &mut HidDeviceManager,
    result: ProbeResult,
    layer_id: u32,
    encoder_id: u8,
) -> Result<(), CliError> {
    let device = result.device;
    println!(
        "verified vid={:04x} pid={:04x} usage_page={:04x} usage={:04x} path={}",
        device.vendor_id, device.product_id, device.usage_page, device.usage, device.path
    );
    if let Some(product) = &device.product {
        println!("  product: {product}");
    }
    if let Some(manufacturer) = &device.manufacturer {
        println!("  manufacturer: {manufacturer}");
    }
    if device.capabilities & CAPABILITY_CONFIG_RPC == 0 {
        println!("  CONFIG_RPC: unsupported");
        return Ok(());
    }

    println!("  CONFIG_RPC: supported");
    match manager.config_get_encoder_bindings(&device, layer_id, encoder_id) {
        Ok(bindings) => {
            println!("  ENCODER GET_BINDINGS: OK");
            println!("    layer_id: {}", bindings.layer_id);
            println!("    encoder_id: {}", bindings.encoder_id);
            println!("    source: {:?}", bindings.source);
            println!("    flags: 0x{:02x}", bindings.flags.bits());
            print_binding("cw_binding", bindings.cw_binding);
            print_binding("ccw_binding", bindings.ccw_binding);
        }
        Err(rawhid_host_core::hid::HidError::ConfigRpcStatus(status)) => {
            println!("  ENCODER GET_BINDINGS: {status:?}");
        }
        Err(error) => {
            println!("  ENCODER GET_BINDINGS: error: {error}");
        }
    }
    Ok(())
}

fn print_config_set_bindings_result(
    manager: &mut HidDeviceManager,
    result: ProbeResult,
    layer_id: u32,
    encoder_id: u8,
    cw_binding: EncoderBinding,
    ccw_binding: EncoderBinding,
) -> Result<(), CliError> {
    let device = result.device;
    println!(
        "verified vid={:04x} pid={:04x} usage_page={:04x} usage={:04x} path={}",
        device.vendor_id, device.product_id, device.usage_page, device.usage, device.path
    );
    if let Some(product) = &device.product {
        println!("  product: {product}");
    }
    if let Some(manufacturer) = &device.manufacturer {
        println!("  manufacturer: {manufacturer}");
    }
    if device.capabilities & CAPABILITY_CONFIG_RPC == 0 {
        println!("  CONFIG_RPC: unsupported");
        return Ok(());
    }

    println!("  CONFIG_RPC: supported");
    match manager.config_set_encoder_bindings(
        &device,
        layer_id,
        encoder_id,
        cw_binding,
        ccw_binding,
    ) {
        Ok(()) => {
            println!("  ENCODER SET_BINDINGS: OK");
        }
        Err(rawhid_host_core::hid::HidError::ConfigRpcStatus(status)) => {
            println!("  ENCODER SET_BINDINGS: {status:?}");
        }
        Err(error) => {
            println!("  ENCODER SET_BINDINGS: error: {error}");
        }
    }
    Ok(())
}

fn print_config_get_dirty_result(
    manager: &mut HidDeviceManager,
    result: ProbeResult,
) -> Result<(), CliError> {
    let device = result.device;
    println!(
        "verified vid={:04x} pid={:04x} usage_page={:04x} usage={:04x} path={}",
        device.vendor_id, device.product_id, device.usage_page, device.usage, device.path
    );
    if let Some(product) = &device.product {
        println!("  product: {product}");
    }
    if let Some(manufacturer) = &device.manufacturer {
        println!("  manufacturer: {manufacturer}");
    }
    if device.capabilities & CAPABILITY_CONFIG_RPC == 0 {
        println!("  CONFIG_RPC: unsupported");
        return Ok(());
    }

    println!("  CONFIG_RPC: supported");
    match manager.config_get_encoder_dirty(&device) {
        Ok(dirty) => {
            println!("  ENCODER GET_DIRTY: OK");
            println!("    dirty: {dirty}");
        }
        Err(rawhid_host_core::hid::HidError::ConfigRpcStatus(status)) => {
            println!("  ENCODER GET_DIRTY: {status:?}");
        }
        Err(error) => {
            println!("  ENCODER GET_DIRTY: error: {error}");
        }
    }
    Ok(())
}

fn print_config_save_result(
    manager: &mut HidDeviceManager,
    result: ProbeResult,
) -> Result<(), CliError> {
    let device = result.device;
    println!(
        "verified vid={:04x} pid={:04x} usage_page={:04x} usage={:04x} path={}",
        device.vendor_id, device.product_id, device.usage_page, device.usage, device.path
    );
    if let Some(product) = &device.product {
        println!("  product: {product}");
    }
    if let Some(manufacturer) = &device.manufacturer {
        println!("  manufacturer: {manufacturer}");
    }
    if device.capabilities & CAPABILITY_CONFIG_RPC == 0 {
        println!("  CONFIG_RPC: unsupported");
        return Ok(());
    }

    println!("  CONFIG_RPC: supported");
    match manager.config_save_encoder(&device) {
        Ok(()) => {
            println!("  ENCODER SAVE: OK");
        }
        Err(rawhid_host_core::hid::HidError::ConfigRpcStatus(status)) => {
            println!("  ENCODER SAVE: {status:?}");
        }
        Err(error) => {
            println!("  ENCODER SAVE: error: {error}");
        }
    }
    Ok(())
}

fn print_config_discard_result(
    manager: &mut HidDeviceManager,
    result: ProbeResult,
) -> Result<(), CliError> {
    let device = result.device;
    println!(
        "verified vid={:04x} pid={:04x} usage_page={:04x} usage={:04x} path={}",
        device.vendor_id, device.product_id, device.usage_page, device.usage, device.path
    );
    if let Some(product) = &device.product {
        println!("  product: {product}");
    }
    if let Some(manufacturer) = &device.manufacturer {
        println!("  manufacturer: {manufacturer}");
    }
    if device.capabilities & CAPABILITY_CONFIG_RPC == 0 {
        println!("  CONFIG_RPC: unsupported");
        return Ok(());
    }

    println!("  CONFIG_RPC: supported");
    match manager.config_discard_encoder(&device) {
        Ok(()) => {
            println!("  ENCODER DISCARD: OK");
        }
        Err(rawhid_host_core::hid::HidError::ConfigRpcStatus(status)) => {
            println!("  ENCODER DISCARD: {status:?}");
        }
        Err(error) => {
            println!("  ENCODER DISCARD: error: {error}");
        }
    }
    Ok(())
}

fn print_config_clear_override_result(
    manager: &mut HidDeviceManager,
    result: ProbeResult,
    layer_id: u32,
    encoder_id: u8,
) -> Result<(), CliError> {
    let device = result.device;
    println!(
        "verified vid={:04x} pid={:04x} usage_page={:04x} usage={:04x} path={}",
        device.vendor_id, device.product_id, device.usage_page, device.usage, device.path
    );
    if let Some(product) = &device.product {
        println!("  product: {product}");
    }
    if let Some(manufacturer) = &device.manufacturer {
        println!("  manufacturer: {manufacturer}");
    }
    if device.capabilities & CAPABILITY_CONFIG_RPC == 0 {
        println!("  CONFIG_RPC: unsupported");
        return Ok(());
    }

    println!("  CONFIG_RPC: supported");
    match manager.config_clear_encoder_override(&device, layer_id, encoder_id) {
        Ok(()) => {
            println!("  ENCODER CLEAR_OVERRIDE: OK");
        }
        Err(rawhid_host_core::hid::HidError::ConfigRpcStatus(status)) => {
            println!("  ENCODER CLEAR_OVERRIDE: {status:?}");
        }
        Err(error) => {
            println!("  ENCODER CLEAR_OVERRIDE: error: {error}");
        }
    }
    Ok(())
}

fn print_binding(label: &str, binding: EncoderBinding) {
    println!(
        "    {label}: behavior_id=0x{:04x} param1={} param2={}",
        binding.behavior_id, binding.param1, binding.param2
    );
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
