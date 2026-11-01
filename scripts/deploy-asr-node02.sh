#!/usr/bin/env bash
# node02 语音服务一键部署（V1：Qwen3-ASR + 可选 Qwen2.5 润色）。
#
# 在本机执行，通过 ssh 在 node02 上安装并启动 OpenAI 兼容服务：
#   ior-asr.service  →  vllm serve <asr-model>  --port 8000
#   ior-llm.service  →  vllm serve <llm-model>  --port 8001   (--with-llm)
#
# 约束：脚本不写入任何密钥（默认无鉴权，仅限可信内网，见
# docs/NODE02-DEPLOY.md）；模型经 ModelScope 下载（国内无需外网）。
#
# 用法：
#   scripts/deploy-asr-node02.sh [user@]node02 [选项]
# 选项：
#   --asr-port N        ASR 端口（默认 8000）
#   --asr-model ID      ASR 模型（默认 Qwen/Qwen3-ASR-4B）
#   --with-llm          同时部署润色服务
#   --llm-port N        润色端口（默认 8001）
#   --llm-model ID      润色模型（默认 Qwen/Qwen2.5-7B-Instruct）
#   --gpu MEM           每服务显存上限 GiB（默认 12；>16 按 90%、否则 60% 折算）
#   --gpu-util F        直接指定 gpu-memory-utilization（0.05-0.95，覆盖 --gpu）
#   --asr-args STR      追加传给 vllm serve 的额外参数（引号包裹，如 "--enforce-eager"）
# 示例：
#   scripts/deploy-asr-node02.sh m@node02 --with-llm
#   # 8GB 共享卡（显存仅剩 ~2GB）上的最小部署：
#   scripts/deploy-asr-node02.sh node02 --asr-model openai-mirror/whisper-small \
#       --gpu-util 0.22 --asr-args "--enforce-eager --max-model-len 448"
set -euo pipefail

REMOTE="" ASR_PORT=8000 ASR_MODEL="Qwen/Qwen3-ASR-4B"
WITH_LLM=0 LLM_PORT=8001 LLM_MODEL="Qwen/Qwen2.5-7B-Instruct" GPU=12
GPU_UTIL="" ASR_ARGS="" PIP_INDEX=""

while [ $# -gt 0 ]; do
  case "$1" in
    --asr-port)  ASR_PORT="$2"; shift 2 ;;
    --asr-model) ASR_MODEL="$2"; shift 2 ;;
    --with-llm)  WITH_LLM=1; shift ;;
    --llm-port)  LLM_PORT="$2"; shift 2 ;;
    --llm-model) LLM_MODEL="$2"; shift 2 ;;
    --gpu)       GPU="$2"; shift 2 ;;
    --gpu-util)  GPU_UTIL="$2"; shift 2 ;;
    --asr-args)  ASR_ARGS="$2"; shift 2 ;;
    --pip-index) PIP_INDEX="$2"; shift 2 ;;
    -h|--help)   sed -n '2,26p' "$0"; exit 0 ;;
    *) if [ -z "$REMOTE" ]; then REMOTE="$1"; shift
       else echo "unknown arg: $1" >&2; exit 1; fi ;;
  esac
done
[ -n "$REMOTE" ] || { sed -n '2,26p' "$0" >&2; exit 1; }

# 一个 vllm serve 服务单元。ExecStart/环境变量见 docs/NODE02-DEPLOY.md。
unit() { # unit <name> <port> <model> <extra-args>
  cat <<UNIT
[Unit]
Description=ior voice service: $3 ($2)
After=network-online.target
Wants=network-online.target

[Service]
Type=exec
# No User= : vllm-entrypoint (root) drops to the vllm account via sudo -H.
Environment=VLLM_USE_MODELSCOPE=true
ExecStart=/usr/local/bin/vllm-entrypoint serve "$3" \
  --host 0.0.0.0 --port "$2" $4
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
UNIT
}

run() { printf '\n==> %s\n' "$*"; ssh "$REMOTE" "$*"; }

echo "deploy target: $REMOTE  (asr $ASR_MODEL:$ASR_PORT, llm $([ "$WITH_LLM" = 1 ] && echo "$LLM_MODEL:$LLM_PORT" || echo off))"

# 1) 服务账号 + 特权下降入口。unit 以 root 跑 wrapper，wrapper 用
#    sudo -H -u vllm 落到服务账号（HOME 指向 /home/vllm，模型缓存同址）。
run 'set -e
  id vllm >/dev/null 2>&1 || sudo useradd -r -m -s /usr/sbin/nologin vllm
  sudo tee /usr/local/bin/vllm-entrypoint >/dev/null <<\ENT
#!/bin/sh
# 特权下降入口：解析 vllm 账号家目录里的入口二进制并以该账号执行。
BIN=/home/vllm/.local/bin/vllm
[ -x "$BIN" ] || BIN=$(command -v vllm || echo /usr/bin/vllm)
# sudo env_reset 会剥离 systemd 传入的环境，这里显式转发白名单变量
#（模型源开关；VLLM_API_KEY 供需要鉴权时在 unit 里注入）。
PASS=""
for v in VLLM_USE_MODELSCOPE VLLM_API_KEY; do
  eval "val=\${$v:-}"
  [ -n "$val" ] && PASS="$PASS $v=\"$val\""
done
exec sudo -H -u vllm env $PASS "$BIN" "$@"
ENT
  sudo chmod 755 /usr/local/bin/vllm-entrypoint'

# 2) 依赖：vLLM >= 0.11（audio/transcriptions 路由），装进 vllm 账号的
#    --user 目录（root 的 ~/.local 对服务账号不可见）。pip 对已满足的
#    版本是 no-op，重复执行幂等；国内可追加清华 pypi 源。
# 注意：sudo env_reset 会丢弃环境变量，镜像索引用 --index-url 参数传入。
PIP_FLAGS=""
[ -n "$PIP_INDEX" ] && PIP_FLAGS="--index-url $PIP_INDEX"
# modelscope SDK 是 VLLM_USE_MODELSCOPE 下载路径的运行时依赖（vllm 不自带）。
run "sudo -H -u vllm python3 -m pip install --user 'vllm>=0.11' modelscope $PIP_FLAGS &&
  sudo -H -u vllm /home/vllm/.local/bin/vllm --version 2>/dev/null | head -1"

# 3) 模型预下载到本地目录。vllm 启动时的在线存在性检查会直连
#    huggingface.co（VLLM_USE_MODELSCOPE 也拦不住该路径），HF 不可达时
#    卡在重试；本地目录 serving 完全离线且确定性最高。--served-model-name
#    保持原模型 id，与 ior 侧 llm.asr.model 配置一致。
ASR_DIR="$ASR_MODEL"; ASR_SERVED="$ASR_MODEL"
case "$ASR_MODEL" in /*) ;; *)
  ASR_DIR="/home/vllm/models/${ASR_MODEL##*/}"
  run "sudo -H -u vllm python3 -c 'from modelscope.hub.snapshot_download import snapshot_download as d; d(\"$ASR_MODEL\", local_dir=\"$ASR_DIR\")'"
;; esac
LLM_DIR="$LLM_MODEL"; LLM_SERVED="$LLM_MODEL"
if [ "$WITH_LLM" = 1 ]; then case "$LLM_MODEL" in /*) ;; *)
  LLM_DIR="/home/vllm/models/${LLM_MODEL##*/}"
  run "sudo -H -u vllm python3 -c 'from modelscope.hub.snapshot_download import snapshot_download as d; d(\"$LLM_MODEL\", local_dir=\"$LLM_DIR\")'"
;; esac; fi

# 4) 写 systemd 单元并拉起。
install_unit() { # install_unit <name> <port> <model> <extra>
  local name="$1" port="$2" model="$3" extra="$4"
  printf '\n==> install %s (%s :%s)\n' "$name" "$model" "$port"
  unit "$name" "$port" "$model" "$extra" | ssh "$REMOTE" "sudo tee /etc/systemd/system/$name.service >/dev/null"
  run "sudo systemctl daemon-reload && sudo systemctl enable --now $name.service"
}

UTIL="${GPU_UTIL:-$(awk -v g="$GPU" 'BEGIN{printf "%.2f", (g>16?0.90:0.60)}')}"
install_unit ior-asr "$ASR_PORT" "$ASR_DIR" \
  "--served-model-name $ASR_SERVED --gpu-memory-utilization $UTIL $ASR_ARGS"
[ "$WITH_LLM" = 1 ] && install_unit ior-llm "$LLM_PORT" "$LLM_DIR" \
  "--served-model-name $LLM_SERVED --gpu-memory-utilization 0.35"

# 5) 冒烟：等 /v1/models 就绪（首次要加载权重，这里最多等 15 分钟）。
smoke() { # smoke <port> <label>
  printf '==> waiting for :%s (%s) ...\n' "$1" "$2"
  for _ in $(seq 1 90); do
    if ssh "$REMOTE" "curl -fsS -m 5 http://127.0.0.1:$1/v1/models >/dev/null" 2>/dev/null; then
      echo "    $2 ready on port $1"; return 0
    fi
    sleep 10
  done
  echo "    WARNING: $2 not ready in 15min — check: journalctl -u ior-$2 -e" >&2
  return 1
}
smoke "$ASR_PORT" asr || true
[ "$WITH_LLM" = 1 ] && { smoke "$LLM_PORT" llm || true; }

cat <<'TIP'

部署完成。本机（ior 所在机器）收尾：
  1) /etc/hosts 增加节点名解析（若未配）： <node02-ip>  node02
  2) 冒烟（本机直接测）：
       curl -s http://node02:ASR_PORT/v1/models | head -c 300; echo
  3) ior 侧配置见 docs/NODE02-DEPLOY.md（config.toml 的 [llm.asr]/[llm] 默认即用 8000/8001）。
TIP
