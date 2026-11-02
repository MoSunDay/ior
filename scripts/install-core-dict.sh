#!/usr/bin/env bash
# Install the prebuilt core dictionary (.ior) and character n-gram model
# (.gram) where the fcitx5 addon looks for them. Both artifacts are
# committed under data/dicts/ (originally compiled from the rime
# luna_pinyin dict + essay preset vocabulary with OpenCC t2s
# simplification; the upstream sources were removed from the repo, see
# data/README.md for attribution). System dir by default; pass `--user`
# to install into $XDG_DATA_HOME instead (no sudo).
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "${1:-}" == "--user" ]]; then
  dest="${XDG_DATA_HOME:-$HOME/.local/share}/fcitx5/ior"
  install -Dm644 data/dicts/core.ior "$dest/core.ior"
  install -Dm644 data/dicts/core.gram "$dest/core.gram"
else
  dest="/usr/share/fcitx5/ior"
  sudo install -Dm644 data/dicts/core.ior "$dest/core.ior"
  sudo install -Dm644 data/dicts/core.gram "$dest/core.gram"
fi
echo "installed: $dest/core.ior"
echo "installed: $dest/core.gram"
