use std::{path::PathBuf, str::FromStr};

use clap::{Parser, Subcommand};
use rawhid_host_core::{
    config::{load_config, write_default_config, ConfigError, ConfigPaths},
    ActiveAppProvider, AppConfig, ComboBinding, ComboItem, DeviceConnectionType, DeviceInfo,
    EncoderBinding, HidConfig, HidDeviceManager, ProbeResult, Runner, SystemActiveAppProvider,
    CAPABILITY_CONFIG_RPC,
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
    /// Read Config RPC COMBO GET_INFO (all devices, or one --uid).
    ComboGetInfo {
        #[arg(long)]
        uid: Option<DeviceUid>,
    },
    /// Read one Config RPC COMBO slot (all devices, or one --uid).
    ComboGetCombo {
        #[arg(long)]
        uid: Option<DeviceUid>,
        #[arg(long)]
        slot: u8,
    },
    /// Read Config RPC COMBO dirty state (all devices, or one --uid).
    ComboGetDirty {
        #[arg(long)]
        uid: Option<DeviceUid>,
    },
    /// Upsert one runtime Combo on the explicitly selected keyboard.
    ComboSet {
        #[arg(long)]
        uid: DeviceUid,
        #[arg(long)]
        slot: u8,
        #[arg(long)]
        name: String,
        #[arg(long, value_delimiter = ',', num_args = 2..=8)]
        key_positions: Vec<u16>,
        #[arg(long)]
        slow_release: bool,
        #[arg(long)]
        behavior_id: u16,
        #[arg(long, default_value_t = 0)]
        param1: u32,
        #[arg(long, default_value_t = 0)]
        param2: u32,
        #[arg(long, default_value_t = 0)]
        layer_mask: u32,
        #[arg(long, default_value_t = 50)]
        timeout_ms: u16,
        #[arg(long)]
        prior_idle_ms: Option<u16>,
    },
    /// Delete one runtime Combo on the explicitly selected keyboard.
    ComboDelete {
        #[arg(long)]
        uid: DeviceUid,
        #[arg(long)]
        slot: u8,
    },
    /// Persist all dirty Combos on the explicitly selected keyboard.
    ComboSave {
        #[arg(long)]
        uid: DeviceUid,
    },
    /// Discard all dirty Combos on the explicitly selected keyboard.
    ComboDiscard {
        #[arg(long)]
        uid: DeviceUid,
    },
    /// Reset runtime Combos to .keymap defaults on the explicitly selected keyboard.
    ComboResetToKeymap {
        #[arg(long)]
        uid: DeviceUid,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceUid(u64);

impl FromStr for DeviceUid {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.strip_prefix("uid:").unwrap_or(value);
        let value = value.strip_prefix("0x").unwrap_or(value);
        if value.len() != 16 {
            return Err("UID must be exactly 16 hexadecimal digits".into());
        }
        u64::from_str_radix(value, 16)
            .map(Self)
            .map_err(|error| format!("invalid UID: {error}"))
    }
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
        Command::ComboGetInfo { uid } => combo_get_info(cli.config, uid),
        Command::ComboGetCombo { uid, slot } => combo_get_combo(cli.config, uid, slot),
        Command::ComboGetDirty { uid } => combo_get_dirty(cli.config, uid),
        Command::ComboSet {
            uid,
            slot,
            name,
            key_positions,
            slow_release,
            behavior_id,
            param1,
            param2,
            layer_mask,
            timeout_ms,
            prior_idle_ms,
        } => combo_set(
            cli.config,
            uid,
            ComboItem::new(
                slot,
                &name,
                &key_positions,
                slow_release,
                ComboBinding {
                    behavior_id,
                    param1,
                    param2,
                },
                layer_mask,
                timeout_ms,
                prior_idle_ms,
            )?,
        ),
        Command::ComboDelete { uid, slot } => combo_delete(cli.config, uid, slot),
        Command::ComboSave { uid } => combo_save(cli.config, uid),
        Command::ComboDiscard { uid } => combo_discard(cli.config, uid),
        Command::ComboResetToKeymap { uid } => combo_reset_to_keymap(cli.config, uid),
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
        "{} vid={:04x} pid={:04x} usage_page={:04x} usage={:04x} capabilities={:#010x} uid={} path={}",
        status,
        result.device.vendor_id,
        result.device.product_id,
        result.device.usage_page,
        result.device.usage,
        result.device.capabilities,
        result
            .device
            .device_uid_hash
            .map(|uid| format!("uid:{uid:016x}"))
            .unwrap_or_else(|| "unavailable".to_string()),
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

fn probe_combo_devices(
    config_path: Option<PathBuf>,
    uid: Option<DeviceUid>,
) -> Result<(HidDeviceManager, Vec<DeviceInfo>), CliError> {
    let (config, loaded_from) = load_config(config_path)?;
    print_config_source(&loaded_from);
    let mut manager = HidDeviceManager::real(config.hid)?;
    let results = manager.probe()?;
    let mut devices = Vec::<DeviceInfo>::new();
    for result in results {
        let device = result.device;
        if !result.verified || device.capabilities & CAPABILITY_CONFIG_RPC == 0 {
            continue;
        }
        if uid.is_some_and(|target| device.device_uid_hash != Some(target.0)) {
            continue;
        }
        if let Some(existing_index) = device.device_uid_hash.and_then(|device_uid| {
            devices
                .iter()
                .position(|existing| existing.device_uid_hash == Some(device_uid))
        }) {
            if devices[existing_index].connection_type != DeviceConnectionType::Usb
                && device.connection_type == DeviceConnectionType::Usb
            {
                devices[existing_index] = device;
            }
        } else {
            devices.push(device);
        }
    }
    if devices.is_empty() {
        return Err(match uid {
            Some(uid) => CliError::ComboDeviceNotFound(uid.0),
            None => CliError::NoComboDevices,
        });
    }
    Ok((manager, devices))
}

fn combo_mutation_device(
    config_path: Option<PathBuf>,
    uid: DeviceUid,
) -> Result<(HidDeviceManager, DeviceInfo), CliError> {
    let (manager, mut devices) = probe_combo_devices(config_path, Some(uid))?;
    if devices.len() != 1 {
        return Err(CliError::AmbiguousComboDevice(uid.0));
    }
    Ok((manager, devices.remove(0)))
}

fn print_combo_device(device: &DeviceInfo) {
    let uid = device
        .device_uid_hash
        .map(|uid| format!("{uid:016x}"))
        .unwrap_or_else(|| "none".into());
    println!(
        "COMBO device uid={} vid={:04x} pid={:04x} connection={:?} path={}",
        uid, device.vendor_id, device.product_id, device.connection_type, device.path
    );
}

fn combo_get_info(config_path: Option<PathBuf>, uid: Option<DeviceUid>) -> Result<(), CliError> {
    let (mut manager, devices) = probe_combo_devices(config_path, uid)?;
    for device in devices {
        print_combo_device(&device);
        match manager.config_get_combo_info(&device) {
            Ok(info) => {
                println!("  COMBO GET_INFO: OK");
                println!("    max_combos: {}", info.max_combos);
                println!("    max_keys_per_combo: {}", info.max_keys_per_combo);
                println!("    combo_count: {}", info.combo_count);
                println!("    flags: 0x{:02x}", info.flags.bits());
                println!("    occupied_slots: 0x{:08x}", info.occupied_slots);
                println!("    stale_slots: 0x{:08x}", info.stale_slots);
                println!("    invalid_slots: 0x{:08x}", info.invalid_slots);
            }
            Err(rawhid_host_core::hid::HidError::ConfigRpcStatus(status)) => {
                println!("  COMBO GET_INFO: {status:?}");
            }
            Err(error) => println!("  COMBO GET_INFO: error: {error}"),
        }
    }
    Ok(())
}

fn combo_get_combo(
    config_path: Option<PathBuf>,
    uid: Option<DeviceUid>,
    slot: u8,
) -> Result<(), CliError> {
    let (mut manager, devices) = probe_combo_devices(config_path, uid)?;
    for device in devices {
        print_combo_device(&device);
        match manager.config_get_combo(&device, slot) {
            Ok(item) => print_combo_item(item),
            Err(rawhid_host_core::hid::HidError::ConfigRpcStatus(status)) => {
                println!("  COMBO GET_COMBO slot={slot}: {status:?}");
            }
            Err(error) => println!("  COMBO GET_COMBO slot={slot}: error: {error}"),
        }
    }
    Ok(())
}

fn combo_get_dirty(config_path: Option<PathBuf>, uid: Option<DeviceUid>) -> Result<(), CliError> {
    let (mut manager, devices) = probe_combo_devices(config_path, uid)?;
    for device in devices {
        print_combo_device(&device);
        match manager.config_get_combo_dirty(&device) {
            Ok(dirty) => println!("  COMBO GET_DIRTY: OK dirty={dirty}"),
            Err(rawhid_host_core::hid::HidError::ConfigRpcStatus(status)) => {
                println!("  COMBO GET_DIRTY: {status:?}");
            }
            Err(error) => println!("  COMBO GET_DIRTY: error: {error}"),
        }
    }
    Ok(())
}

fn print_combo_item(item: ComboItem) {
    let positions = &item.key_positions[..item.key_count as usize];
    let prior_idle = item
        .require_prior_idle_ms
        .map(|value| value.to_string())
        .unwrap_or_else(|| "disabled".into());
    println!("  COMBO GET_COMBO slot={}: OK", item.slot);
    println!("    name: {}", item.name.as_str());
    println!("    key_positions: {positions:?}");
    println!("    slow_release: {}", item.flags.slow_release());
    println!(
        "    binding: behavior_id=0x{:04x} param1={} param2={}",
        item.binding.behavior_id, item.binding.param1, item.binding.param2
    );
    println!("    layer_mask: 0x{:08x}", item.layer_mask);
    println!("    timeout_ms: {}", item.timeout_ms);
    println!("    require_prior_idle_ms: {prior_idle}");
}

fn combo_set(
    config_path: Option<PathBuf>,
    uid: DeviceUid,
    item: ComboItem,
) -> Result<(), CliError> {
    let (mut manager, device) = combo_mutation_device(config_path, uid)?;
    print_combo_device(&device);
    manager.config_set_combo(&device, item)?;
    println!("  COMBO SET_COMBO slot={}: OK", item.slot);
    Ok(())
}

fn combo_delete(config_path: Option<PathBuf>, uid: DeviceUid, slot: u8) -> Result<(), CliError> {
    let (mut manager, device) = combo_mutation_device(config_path, uid)?;
    print_combo_device(&device);
    manager.config_delete_combo(&device, slot)?;
    println!("  COMBO DELETE_COMBO slot={slot}: OK");
    Ok(())
}

fn combo_save(config_path: Option<PathBuf>, uid: DeviceUid) -> Result<(), CliError> {
    let (mut manager, device) = combo_mutation_device(config_path, uid)?;
    print_combo_device(&device);
    manager.config_save_combos(&device)?;
    println!("  COMBO SAVE: OK");
    Ok(())
}

fn combo_discard(config_path: Option<PathBuf>, uid: DeviceUid) -> Result<(), CliError> {
    let (mut manager, device) = combo_mutation_device(config_path, uid)?;
    print_combo_device(&device);
    manager.config_discard_combos(&device)?;
    println!("  COMBO DISCARD: OK");
    Ok(())
}

fn combo_reset_to_keymap(config_path: Option<PathBuf>, uid: DeviceUid) -> Result<(), CliError> {
    let (mut manager, device) = combo_mutation_device(config_path, uid)?;
    print_combo_device(&device);
    manager.config_reset_combos_to_keymap(&device)?;
    println!("  COMBO RESET_TO_KEYMAP: OK");
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
    #[error("{0}")]
    Packet(#[from] rawhid_host_core::packet::PacketError),
    #[error("could not determine a config path")]
    NoConfigPath,
    #[error("no verified CONFIG_RPC devices with a usable Host Link endpoint were found")]
    NoComboDevices,
    #[error("no verified CONFIG_RPC device matched uid:{0:016x}")]
    ComboDeviceNotFound(u64),
    #[error("multiple CONFIG_RPC devices matched uid:{0:016x}")]
    AmbiguousComboDevice(u64),
}

#[allow(dead_code)]
fn _keep_types_public_for_tauri(_: AppConfig, _: HidConfig) {}
