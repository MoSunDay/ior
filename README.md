<img src="assets/ior.svg" width="72" align="right" alt="ior logo">

# ior

对标搜狗输入法本地能力的 Rust 输入法：**引擎纯 Rust、fcitx5 薄壳**。
目标是 Linux 下开箱即用的拼音输入（词库、整句、自学习、双拼/模糊音），
二期再叠加本地语音与本地 LLM 润色。

## 架构

```
应用 (GTK / Qt / XIM)
        │
        ▼
    fcitx5  ──加载──►  ior C++ 薄壳 (≤500 行, crates/fcitx5-addon/cpp)
                          │  C ABI (extern "C")
                          ▼
                  ior_engine (Rust cdylib/staticlib)
                  └─ engine-core  纯函数引擎：切分 / 词库 / 解码 / 自学习
                          │
                          ├── voice  语音输入(V1): 热键采集→node02 远端 ASR
                          └── llm    (可选)转写润色: node02 chat 服务
```

## crates 一览

| crate | 职责 |
|-------|------|
| `engine-core` | 纯 Rust 引擎：拼音切分、词库、词级/整句 beam 解码、用户自学习（无 UI、headless 可测） |
| `dict-tools` | 独立词库工具链：TSV / rime `dict.yaml` ↔ `.ior` v2，build/inspect/export/merge/compile/validate/train；`compile --simplify` 用 vendored OpenCC t2s 表输出简体 |
| `ior-config` | 零依赖极简 TOML 子集 + 配置 schema/default/patch/校验（addon 启动加载） |
| `bench` | 自研计时的性能与端到端互锁基准（编译/加载/查询/整句解码） |
| `fcitx5-ior` (fcitx5-addon) | C ABI 桥（Rust，staticlib/cdylib）+ ≤500 行 C++ 薄壳接入 fcitx5 |
| `voice` | 语音输入 V1：热键解析、录音采集（pw-record/parec/arecord）、WAV 封装、零依赖 http/multipart/json、OpenAI 兼容转写客户端（远端 Qwen3-ASR） |
| `llm` | 润色辅助（V1 远端可选）：延迟预算/降级策略/prompt 模板；转写润色经 node02 chat 服务 |

## 快速开始

需要 Rust stable（无任何第三方依赖，零外部 crate）。

```sh
# 构建 / 测试整个工作区
cargo build --workspace
cargo test  --workspace

# 词库工具（示例）
cargo run -p dict-tools -- build  data/dicts/sample-extra.tsv /tmp/extra.ior
cargo run -p dict-tools -- inspect /tmp/extra.ior
cargo run -p dict-tools -- export /tmp/extra.ior /tmp/extra-back.tsv
cargo run -p dict-tools -- merge  /tmp/extra.ior /tmp/extra.ior -o /tmp/merged.ior
cargo run -p dict-tools -- --help

# 全量词库：安装入库的预编译 core.ior / core.gram（--user 装到用户目录）
scripts/install-core-dict.sh

# 二期模型下载计划（本地 onnx/GGUF 清单，V2+ 实现；V1 用远端服务）
scripts/download_models.sh

# node02 语音服务一键部署（ASR 8000 + 可选润色 8001，见 docs/NODE02-DEPLOY.md）
scripts/deploy-asr-node02.sh m@node02 --with-llm
```

`dict-tools` 退出码：`0` 成功，`1` 用法错误，`2` IO 错误，`3` 格式错误。
词库与二进制格式说明见 `data/README.md`。

## fcitx5 addon 构建说明

- 系统依赖（Ubuntu noble 实测）：`libfcitx5core-dev libfcitx5utils-dev
  libfcitx5config-dev cmake ninja-build pkg-config`。
- Rust 侧由 CMake 自动驱动（`cargo build --release -p fcitx5-ior`），产出
  `target/release/libior_engine.a`（staticlib，链入薄壳）与
  `libior_engine.so`（cdylib，供独立宿主复用）。
- 构建 + 系统安装（fcitx5 5.1.7 实测通过，详见
  `crates/fcitx5-addon/cpp/README.md` 与 `docs/M0-FRONTEND-TEST.md`）：

  ```sh
  cd crates/fcitx5-addon/cpp
  cmake -S . -B build -G Ninja -DCMAKE_INSTALL_PREFIX=/usr
  cmake --build build
  sudo cmake --install build        # libior.so → /usr/lib/…/fcitx5, ior.conf → /usr/share/fcitx5/addon
  fcitx5 --replace &                # 或注销重登；然后 fcitx5-remote -s ior 切换
  ```

  注意：`fcitx5-remote -s ior` 只能切到**当前 profile 组内**的输入法——组内
  没有 ior 时该命令静默不生效。需在 `~/.config/fcitx5/profile` 的
  `[Groups/0/Items/*]` 段补一条 `Name=ior` 并重启 fcitx5（`fcitx5-remote -r`
  不重载 profile）。真机回归记录见 `docs/M1-VERIFY.md`。

## 配置

用户配置：`${XDG_CONFIG_HOME:-~/.config}/fcitx5/ior/config.toml`。文件缺失或
非法时回退默认值（warning 输出到 stderr），修改后重启 fcitx5 生效。

```toml
[engine]
double_pinyin = "natural"  # off | natural | xiaohe | mspy | custom
page_size = 5
prev_page_keys = ["Page_Up", "minus"]    # 翻页键可配（rime 风格，仅候选窗激活时生效）
next_page_keys = ["Page_Down", "equal"]
max_candidates = 50
chinese_punct = false       # 中文标点映射（，。）；默认 false = 半角符号
full_width = false          # 全角（１２３ / ａｂｃ / 全角空格）；默认半角，托盘图标与候选窗右侧可即时切换
user_learning = true
enter_action = "raw"       # raw（回车上屏拼音原码）| candidate（同空格上屏首选）；Shift 中英切换走同一逻辑

# 只写想改的项，其余取默认（模糊音默认全关）
#
# Shift 中英切换前置要求：fcitx5 全局默认 [Hotkey/AltTriggerKeys] 0=Shift_L
# 会在引擎之前把 Shift 消费掉切走整个输入法（journal 实证引擎收不到
# Shift）。需在 ~/.config/fcitx5/config 中清空该段（删除 0=Shift_L 行），
# Shift 才归 ior 做中英切换；输入法间切换改用 Ctrl+Space 或托盘。
[engine.fuzzy]
z_zh = true
an_ang = true

# 语音输入 V1：ctrl+` 说话 → node02 Qwen3-ASR 转写上屏（默认即开）
[voice]
enabled = true            # false 时热键完全透传
hotkey = "ctrl+grave"     # 热键可配；hotkey_mode = "toggle" | "hold"
capture_command = "auto"  # auto 探测 pw-record → parec → arecord
max_duration_ms = 60000   # 单次上限，到点自动停并转写

[llm.asr]                 # 转写服务（OpenAI 兼容 /v1/audio/transcriptions）
endpoint = "http://node02:8000/v1/audio/transcriptions"
model = "Qwen/Qwen3-ASR-4B"

[llm]                     # 可选润色（chat 服务），默认关
enabled = false
endpoint = "http://node02:8001/v1/chat/completions"
```

语音细节（部署、安全模型、密钥经环境变量 `IOR_ASR_API_KEY` /
`IOR_LLM_API_KEY` 而非配置文件）见 `docs/NODE02-DEPLOY.md`；验证清单见
`docs/VOICE-VERIFY.md`。

`[switches]` / `[punct]` 已被解析保留，但尚未驱动会话（P4 收尾项）。

## 里程碑

| 阶段 | 内容 |
|------|------|
| **M0 ✅ 完成** | 骨架 + 薄壳 + 词级转换打通，GTK/Qt/XIM 三前端实测通过（2026-10-31，见 `docs/M0-FRONTEND-TEST.md`） |
| **M1 ✅ 完成** | `.ior` v2 + engine-core 加载/多词典叠加；luna + essay preset 经 OpenCC t2s 后 427,661 简体词条 / 9.4 MB，真机验证（`docs/M2-VERIFY.md`） |
| **M2a ✅ 完成** | 查询侧模糊音/简拼 + 自然碼/小鶴/微軟三套内置双拼，`config.toml` 可选 |
| M2b 🟡 词表模型可用 | `.gram` trigram + beam + bench/互锁 ✅；安装脚本训练/安装 `core.gram`，addon 启动加载，整句真机通过；真实语料模型与准确率基线待做 |
| M3 🟡 部分 | 用户词典持久化（append-log+snapshot、时间衰减、TSV 备份）✅ 真机实测；选词即学习即落盘（键盘/点选四路）；候选窗 🗑 删除自造词 ✅；整句自动造词待实现 |
| M3.5 🟡 部分 | `ior-config` TOML/schema/patch + 启动映射 ✅；switches/punct 生效、热重载、配置 UI 待做 |
| M4 🟡 部分 | 分页/标点/Shift 中英/FFI v2 ✅ 真机实测；以词定字/符号面板/per-IC/状态区/联想待做 |
| M4+/M5 | 编译期简体（OpenCC t2s）✅、评测体系（CI 门禁）✅；运行时简繁切换、反查与码表方案为 backlog |
| 二期 V1 ✅ 远端先行 | 按键说话→采集→node02 Qwen3-ASR 转写（+可选润色）：voice crate、FFI v2 增补 10 符号、C++ 热键/轮询/上屏；部署脚本与文档见 `docs/NODE02-DEPLOY.md` |
| 二期 V2~V4 | 语音：V2 本地 VAD+流式 ASR → V3 批量精解码 → V4 模型管理与下载 |

## 数据版权注意

详见 `data/README.md`，要点：

- **搜狗细胞词库（`.scel`/`.ssf`）不可分发**，转换数据不得入库；
- 可用来源：rime `luna_pinyin`（开源）、CC-CEDICT（CC-BY-SA 4.0，需署名
  与同协议共享）、维基语料自建（遵循各自许可）；
- 简体转换表来自 OpenCC ver.1.4.2（Apache-2.0），原样置于 `data/opencc/`，
  来源与 sha256 见该目录 `README.md`；
- 入库词库必须在文件头注明来源与许可证；模型文件（`*.onnx`/`*.gguf`）不入库。

## 仓库规范

- 单文件 ≤ 400 行（迭代中文件 ≤ 800 行），超限按职责拆分模块；
- 纯函数式：普通结构体 + 自由函数，禁止 class/OOP、trait 对象与继承；
- 引擎与工具 crate 零第三方依赖、无 `unsafe`；
- 敏感信息（密钥、Token、密码、连接串）不入库；`rustfmt` 风格统一。
