#!/usr/bin/env bash
# Compile the vendored rime luna_pinyin dict and install the .ior binary
# where the fcitx5 addon looks for it. System dir by default; pass
# `--user` to install into $XDG_DATA_HOME instead (no sudo needed).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo run -q -p dict-tools --release -- compile \
  data/dicts/luna_pinyin.dict.yaml data/dicts/core.ior

if [[ "${1:-}" == "--user" ]]; then
  dest="${XDG_DATA_HOME:-$HOME/.local/share}/fcitx5/ior"
  install -Dm644 data/dicts/core.ior "$dest/core.ior"
else
  dest="/usr/share/fcitx5/ior"
  sudo install -Dm644 data/dicts/core.ior "$dest/core.ior"
fi
echo "installed: $dest/core.ior"
