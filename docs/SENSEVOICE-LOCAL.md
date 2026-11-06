# 本地 SenseVoice-Small 接入（可选，CPU int8）

ior 的语音输入默认仍是“本地优先、远端兜底”：

```text
[voice.asr] engine = "auto"
  └─ 探测 sherpa-onnx-offline + 本地模型
       ├─ 有 → SenseVoice（本地 CPU）→ 失败且 fallback=true → node02 远端
       └─ 无 → node02 远端（现有行为，零变化）
```

本地引擎通过 **shell out `sherpa-onnx-offline` 子进程**运行，不引入任何
第三方 crate，也不把 onnxruntime 链入 fcitx5 进程。每次上屏加载一次模型，
CPU int8 通常几百毫秒；默认 toggle 热键按一次开始录音、再按一次停止并
转写（转写中再按取消），上屏也可由 `max_duration_ms` 到点自动停并转写；
hold 模式仍是按住说话、松开转写（push-to-talk）。可选 `voice.live` 的录音
期实时预览只用链上第一个**远端**引擎：`sherpa-onnx-offline` 每次调用都要
重载模型，无法承担 partial 节奏；本地引擎继续负责停止后的全量最终转写。
常驻 GPU 服务预留在
`voice::engine::resolve_chain` 的链首槽位，后续加一个枚举臂即可。

## 1. 安装 sherpa-onnx-offline（版本钉死）

验证基线：**v1.10.30**（首个稳定支持 SenseVoice 的版本之一）。升级时请
保持版本固定并重新跑本页验证清单。

```sh
SHERPA_VERSION=v1.10.30
curl -fL -o /tmp/sherpa-onnx.tar.bz2 \
  "https://github.com/k2-fsa/sherpa-onnx/releases/download/${SHERPA_VERSION}/sherpa-onnx-${SHERPA_VERSION}-linux-x64-static.tar.bz2"
mkdir -p "$HOME/.local/opt"
tar -xjf /tmp/sherpa-onnx.tar.bz2 -C "$HOME/.local/opt"
mkdir -p "$HOME/.local/bin"
ln -sf "$HOME/.local/opt/sherpa-onnx-${SHERPA_VERSION}-linux-x64-static/bin/sherpa-onnx-offline" \
  "$HOME/.local/bin/sherpa-onnx-offline"
sherpa-onnx-offline --help
```

若 `sherpa-onnx-offline: command not found`，确认 `$HOME/.local/bin` 在
`PATH` 中（`echo "$PATH"`）。发行版仓库若提供同名且含 SenseVoice 的包，
也可以使用，但需自行保证版本与参数兼容。

## 2. 下载模型

```sh
scripts/download_models.sh --sensevoice
```

脚本按顺序尝试 GitHub Releases 与 ModelScope，解压后放到：

```text
${XDG_DATA_HOME:-$HOME/.local/share}/ior/models/sensevoice-small/
  model.int8.onnx
  tokens.txt
```

可用 `IOR_MODELS_DIR` 覆盖模型根目录（下载脚本与 `auto` 探测都遵循），或用 `IOR_SENSEVOICE_URL` 指定镜像。
两个文件缺一不可；`voice.asr.sensevoice_model_dir` 显式指向的目录也必须
同时包含它们。`data/models/` 与 `*.onnx` 已在 `.gitignore` 中，不会入库。

## 3. 配置

默认值已经启用 `auto`，无需改配置即可在装好模型后本地优先：

```toml
[voice]
enabled = true
hotkey = "ctrl+grave"
hotkey_mode = "toggle"  # toggle=再按停止并转写；hold=按住说话
capture_command = "auto"
live = false             # 录音期实时预览（partials 只用远端引擎）
live_interval_ms = 2000  # 500..=10000 ms

[voice.asr]
engine = "auto"                  # auto | sensevoice | remote
fallback = true                  # 首选引擎失败后尝试下一个
sensevoice_command = "auto"      # auto 探测 PATH；也可写显式 argv（空格分隔）
sensevoice_model_dir = "auto"    # auto 探测 XDG/HOME 数据目录
sensevoice_threads = 2           # CPU int8 线程数
timeout_ms = 20000               # 单次本地转写超时

[llm.asr]                        # 远端引擎设置（保持向后兼容）
endpoint = "http://node02:8000/v1/audio/transcriptions"
model = "Qwen/Qwen3-ASR-4B"
```

行为矩阵：

| engine | 本地可用 | fallback | 实际链 |
|--------|----------|----------|--------|
| `auto` | 是 | 任意 | `sensevoice -> remote`（fallback=false 时仅本地） |
| `auto` | 否 | 任意 | `remote`（现有用户行为） |
| `sensevoice` | 是 | 是/否 | 本地；是时再挂远端 |
| `sensevoice` | 否 | 是 | `remote` |
| `sensevoice` | 否 | 否 | 空链，启动 journal 报错 |
| `remote` | 任意 | 任意 | 仅 `remote` |

启动时 journal（stderr）会打印引擎链一行；若 `auto`/`sensevoice` 未探测到本地，再补一行提示，例如：

```text
ior: asr engines: sensevoice -> remote
ior: asr local SenseVoice unavailable; install sherpa-onnx-offline and run ...
```

运行时降级会逐条打印：

```text
ior: asr engine sensevoice: sensevoice exit 1: ...; fallback -> remote
```

全部引擎失败时 worker 返回 `all ASR engines failed: ...`，运行状态回到 idle。

若 `sensevoice_command` 指向包装脚本，脚本必须用
`exec sherpa-onnx-offline "$@"` 启动真实进程。超时监控只会 kill 直接子
进程；未 `exec` 的孙进程可能继续持有 stdout/stderr，但超时路径不再等待
排空线程（读取线程被分离，管道关闭后自行退出）。正常退出路径仍会等待 stdout/stderr
排空，因此包装脚本仍应 `exec`。

## 4. 验证清单

### 4.1 纯函数与子进程桩（无需模型）

```sh
cargo test -p voice
cargo test -p fcitx5-ior voice_
cargo clippy --workspace --all-targets -- -D warnings
scripts/check_line_limits.sh
```

单测覆盖：模型目录探测、命令解析、argv byte-exact、JSON/`Text:` 输出解析、
超时 kill、引擎链解析（本地优先/纯远端/无本地且禁用 fallback）、
`transcribe_best` 首引擎失败后远端成功并产生降级 journal，以及 worker 用
`cat` 采集 + `sh -c "echo 你好"` 本地桩验证 worker 返回值（C++ 上屏路径
不在单测内）。

### 4.2 手工桩命令（不装真模型）

```sh
STUB=/tmp/ior-sensevoice-stub
mkdir -p "$STUB/model"
touch "$STUB/model/model.int8.onnx" "$STUB/model/tokens.txt"
cat > "$STUB/fake-asr" <<'SH'
#!/bin/sh
echo 你好，本地桩引擎
SH
chmod +x "$STUB/fake-asr"
```

把 `[voice.asr]` 设为：

```toml
engine = "sensevoice"
fallback = false
sensevoice_command = "/tmp/ior-sensevoice-stub/fake-asr"
sensevoice_model_dir = "/tmp/ior-sensevoice-stub/model"
```

重启 fcitx5 后按一次 `ctrl+grave` 开始说话（把 `max_duration_ms` 调到 3000，
到点自动停并转写），应上屏 `你好，本地桩引擎`（t2s 与上屏链路不变）。journal 应出现
`ior: asr engines: sensevoice`。

### 4.3 真实语音

1. 装好 `sherpa-onnx-offline` + 模型，保持 `engine = "auto"`。
2. 重启 fcitx5，确认 journal 为 `asr engines: sensevoice -> remote`。
3. 按一次 `ctrl+grave` 说普通话/粤语/英语短句，再按一次停止并转写（或等
   `max_duration_ms` 到点自动停；hold 模式松开即转写），确认简体中文上屏；
   转写中再按一次热键可取消。
4. 观察延迟：CPU int8 每次加载模型，短句通常几百毫秒；若明显超时，
   调大 `timeout_ms` 或减少 `sensevoice_threads` 竞争。

### 4.4 断引擎验降级

任选一种方式让本地引擎失败：

- 临时把 `sensevoice_command` 指到不存在的命令；或
- 用 `chmod -x` 去掉包装脚本执行位；或
- 显式 `engine = "sensevoice"` 且把 `sensevoice_model_dir` 指向空目录、
  保持 `fallback = true`。

预期：journal 出现 `local SenseVoice unavailable` 或运行期
`asr engine sensevoice: ...; fallback -> remote`，随后远端成功上屏。
若把 `fallback = false`，则应报错并回到 idle，不应偷偷请求远端。

### 4.5 GPU 槽位路线图

`voice::engine::detect_gpu()` 目前只探测 `/dev/nvidia*`、
`/proc/driver/nvidia/version` 与 `PATH` 中的 `nvidia-smi`，命中时启动
journal 提示一行。GPU 引擎落地时：

1. 在 `EngineConfig` 增加 GPU 变体；
2. 在 `resolve_chain` 的 `auto`/`sensevoice` 分支把 GPU 放在
   `SenseVoice` 之前；
3. worker 的 `transcribe_best` 加一个 match 臂；
4. ABI 与 C++ 薄壳零变化（引擎链在 settings 冻结时完成）。

## 5. 已知限制 / 安全

- 本地模型与二进制均为可选外部资产，不随仓库分发；模型路径与命令来自
  用户配置，等同于 `capture_command` 的信任级别。
- 每次调用写一个临时 WAV 到系统临时目录：`create_new` 拒绝复用已有文件/
  符号链接，Unix 下权限固定为 `0600`，退出后 best-effort 删除。
- 本地链不做 VAD / 原生流式识别；长音频受 `voice.max_duration_ms` 与
  `voice.asr.timeout_ms` 双重约束。`voice.live` 是远端 trailing-window
  重复整段转写合成的预览，不调用本地 SenseVoice，也不改变最终链路。
- `sherpa-onnx-offline` 的 stdout 目前按“最后非空行，JSON `text` 或
  `Text:` 前缀”解析；升级二进制后若输出格式变化，请先跑
  `cargo test -p voice sensevoice` 并更新 `parse_transcript`。
- 不包含任何 API Key / Token；远端密钥仍只从 `IOR_ASR_API_KEY` 环境变量读取。
- 取消会在采集后、每个引擎前后与润色前检查；已开始的本地子进程或远端
  HTTP 请求不会即时中断，但取消后不会再降级到另一引擎或发起润色上传。
