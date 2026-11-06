最后验证：2026-11-06 ·（语音波次收尾：SenseVoice 多引擎 + toggle 二按停止转写 + ctrl+grave 提示滞留修复；cargo test --workspace 423 passed）

# ior 仓库记忆

## 定位 / 边界
- Rust 拼音输入法：引擎纯 Rust、fcitx5 薄 C++ 壳（README.md 首段架构图）。
- 一期：词库、整句解码、自学习、双拼、模糊音；二期 V1 远端先行（node02 Qwen3-ASR + 可选 LLM 润色），V1.5 本地可选（SenseVoice-Small 经 shell out `sherpa-onnx-offline`，auto 本地优先/远端兜底）。
- 非目标（当前）：进程内 onnx/GGUF 推理（本地模型一律 shell out 外部 CLI；常驻 GPU 引擎留 V2+）；搜狗 `.scel`/`.ssf` 词库分发（版权禁用，data/README.md）。

## 组件地图（crates/，8 个 workspace 成员，见 Cargo.toml）
| crate | 职责 | 关键源 |
|-------|------|--------|
| engine-core | 纯 Rust 引擎：拼音切分 / 词库 / 词级+整句 beam 解码 / 用户自学习 | src/{segmenter,decoder,beam_search,session,user_dict,punct,width}.rs |
| dict-tools | 词库工具链 CLI：TSV / rime dict.yaml ↔ `.ior` v2；build/inspect/export/merge/compile/validate/train | src/{cli,format,gram,simplify,preset}.rs |
| ior-config | 零依赖 TOML 子集解析 + schema/default/patch | src/{parser,schema,model,patch}.rs |
| fcitx5-addon | C ABI 桥（staticlib+cdylib）+ C++ 薄壳接入 fcitx5 | src/ffi*.rs（4 个）、src/{engine_state,voice_*}.rs、cpp/ioraddon.cpp |
| voice | 语音：热键 / pw-record 采集 / WAV / 本地 SenseVoice（shell-out）/ 多引擎降级链 / 零依赖 http·multipart·json / 远端 OpenAI 兼容 ASR | src/{asr,capture,engine,sensevoice,temp_wav,hotkey,http,json}.rs |
| t2s | 繁→简汉字转换（vendored OpenCC 表，dict-tools 与 fcitx5-addon 语音路径共享） | src/lib.rs |
| llm | 润色 V1（node02 chat 远端），薄封装 | src/lib.rs |
| bench | 自研计时基准：编译/加载/查询/整句解码 | src/main.rs |

- 数据：`data/dicts/`（预编译 `core.ior`+`core.gram` 已入库直接加载；`sample-extra.tsv` 冒烟用）、`data/double-pinyin/*.tsv`、`data/opencc/`（vendored t2s，Apache-2.0）。
- 验证文档：`docs/{M0-FRONTEND-TEST,M1-VERIFY,M2-VERIFY,VOICE-VERIFY,NODE02-DEPLOY,SENSEVOICE-LOCAL}.md`。

## 架构与关键不变量（含证据）
- **FFI ABI**：`ior_engine_abi_version()` 版本门禁；共 32 个 `ior_` extern "C" 符号（ffi.rs / ffi_userdict.rs / ffi_voice.rs / ffi_width.rs），改动须 Rust 与 `cpp/ioraddon.cpp` 双端同步；惯例为**纯增补**（不删不改签名）。`ior_engine_select` 语义：选中 span 候选时返回**空串**且状态 Editing（组句暂存，commit 推迟到整段组合上屏）——C++ 端须守卫空串勿清 preedit。
- **依赖纪律**：除 fcitx5-addon 与 bench 外全部零第三方依赖（各 Cargo.toml `[dependencies]` 为空，已逐个核对）；fcitx5-addon 的 5 个 path 依赖为 engine-core、ior-config、llm、voice、t2s（均 path 依赖）。
- **`unsafe` 边界**：只允许在 fcitx5-addon FFI 层；engine-core / dict-tools / ior-config / voice 源码无 unsafe（grep 已核）。
- **纯函数式**：无 class / trait 对象 / 继承（README「仓库规范」）；状态经参数与返回值传递。
- **行数门禁**：`scripts/check_line_limits.sh`——迭代文件 ≤800 行、新增文件 ≤400 行；CI（.github/workflows/ci.yml）强制 fmt + clippy `-D warnings` + test + 行数。注意 `cpp/ioraddon.cpp` 已 631 行（迭代中文件上限 ≤800，README 已对齐），新增功能优先放 Rust 侧。
- **用户词典**：`$XDG_DATA_HOME/fcitx5/ior/userdb.tsv`，append-log+快照、30 天半衰；`session::delete_flat` 删除键与 `commit_candidate` 学习键严格镜像（crates/engine-core/src/session.rs、user_dict.rs）。
- **语音 ASR 引擎链**：`[voice.asr]`（engine/fallback/command/model_dir/threads/timeout）在 addon 启动时经 `voice::engine::resolve_chain` 冻结为 `Vec<EngineConfig>`；`auto` 探测到本地模型+二进制才把 SenseVoice 放链首，否则仅 remote（现有用户零变化）。`transcribe_best` 失败逐级降级并写 journal；`[llm.asr]` 原样是远端设置，ABI/C++ 零变化。本地模型目录按 `$IOR_MODELS_DIR`（若设）→ `$XDG_DATA_HOME/ior/models/sensevoice-small/` → `$HOME/.local/share/...` 探测（需 `model.int8.onnx`+`tokens.txt`），下载 `scripts/download_models.sh --sensevoice`。取消在采集后、每个引擎前后与润色前检查：取消的语音不会再降级或润色，但已开始的子进程/HTTP 请求仍会跑完。零字节录音器（音频服务卡死/设备被占）由 `capture::start_guarded` 的 kill 看门狗兜底：到 `max_duration_ms`+5s 或收到 stop/cancel 即杀子进程以 EOF 解除阻塞的 `read`（否则 runtime 永卡 Recording/Transcribing、候选区被隐藏）；C++ `pollVoice` 在折回 idle 或空转写时必须重绘面板，否则 `[voice] ...` 状态行滞留。热键默认 toggle：按一次进入语音输入（录音），再按一次停止采集并转写，转写中再按取消；上屏由 max_duration_ms 到点自动停并转写（也可二按提前停）。hold 仍是按住说话、松开转写，转写中按热键也取消。
- **数据版权**：CC-CEDICT 需 CC-BY-SA 4.0 署名；`*.onnx`/`*.gguf` 不入库（.gitignore、data/README.md）。

## 构建与测试
- `cargo build --workspace` / `cargo test --workspace`（与 CI 同款；提交前另跑 fmt/clippy/行数脚本）。
- 基准：`cargo run -p ior-bench --release`（参考性不阻断；P0 基线：10 万词编译 <1s、查询 <10µs）。
- 词库：`cargo run -p dict-tools -- <cmd>`；退出码 0 成功/1 用法/2 IO/3 格式；全量安装 `scripts/install-core-dict.sh`（纯安装入库的预编译 core.ior/core.gram，不再现场编译）。
- addon 真机：`crates/fcitx5-addon/cpp` 下 `cmake -S . -B build -G Ninja && cmake --build build`（CMake 自动驱动 `cargo build -p fcitx5-ior`）；切换需 profile 组内含 ior（README「fcitx5 addon 构建说明」）。
- 语音部署：`scripts/deploy-asr-node02.sh`（node02：ASR 8000 / LLM 8001，docs/NODE02-DEPLOY.md）；本地模型 `scripts/download_models.sh --sensevoice` + `docs/SENSEVOICE-LOCAL.md`（二进制 v1.10.30 钉死、配置矩阵、降级/GPU 验证清单）。

## 记忆索引
- `features/`：暂无按特性拆分的常驻记忆（变更记录在 features/changelog/ 足够）。
- `features/changelog/YYYY-MM-DD/<topic>.md`：提交批次记录（删词+预编译词库、组句暂存、语音 t2s、本地 SenseVoice 多引擎等）。
- `agents/`：未创建（暂无按域拆分需求）。
- 历史事实首要来源是项目自身 `CHANGELOG.md`（Keep a Changelog，条目极详尽）——查历史先读它，再决定是否值得落记忆。
