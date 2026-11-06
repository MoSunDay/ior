#!/usr/bin/env bash
# ior model downloader: optional local SenseVoice ASR model.
#
# The default install stays remote-only. `--sensevoice` downloads the
# official sherpa-onnx SenseVoice-Small int8 bundle (~230 MB) into
# $XDG_DATA_HOME/ior/models/sensevoice-small/, which the voice crate probes
# automatically when voice.asr.engine = "auto".
#
# VAD / streaming / batch models remain roadmap items and are listed only.

set -euo pipefail

MODELS_DIR="${IOR_MODELS_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/ior/models}"
SENSEVOICE_NAME="sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17"
SENSEVOICE_DIR="$MODELS_DIR/sensevoice-small"
SENSEVOICE_URLS=(
	"https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/${SENSEVOICE_NAME}.tar.bz2"
	"https://www.modelscope.cn/models/csukuangfj/${SENSEVOICE_NAME}/resolve/master/${SENSEVOICE_NAME}.tar.bz2"
)
if [ -n "${IOR_SENSEVOICE_URL:-}" ]; then
	SENSEVOICE_URLS=("$IOR_SENSEVOICE_URL")
fi
TMP_DIR=""

cleanup() {
	if [ -n "$TMP_DIR" ]; then
		rm -rf "$TMP_DIR"
	fi
}
trap cleanup EXIT

usage() {
	cat <<EOF
usage: scripts/download_models.sh [--sensevoice | --list]

  --sensevoice   download SenseVoice-Small int8 into
                 $SENSEVOICE_DIR
  --list         print the model manifest without downloading
  (no args)      same as --list
EOF
}

list_models() {
	echo "ior voice model manifest:"
	printf '  %-36s %-22s %8s\n' "MODEL" "ROLE" "SIZE"
	printf '  %-36s %-22s %8s\n' "sensevoice-small" "asr-local" "~230 MB"
	printf '  %-36s %-22s %8s\n' "silero-vad" "vad" "~2 MB"
	printf '  %-36s %-22s %8s\n' "sherpa-onnx-zipformer-zh-en" "asr-stream" "~100 MB"
	printf '  %-36s %-22s %8s\n' "whisper-small" "asr-batch-accurate" "~460 MB"
	printf '  %-36s %-22s %8s\n' "whisper-turbo" "asr-batch-fast" "~750 MB"
	echo
	echo "Only sensevoice-small has a real download today; the rest are V2+."
	echo "The binary (sherpa-onnx-offline) is documented in docs/SENSEVOICE-LOCAL.md."
}

download_file() {
	local url="$1"
	local dest="$2"
	if command -v curl >/dev/null 2>&1; then
		curl -fL --retry 3 --connect-timeout 15 -o "$dest" "$url"
	elif command -v wget >/dev/null 2>&1; then
		wget -q --show-progress -O "$dest" "$url"
	else
		echo "error: need curl or wget to download models" >&2
		return 1
	fi
}

sensevoice_ready() {
	[ -f "$SENSEVOICE_DIR/model.int8.onnx" ] && [ -f "$SENSEVOICE_DIR/tokens.txt" ]
}

download_sensevoice() {
	if sensevoice_ready; then
		echo "SenseVoice already present: $SENSEVOICE_DIR"
		return 0
	fi
	mkdir -p "$MODELS_DIR"
	TMP_DIR="$(mktemp -d "$MODELS_DIR/.sensevoice.XXXXXX")"
	local archive="$TMP_DIR/$SENSEVOICE_NAME.tar.bz2"
	local url ok=0
	for url in "${SENSEVOICE_URLS[@]}"; do
		echo "trying $url"
		if download_file "$url" "$archive"; then
			ok=1
			break
		fi
		rm -f "$archive"
	done
	if [ "$ok" -ne 1 ]; then
		echo "error: all SenseVoice download sources failed" >&2
		return 1
	fi

	echo "extracting $archive"
	tar -xjf "$archive" -C "$TMP_DIR"
	local src="$TMP_DIR/$SENSEVOICE_NAME"
	if [ ! -d "$src" ]; then
		src="$(find "$TMP_DIR" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
	fi
	if [ -z "$src" ] || [ ! -d "$src" ]; then
		echo "error: archive did not contain a model directory" >&2
		return 1
	fi
	mkdir -p "$SENSEVOICE_DIR"
	cp -a "$src"/. "$SENSEVOICE_DIR"/
	if ! sensevoice_ready; then
		echo "error: model.int8.onnx / tokens.txt missing after extraction" >&2
		return 1
	fi
	echo "installed: $SENSEVOICE_DIR"
	echo
	echo "Enable it with the default [voice.asr] section (engine = \"auto\"),"
	echo "or explicitly with engine = \"sensevoice\"."
}

case "${1:-}" in
--sensevoice)
	download_sensevoice
	;;
--list)
	list_models
	;;
-h | --help)
	usage
	;;
"")
	list_models
	;;
*)
	usage >&2
	exit 1
	;;
esac
