#!/usr/bin/env bash
set -euo pipefail

workspace=/home/onigiri/zmk-workspace
script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
gate_b_conf="$script_dir/gate-b.conf"

cd "$workspace"
nix develop --command west build -p always --cmake-only -s zmk/app \
  -d build/gate-b-screenkeytest -b xiao_ble//zmk -- \
  -DSHIELD="screenkeytest raw_hid_adapter" \
  -DZMK_CONFIG=/home/onigiri/zmk-workspace/config/zmk-config-screenkeytest \
  -DSNIPPET=studio-rpc-usb-uart

module_file="$workspace/build/gate-b-screenkeytest/zephyr_modules.txt"
module_paths="$({
  awk -F'"' '{ print $4 }' "$module_file" |
    sed 's|^/home/onigiri/zmk-workspace/zmk-rawhid-app$|/home/onigiri/zmk-workspace/config/zmk-rawhid-app|'
} | paste -sd';' -)"

nix develop --command west build -p always -s zmk/app -d build/gate-b-screenkeytest \
  -b xiao_ble//zmk -- \
  -DSHIELD="screenkeytest raw_hid_adapter" \
  -DZMK_CONFIG=/home/onigiri/zmk-workspace/config/zmk-config-screenkeytest \
  -DSNIPPET=studio-rpc-usb-uart \
  -DZEPHYR_MODULES="$module_paths" \
  -DEXTRA_CONF_FILE="$gate_b_conf"

uf2_source="$workspace/build/gate-b-screenkeytest/zephyr/zmk.uf2"
firmware_dir="$workspace/firmware"
uf2_destination="$firmware_dir/screenkeytest.uf2"

test -f "$uf2_source"
mkdir -p "$firmware_dir"
cp "$uf2_source" "$uf2_destination"
cmp -s "$uf2_source" "$uf2_destination"

printf 'Gate B UF2 copied to: %s\n' "$uf2_destination"
