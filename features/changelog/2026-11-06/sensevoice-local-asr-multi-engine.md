# 2026-11-06 · 本地 SenseVoice-Small 接入 + ASR 多引擎链

- **一句话**：voice 在远端 Qwen3-ASR 之外新增可选本地 SenseVoice-Small（shell out `sherpa-onnx-offline`，零 crate），并以 `auto` 链实现“有本地用本地、失败降级远端”，未装模型用户行为不变。
- **变更点**：
  - `voice::sensevoice`：模型目录/命令探测、byte-exact argv、JSON/`Text:` 输出解析、临时 WAV（Unix `0600` + `create_new` 拒绝复用/符号链接）、线程排空 stdout/stderr、`try_wait` 轮询超时 + kill（超时路径不等待持有管道的孙进程）。
  - `voice::engine`：`EnginePolicy`/`EngineConfig` 纯数据枚举；`resolve_chain` 按 `auto|sensevoice|remote` + `fallback` 冻结链；`transcribe_best` 逐引擎降级并返回 journal，新增取消感知的 `transcribe_best_with_cancel`（每个引擎前后检查，取消后不再降级）；`detect_gpu` 预留链首 GPU 槽位（当前仅提示）。
  - `ior-config`：`Voice.asr: VoiceAsr` 六字段（engine/fallback/command/model_dir/threads/timeout），`[voice.asr]` 嵌套解析、range/unknown-key 校验；`to_table` 回写。`[llm.asr]` 原样保留；两个 endpoint（`[llm.asr]`/`[llm]`）仅接受 http（https 在加载期拒绝，与零 TLS 的 HTTP 客户端一致）。
  - addon：`VoiceSettings.asr` 由单个 `AsrSettings` 改为 `Vec<EngineConfig>`；启动探测并打印 `asr engines: ...`；worker 改走 `transcribe_best_with_cancel`（每个引擎前后与润色前检查取消），降级行逐条 eprintln。FFI/C++ 零变化。
  - `scripts/download_models.sh --sensevoice` 双源真下载并解压到 XDG 数据目录；新增 `docs/SENSEVOICE-LOCAL.md`；README/CHANGELOG 同步。
- **不变量影响**：voice crate 仍零第三方依赖；workspace 8 成员不变；语音 ABI 不变；远端 endpoint/密钥/t2s/润色路径不变；默认无模型时 `auto` 打印引擎链一行，再补一行本地不可用提示（至多两行 journal）。
- **验证**：`cargo fmt`、`cargo build --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`scripts/check_line_limits.sh` 全绿。单测覆盖探测/argv/解析/超时/链解析/降级 journal/取消传播/https endpoint 拒绝/worker 桩 e2e；真实模型语音与断引擎降级按文档清单待人工执行。
- **评审**：本批经独立评审，已按评审意见修复取消传播、https 拒绝与临时 WAV/文档一致性；真实模型与目标机检查仍待执行。
- **遗留**：取消只在采集后、每个引擎前后与润色前检查，已开始的本地子进程或 HTTP 请求不会即时中断；常驻 GPU 引擎；真实 `sherpa-onnx-offline` 输出格式需在目标机复跑 `cargo test -p voice sensevoice`；模型下载 URL 若上游变更可用 `IOR_SENSEVOICE_URL` 覆盖。
