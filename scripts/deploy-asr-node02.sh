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
#   --gpu MEM           每服务显存上限 GiB（默认 12）
# 示例：
#   scripts/deploy-asr-node02.sh m@node02 --with-llm
set -euo pipefail

REMOTE="" ASR_PORT=8000 ASR_MODEL="Qwen/Qwen3-ASR-4B"
WITH_LLM=0 LLM_PORT=8001 LLM_MODEL="Qwen/Qwen2.5-7B-Instruct" GPU=12

while [ $# -gt 0 ]; do
  case "$1" in
    --asr-port)  ASR_PORT="$2"; shift 2 ;;
    --asr-model) ASR_MODEL="$2"; shift 2 ;;
    --with-llm)  WITH_LLM=1; shift ;;
    --llm-port)  LLM_PORT="$2"; shift 2 ;;
    --llm-model) LLM_MODEL="$2"; shift 2 ;;
    --gpu)       GPU="$2"; shift 2 ;;
    -h|--help)   sed -n '2,22p' "$0"; exit 0 ;;
    *) if [ -z "$REMOTE" ]; then REMOTE="$1"; shift
       else echo "unknown arg: $1" >&2; exit 1; fi ;;
  esac
done
[ -n "$REMOTE" ] || { sed -n '2,22p' "$0" >&2; exit 1; }

# 一个 vllm serve 服务单元。ExecStart/环境变量见 docs/NODE02-DEPLOY.md。
unit() { # unit <name> <port> <model> <extra-args>
  cat <<UNIT
[Unit]
Description=ior voice service: $3 ($2)
After=network-online.target
Wants=network-online.target

[Service]
Type=exec
User=vllm
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

# 1) 依赖：vLLM >= 0.11（audio/transcriptions 路由）。
run 'set -e
  if command -v vllm >/dev/null 2>&1 && python3 -c "import vllm; from packaging.version import Version; import sys; sys.exit(0 if Version(vllm.__version__) >= Version(\"0.11\") else 1)"; then
    echo "vllm $(vllm --version 2>/dev/null || echo ok) already >= 0.11"
  else
    python3 -m pip install --user "vllm>=0.11"  # 国内可加 -i https://pypi.tuna.tsinghua.edu.cn/simple
  fi'

# 2) 服务账号 + 固定入口（pip --user 装在登录家目录，unit 用绝对路径最稳）。
run 'id vllm >/dev/null 2>&1 || sudo useradd -r -m -s /usr/sbin/nologin vllm
  [ -x /usr/local/bin/vllm-entrypoint ] || sudo tee /usr/local/bin/vllm-entrypoint >/dev/null <<\ENT
#!/bin/sh
# vllm 的 login-shell 包装：让 systemd unit 能找到 pip --user 安装的入口。
exec sudo -u vllm $(sudo -u vllm sh -c "command -v vllm" || echo /usr/bin/vllm) "$@"
ENT
  sudo chmod 755 /usr/local/bin/vllm-entrypoint'

# 3) 写 systemd 单元并拉起（模型首启时经 ModelScope 下载，耗时数分钟）。
install_unit() { # install_unit <name> <port> <model> <extra>
  local name="$1" port="$2" model="$3" extra="$4"
  printf '\n==> install %s (%s :%s)\n' "$name" "$model" "$port"
  unit "$name" "$port" "$model" "$extra" | ssh "$REMOTE" "sudo tee /etc/systemd/system/$name.service >/dev/null"
  run "sudo systemctl daemon-reload && sudo systemctl enable --now $name.service"
}

install_unit ior-asr "$ASR_PORT" "$ASR_MODEL" "--gpu-memory-utilization $(awk -v g="$GPU" 'BEGIN{printf "%.2f", (g>16?0.90:0.60)}')"
[ "$WITH_LLM" = 1 ] && install_unit ior-llm "$LLM_PORT" "$LLM_MODEL" "--gpu-memory-utilization 0.35"

# 4) 冒烟：等 /v1/models 就绪（首次要下载模型权重，这里最多等 15 分钟）。
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
