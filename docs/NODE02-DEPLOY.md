# node02 语音服务部署（V1：远端 Qwen ASR + 可选润色）

ior V1 的语音输入走**远端推理**：本机只负责按键采集（`pw-record` /
`parec` / `arecord` → 16 kHz s16le mono），node02 上跑 vLLM 的
OpenAI 兼容服务完成转写（与可选的 LLM 润色）。模型不进本仓库、
不进本机（`scripts/download_models.sh` 的本地 onnx/GGUF 清单是 V2+
的规划）。

## 拓扑与端口

| 服务 | 端口 | 模型（默认） | 用途 |
|------|------|--------------|------|
| `ior-asr.service` | 8000 | `Qwen/Qwen3-ASR-4B` | `POST /v1/audio/transcriptions`（multipart，`response_format=text`） |
| `ior-llm.service` | 8001 | `Qwen/Qwen2.5-7B-Instruct` | `POST /v1/chat/completions`（转写润色，可选） |

要求：Python ≥ 3.10、CUDA GPU（4B ASR + 7B 润色同卡建议 ≥ 24 GiB；
单 ASR 12 GiB 即可）、内网互通。

## 一键部署

```sh
# 本机执行；REMOTE 形如 m@node02 或 m@192.168.x.x
scripts/deploy-asr-node02.sh m@node02 --with-llm
# 常用旋钮：--asr-port/--llm-port/--asr-model/--llm-model/--gpu
```

脚本做四件事：装 vLLM ≥ 0.11（transcriptions 路由所需）、建 `vllm`
服务账号与入口包装、写并启动 systemd 单元（模型经
`VLLM_USE_MODELSCOPE=true` 从 ModelScope 下载，无需外网）、轮询
`/v1/models` 冒烟。手动部署（等价步骤）见单元文件内容与下文 curl。

## 本机收尾

1. **名字解析**：`ior` 配置里默认用主机名 `node02`，本机
   `/etc/hosts` 需有一条（或把配置里的 endpoint 写成 IP）：

   ```sh
   echo '192.168.x.x  node02' | sudo tee -a /etc/hosts
   ```

2. **冒烟**（本机执行）：

   ```sh
   curl -s http://node02:8000/v1/models | head -c 300; echo
   # 真实转写（任意 16k mono wav；服务端自动重采样亦可）
   curl -s http://node02:8000/v1/audio/transcriptions \
     -F model=Qwen/Qwen3-ASR-4B -F language=zh \
     -F response_format=text -F file=@/tmp/test.wav
   ```

3. **ior 配置**（默认值即开即用，改端点才需要写）：

   ```toml
   [voice]
   enabled = true          # 总开关（false 时热键失效）
   hotkey = "ctrl+grave"   # 也可写 "ctrl+`"；ctrl+shift+u 之类同理
   hotkey_mode = "toggle"  # toggle=再按取消 | hold=按住说话
   capture_command = "auto"  # auto=探测 pw-record→parec→arecord
   sample_rate = 16000
   max_duration_ms = 60000  # 单次上限，到点自动停并转写

   [llm.asr]               # 转写服务
   endpoint = "http://node02:8000/v1/audio/transcriptions"
   model = "Qwen/Qwen3-ASR-4B"
   language = "zh"
   timeout_ms = 5000

   [llm]                   # 润色（默认关）
   enabled = false
   endpoint = "http://node02:8001/v1/chat/completions"
   model = "Qwen/Qwen2.5-7B-Instruct"
   ```

## 安全模型（为什么只有 http）

- **仅内网**：vLLM 只绑内网网卡时保持明文 `http://`；`ior` 的
  endpoint 校验**只接受 `http://` 前缀**（自签证书在内网收益为零，
  强上 TLS 只会让配置更脆）。请勿把 8000/8001 暴露到公网。
- **密钥永不入库**：配置文件只写**环境变量名**
  （`llm.asr.api_key_env = "IOR_ASR_API_KEY"`、
  `llm.api_key_env = "IOR_LLM_API_KEY"`）。设了变量才发
  `Authorization: Bearer`，没设就不带。给服务端加鉴权时：

  ```sh
  # node02 的 unit 里加（vLLM 原生支持）：
  #   Environment=VLLM_API_KEY=<服务端口令>   # 别提交进任何仓库
  # 本机：export IOR_ASR_API_KEY=...（建议放 ~/.profile 或 systemd --user 环境）
  ```

## 8GB 共享卡实测配方（2026-11-01 node02）

node02 实际是 8GB RTX 4060 Ti 且被 llama-server 等常驻占用 ~6GB，
默认的 Qwen3-ASR-4B（bf16 ~8GB）跑不动。小模型替代配方（已验证）：

```sh
scripts/deploy-asr-node02.sh node02 \
  --asr-model openai-mirror/whisper-small \
  --gpu-util 0.17 \
  --asr-args "--enforce-eager --max-model-len 448 --max-num-seqs 1 --max-num-batched-tokens 1600" \
  --pip-index https://mirrors.aliyun.com/pypi/simple/
```

实测要点（脚本已内置前两条）：

- **pip 直连 pypi 仅 ~20 KB/s**，必须 `--pip-index` 国内源；vLLM 全家
  ~7.9GB，实测 ~1h。ModelScope 权重下载反而有 15+ MB/s。
- **模型预下载到本地目录再 serve**：vLLM 启动时的在线存在性检查会
  直连 huggingface.co（`VLLM_USE_MODELSCOPE` 拦不住该路径），HF 不可
  达时会卡重试循环；脚本先用 `modelscope snapshot_download` 落到
  `/home/vllm/models/<name>` 并以本地路径 + `--served-model-name <id>`
  启动（模型名仍与 ior 配置一致）。
- **whisper 系列需 `--max-num-batched-tokens ≥ 1500`**（30 s 音频的
  token 数），否则启动报错。
- `--gpu-util` 按**空闲**显存折算（vLLM 校验的是 free/total），剩余
  1.4GB 时 0.17 可行（weights 0.56 + KV 0.7 GiB）。
- vLLM 0.11+ 对 `response_format=text` 仍返回 `{"text":...}` JSON
  信封——`voice::asr` 已兼容（plain text 与 JSON 均解）。
- 润色可**复用**节点上已有的 llama-server（如 8081 口的
  Qwen3.5-9B）：把 `[llm] endpoint` 指过去即可，零新增显存；注意
  thinking 模型润色延迟偏高，`docs/VOICE-VERIFY.md` 清单第 10 项。

## 故障排查

| 症状 | 排查 |
|------|------|
| 按热键无反应 | `voice.enabled`、热键拼写（stderr 会提示回退 ctrl+grave）、本机是否有录音命令（`command -v pw-record parec arecord`） |
| 面板停在 transcribing 后无结果 | `journalctl -u ior-asr -e`（node02）；本机 `IOR_ASR_API_KEY` 是否被服务端要求 |
| 转写慢/超时 | `llm.asr.timeout_ms`（默认 5 s）；node02 GPU 是否被 LLM 服务挤占 |
| 名字不通 | `curl http://node02:8000/v1/models`；`/etc/hosts` |
