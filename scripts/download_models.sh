#!/usr/bin/env bash
# ior model downloader (M0 stub).
#
# This script performs NO network calls and writes NO files in M0:
# it only prints the planned model list. Real downloads land in
# milestone V4 of the voice phase. Kept dependency-free on purpose.

set -euo pipefail

echo "ior model download plan (M0 stub, nothing downloaded)"
echo
echo "voice models (crates/voice :: default_model_manifest):"
printf '  %-36s %-22s %8s\n' "MODEL" "ROLE" "SIZE"
printf '  %-36s %-22s %8s\n' "silero-vad" "vad" "~2 MB"
printf '  %-36s %-22s %8s\n' "sherpa-onnx-zipformer-zh-en" "asr-stream" "~100 MB"
printf '  %-36s %-22s %8s\n' "whisper-small" "asr-batch-accurate" "~460 MB"
printf '  %-36s %-22s %8s\n' "whisper-turbo" "asr-batch-fast" "~750 MB"
echo
echo "llm models (GGUF, Q4_K_M quantization):"
printf '  %-36s %-22s %8s\n' "qwen2.5-1.5b-instruct-q4" "fast polish" "~1 GB"
printf '  %-36s %-22s %8s\n' "qwen2.5-3b-instruct-q4" "balanced polish" "~2 GB"
printf '  %-36s %-22s %8s\n' "qwen2.5-7b-instruct-q4" "async re-check" "~4.5 GB"
echo
echo "Models will be stored under data/models/ (git-ignored)."
echo "TODO: implement downloads in V4"
exit 0
