# Keylink Studio

[日本語 README](README.md)

Keylink Studio is a Windows host application built to work alongside ZMK keyboards. It can switch keyboard layers automatically based on the app you're using, show the PC's clock on the keyboard's display, and let keys on the keyboard trigger actions on the PC. It also supports ZMK Studio keymap editing, so you can adjust layers, encoder assignments, and combos right from the GUI.

This repository contains only the PC-side application. Host Link features such as layer switching and time sync require [compatible ZMK firmware](https://github.com/hrmt-lab/zmk-rawhid-app). Keymap editing goes through a separate path, ZMK Studio RPC, so it works on any Studio-enabled firmware even without Host Link support. Firmware compatibility details are covered in [Compatibility](docs/compatibility.md) (Japanese only for now).

## Main features

- Automatic layer switching by app. The app watches the foreground application (executable name, window title, and so on) and switches the keyboard's layer accordingly. Rules are set per keyboard, so it's fine if different keyboards use different layer layouts.
- PC actions from the keyboard. A key bound on the keyboard side can launch an app, open a folder, or stop monitoring on the PC. This is disabled by default to avoid accidental triggers, and nothing runs until you turn it on.
- Time sync. Sends the PC's clock to the keyboard's display. Display format, 12/24-hour mode, and time zone are all configurable.
- AI usage display. Sends Codex and Claude Code usage rates to the keyboard so you can check them at a glance. Also disabled by default.
- Battery display. Shows battery level for supported keyboards on the Devices screen and in the system tray tooltip. If the same keyboard is visible over both USB and Bluetooth, it's shown as a single device rather than two.
- Typing stats and a key tester. View a heatmap of how often each key is pressed, or watch key presses in real time.
- ZMK Studio keymap viewing and editing. View and edit layers and key assignments from the GUI, including not just regular keys but layer-move behaviors, tap-hold, sticky keys, Bluetooth actions, and more.
- Encoder and combo editing. On supported keyboards, you can edit encoder rotation assignments and add or edit combos (multi-key chords) from the GUI as well.
- Keymap backup and restore. Export regular keys, encoder assignments, and combos to a JSON file, then restore them later. Handy as a safety net after reflashing firmware wipes out your settings.

## Quick start

To try the app, set up a development environment and run:

```powershell
.\dev.ps1
```

This starts the GUI in development mode. Connect your keyboard over USB or Bluetooth, then use `Scan` on the `Devices` screen to check that it's detected. First-run steps and firmware requirements are covered in detail in the [Setup Guide](docs/manual-setup.md) (Japanese only for now).

To build a distributable package:

```powershell
.\build-release.ps1
```

The output goes into `target/` (not tracked in this repository).

## GUI screens

- Devices: Start/stop monitoring, check Host Link and ZMK Studio detection status, battery levels, and recent logs.
- Layer Rules: Edit per-app layer rules. Changes are saved automatically as you make them.
- Actions: Configure bindings that let keys on the keyboard trigger PC-side actions.
- Time Sync: Enable/disable time sync and set display format and sync interval.
- AI Usage: Configure and check the status of Codex / Claude Code usage sending.
- Keymap Viewer: View the keymap, check the heatmap, use the key tester, and edit the keymap.
- Settings: Configure appearance, polling, HID, and startup behavior.

The UI can switch between Japanese and English, and the accent color can be changed from Settings. Detailed instructions for each screen are in the [App Usage Manual](docs/manual-app-usage.md) (Japanese only for now).

## ZMK Studio keymap editing

In Keymap Viewer's edit mode, you can rewrite the keymap on the actual device on the spot. Beyond plain key assignments, supported keyboards let you edit encoder CW/CCW assignments and add, edit, or delete combos. Changes take effect on the device's unsaved state as soon as you pick a key, but you need to press `Save` for them to survive a restart. `Discard changes` can undo them at any time.

A few things worth knowing:

- What ZMK Studio saves lives in the device's settings/NVS storage, not in the firmware's `.keymap` source itself. A full erase or a settings-reset flash can wipe out keymap edits you made through Studio.
- Editing isn't possible while Studio is locked. Run `&studio_unlock` on the keyboard side before opening the editor.
- Encoders and combos go through a separate communication path (Host Link Config RPC), so they require the matching capability and a Host Link connection with the same UID as the Studio device. Save or discard can succeed on one path and fail on the other, in which case the app shows the result for each path separately.

`Export` / `Restore` let you write out regular keys, encoder assignments, and combos as a backup and bring them back later. This is meant as an operational safety net, though — it doesn't generate `.keymap` source or write anything back into the firmware. Detailed behavior for edit mode, its constraints, and notes on BLE connections are covered in the [App Usage Manual](docs/manual-app-usage.md) (Japanese only for now).

## About AI Usage

AI Usage is disabled by default. When enabled, it sends Codex / Claude Code's 5-hour and 7-day usage rates, along with reset times, to a supported keyboard. Codex prefers `rate_limits` from its session history, and only falls back to an estimate from local history when that isn't available. Claude Code treats the OAuth usage API as an experimental source. In both cases, the access tokens, credentials, or raw API response contents themselves never show up in the UI or logs.

## For developers

### Project layout

```text
Keylink-Studio/
├─ crates/
│  ├─ rawhid-host-core/   # core logic: config, packets, HID, runner, AI usage, ZMK Studio
│  ├─ rawhid-host-cli/    # CLI
│  └─ rawhid-host-tauri/  # Tauri commands and the monitoring thread
├─ ui/                    # React + TypeScript + Vite UI
├─ docs/                  # detailed documentation
├─ examples/              # example config files
├─ create-icons.ps1
├─ dev.ps1
└─ build-release.ps1
```

### CLI

If you want to check behavior or run scripts without the GUI, the CLI covers that.

```powershell
cargo run -p rawhid-host-cli -- list-devices
cargo run -p rawhid-host-cli -- run
cargo run -p rawhid-host-cli -- init-config --output keylink-studio.toml
cargo run -p rawhid-host-cli -- config-path
```

### Config file

The Settings screen in the GUI covers most configuration, so you shouldn't need to touch `keylink-studio.toml` directly in normal use. If you do need to look at it for troubleshooting or fine-tuning, the full list of options and the file lookup order are documented in the [Setup Guide](docs/manual-setup.md) (Japanese only for now).

### Build

```powershell
cd ui && npm run build   # UI only
cargo build              # Rust / CLI
.\dev.ps1                # Tauri dev launch
.\build-release.ps1      # distributable build
```

## Further documentation

Most of the following documents are Japanese only for now.

- [Compatibility](docs/compatibility.md)
- [Setup Guide](docs/manual-setup.md)
- [App Usage Manual](docs/manual-app-usage.md)
- [Technology Overview](docs/technology-overview.md)
- [Spec](docs/spec.md)
- [Packet Spec](docs/packet-spec.md)
