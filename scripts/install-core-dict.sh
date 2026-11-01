#!/usr/bin/env bash
# Compile the vendored rime luna_pinyin dict with OpenCC t2s conversion and
# install the .ior binary where the fcitx5 addon looks for it. System dir by
# default; pass `--user` to install into $XDG_DATA_HOME instead (no sudo).
set -euo pipefail
cd "$(dirname "$0")/.."

cargo run -q -p dict-tools --release -- compile --simplify \
  data/dicts/luna_pinyin.dict.yaml data/dicts/core.ior

# Train the character n-gram model from the exact compiled dictionary (the
# export includes preset-vocabulary phrases). The writer compresses oversized
# totals with a common divisor so the `.gram` v1 u32 header stays valid.
tmp_tsv="$(mktemp)"
trap 'rm -f "$tmp_tsv"' EXIT
cargo run -q -p dict-tools --release -- export data/dicts/core.ior "$tmp_tsv"
cargo run -q -p dict-tools --release -- train "$tmp_tsv" data/dicts/core.gram

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
