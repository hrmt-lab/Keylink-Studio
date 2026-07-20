#!/usr/bin/env bash
set -euo pipefail

workspace=/home/onigiri/zmk-workspace
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
firmware_module="$workspace/config/zmk-rawhid-app"
output="$workspace/build/gate-b-ai-client-model-test"

cc -std=c11 -Wall -Wextra -Werror \
  -I"$firmware_module/include" \
  -I"$firmware_module/src" \
  "$repo_root/tools/host-link-gate-b/firmware-model-test.c" \
  "$firmware_module/src/ai_client_state_model.c" \
  -o "$output"
"$output"
printf 'Firmware AI client state model test passed.\n'
