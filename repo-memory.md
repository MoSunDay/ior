最后验证：2026-11-02 ·（基线：ec3feed @ main + 工作区未提交变更；组件状态按工作区现状记录）

# ior 仓库记忆

## 定位 / 边界
- Rust 拼音输入法：引擎纯 Rust、fcitx5 薄 C++ 壳（README.md 首段架构图）。
- 一期：词库、整句解码、自学习、双拼、模糊音；二期 V1 远端先行：node02 Qwen3-ASR 语音 + 可选 LLM 润色。
- 非目标（当前）：本地 onnx/GGUF 模型（V2+，scripts/download_models.sh 注释）；搜狗 `.scel`/`.ssf` 词库分发（版权禁用，data/README.md）。

## 组件地图（crates/，7 个 workspace 成员，见 Cargo.toml）
| crate | 职责 | 关键源 |
|-------|------|--------|
| engine-core | 纯 Rust 引擎：拼音切分 / 词库 / 词级+整句 beam 解码 / 用户自学习 | src/{segmenter,decoder,beam_search,session,user_dict,punct,width}.rs |
| dict-tools | 词库工具链 CLI：TSV / rime dict.yaml ↔ `.ior` v2；build/inspect/export/merge/compile/validate/train | src/{cli,format,gram,simplify,preset}.rs |
| ior-config | 零依赖 TOML 子集解析 + schema/default/patch | src/{parser,schema,model,patch}.rs |
| fcitx5-addon | C ABI 桥（staticlib+cdylib）+ C++ 薄壳接入 fcitx5 | src/ffi*.rs（4 个）、src/{engine_state,voice_*}.rs、cpp/ioraddon.cpp |
| voice | 语音 V1：热键 / pw-record 采集 / WAV / 零依赖 http·multipart·json / OpenAI 兼容 ASR 客户端 | src/{asr,capture,hotkey,http,json}.rs |
| llm | 润色 V1（node02 chat 远端），薄封装 | src/lib.rs |
| bench | 自研计时基准：编译/加载/查询/整句解码 | src/main.rs |

- 数据：`data/dicts/`（预编译 `core.ior`+`core.gram` 已入库直接加载；`sample-extra.tsv` 冒烟用）、`data/double-pinyin/*.tsv`、`data/opencc/`（vendored t2s，Apache-2.0）。
- 验证文档：`docs/{M0-FRONTEND-TEST,M1-VERIFY,M2-VERIFY,VOICE-VERIFY,NODE02-DEPLOY}.md`。

## 架构与关键不变量（含证据）
- **FFI ABI**：`ior_engine_abi_version()` 版本门禁；共 32 个 `ior_` extern "C" 符号（ffi.rs / ffi_userdict.rs / ffi_voice.rs / ffi_width.rs），改动须 Rust 与 `cpp/ioraddon.cpp` 双端同步；惯例为**纯增补**（不删不改签名）。
- **依赖纪律**：除 fcitx5-addon 与 bench 外全部零第三方依赖（各 Cargo.toml `[dependencies]` 为空，已逐个核对）；fcitx5-addon 仅 path 依赖 engine-core + ior-config。
- **`unsafe` 边界**：只允许在 fcitx5-addon FFI 层；engine-core / dict-tools / ior-config / voice 源码无 unsafe（grep 已核）。
- **纯函数式**：无 class / trait 对象 / 继承（README「仓库规范」）；状态经参数与返回值传递。
- **行数门禁**：`scripts/check_line_limits.sh`——迭代文件 ≤800 行、新增文件 ≤400 行；CI（.github/workflows/ci.yml）强制 fmt + clippy `-D warnings` + test + 行数。注意 `cpp/ioraddon.cpp` 已 559 行（README 宣称 ≤500），新增功能优先放 Rust 侧。
- **用户词典**：`$XDG_DATA_HOME/fcitx5/ior/userdb.tsv`，append-log+快照、30 天半衰；`session::delete_flat` 删除键与 `commit_candidate` 学习键严格镜像（crates/engine-core/src/session.rs、user_dict.rs）。
- **数据版权**：CC-CEDICT 需 CC-BY-SA 4.0 署名；`*.onnx`/`*.gguf` 不入库（.gitignore、data/README.md）。

## 构建与测试
- `cargo build --workspace` / `cargo test --workspace`（与 CI 同款；提交前另跑 fmt/clippy/行数脚本）。
- 基准：`cargo run -p ior-bench --release`（参考性不阻断；P0 基线：10 万词编译 <1s、查询 <10µs）。
- 词库：`cargo run -p dict-tools -- <cmd>`；退出码 0 成功/1 用法/2 IO/3 格式；全量安装 `scripts/install-core-dict.sh`（`--simplify` 编译期转简体）。
- addon 真机：`crates/fcitx5-addon/cpp` 下 `cmake -S . -B build -G Ninja && cmake --build build`（CMake 自动驱动 `cargo build -p fcitx5-ior`）；切换需 profile 组内含 ior（README「fcitx5 addon 构建说明」）。
- 语音部署：`scripts/deploy-asr-node02.sh`（node02：ASR 8000 / LLM 8001，docs/NODE02-DEPLOY.md）。

## 记忆索引
- `features/`：尚无特性记忆（本次为初始化；后续特性工作按需新增）。
- `features/changelog/`：尚无记忆级变更记录。
- `agents/`：未创建（暂无按域拆分需求）。
- 历史事实首要来源是项目自身 `CHANGELOG.md`（Keep a Changelog，条目极详尽）——查历史先读它，再决定是否值得落记忆。
