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
                          ├──(二期) voice  语音输入: VAD + 流式/批量 ASR
                          └──(二期) llm    本地 LLM: 标点 / 口语顺滑 / 纠错
```

## crates 一览

| crate | 职责 |
|-------|------|
| `engine-core` | 纯 Rust 引擎：拼音切分、词库、Viterbi/beam 解码、用户自学习（无 UI、headless 可测） |
| `dict-tools` | 独立词库工具链：TSV ↔ `.ior` 二进制，build/inspect/export/merge，不依赖 engine-core |
| `fcitx5-ior` (fcitx5-addon) | C ABI 桥（Rust，staticlib/cdylib）+ ≤500 行 C++ 薄壳接入 fcitx5 |
| `voice` | 二期占位：语音管线规划（capture/VAD/流式 ASR/批量 ASR）、模型清单 |
| `llm` | 二期占位：本地 LLM 规划（延迟预算、降级策略、prompt 模板、模型分档） |

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

# 二期模型下载计划（M0 仅打印清单，无网络请求）
scripts/download_models.sh
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

## 里程碑

| 阶段 | 内容 |
|------|------|
| **M0 ✅ 完成** | 骨架 + 薄壳 + 词级转换打通，GTK/Qt/XIM 三前端实测通过（2026-10-31，见 `docs/M0-FRONTEND-TEST.md`） |
| M1 (=P0) 🟡 部分 | 词库规模：`.ior` v2 二进制 + engine-core 加载/多词典叠加 ✅；10 万级数据导入待网络 |
| M2a (=P1) 🟡 部分 | 拼式管线数据化：运行时模糊音、简拼、双拼框架 ✅；真实键位表待数据导入 |
| M2b (=P2) | 整句 trigram + beam 解码、`.gram` 语言模型 |
| M3 (=P3) 🟡 部分 | 用户词典持久化（append-log+snapshot、时间衰减、TSV 备份）✅ 真机实测；自动造词待 P2 |
| M3.5 (=P4) | 配置/方案体系：极简 TOML、switches、热部署、user patch |
| M4 (=P5) 🟡 部分 | 分页/标点/Shift 中英/FFI v2 ✅ 真机实测；以词定字/符号面板/per-IC/状态区/联想待做 |
| M4+/M5 (=P6) | 简繁、评测体系（CI 门禁已落地）；反查与码表方案明确降级 backlog |
| 二期 V1~V4 | 语音：V1 按键说话+VAD → V2 流式 ASR → V3 批量精解码 → V4 模型管理与下载 |

完整差距对照与依赖图见 `docs/ROADMAP-RIME-PARITY.md`。

## 数据版权注意

详见 `data/README.md`，要点：

- **搜狗细胞词库（`.scel`/`.ssf`）不可分发**，转换数据不得入库；
- 可用来源：rime `luna_pinyin`（开源）、CC-CEDICT（CC-BY-SA 4.0，需署名
  与同协议共享）、维基语料自建（遵循各自许可）；
- 入库词库必须在文件头注明来源与许可证；模型文件（`*.onnx`/`*.gguf`）不入库。

## 仓库规范

- 单文件 ≤ 400 行（迭代中文件 ≤ 800 行），超限按职责拆分模块；
- 纯函数式：普通结构体 + 自由函数，禁止 class/OOP、trait 对象与继承；
- 引擎与工具 crate 零第三方依赖、无 `unsafe`；
- 敏感信息（密钥、Token、密码、连接串）不入库；`rustfmt` 风格统一。
