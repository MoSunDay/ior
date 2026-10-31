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

- 系统依赖：`libfcitx5-core-dev`、`cmake`、`ninja`
  （**本机当前未安装 dev 包**，故 C++ 侧暂不在 CI/默认构建路径中）。
- Rust 侧：`cargo build -p fcitx5-ior --release` 产出
  `target/release/libior_engine.a`（staticlib）与
  `target/release/libior_engine.so`（cdylib），C++ 薄壳通过 C ABI 调用。
- CMake 构建指引见 `crates/fcitx5-addon/cpp/README.md`。

## 里程碑

| 阶段 | 内容 |
|------|------|
| **M0（当前）** | 骨架 + 薄壳 + 词级转换打通；dict-tools/voice/llm 规划面 |
| M1 | 核心词库 10 万级（core.tsv → engine-core Dictionary） |
| M2 | 整句 trigram + beam 解码、双拼、模糊音 |
| M3 | 用户自学习（词频调序、造词、云备份接口） |
| M4 | U/V 模式、标点配对、皮肤/配置 UI |
| M5 | 评测体系（转换准确率、延迟、内存基准） |
| 二期 V1~V4 | 语音：V1 按键说话+VAD → V2 流式 ASR → V3 批量精解码 → V4 模型管理与下载 |

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
