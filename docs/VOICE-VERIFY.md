# V1 语音输入验证记录与真机清单（2026-11-01）

对象：`ctrl+\``（可配）按键说话 → 本机采集 → node02 Qwen3-ASR 转写 →
（可选）LLM 润色 → 上屏。环境与 M2 批次相同（Ubuntu noble / X11 /
fcitx5 5.1.7）。

## 一、headless 回归（全部通过）

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # 343 passed, 0 failed
./scripts/check_line_limits.sh  # OK
cd crates/fcitx5-addon/cpp && cmake -S . -B build -G Ninja && cmake --build build
nm -D --defined-only build/libior.so | grep -c ior_engine_voice   # = 10
```

覆盖点（按 crate）：

- `ior-config`：`[voice]`/`[llm.asr]`/`[llm]` schema 解析、默认值、
  非法值报错（空字符串、非 http 前缀、越界数值、坏 hotkey_mode）。
- `voice`：
  - `hotkey`：`ctrl+grave`/`ctrl+\``/`ctrl+shift+u`/大小写/裸键 →
    keysym+mods；`KeyStates::toInteger` 位对齐（Shift=1<<0 …）。
  - `capture`：`auto` 探测顺序、`cat` 桩读满 max_bytes、WAV 头写/剥。
  - `http`：分块响应重组、状态码非 2xx 报错、拒绝连接路径。
  - `json`：转义/UTF-16 代理对、`extract_string_field("content")`。
  - `asr`：multipart 边界拼装；本机 `TcpListener` 桩服务端到端
    转写一例（`response_format=text`）。
- `fcitx5-ior`：
  - `voice_state`：状态机 idle→recording→transcribing→idle、
    toggle 二按=cancel、stop≠cancel（停录仍转写）、poll 驱动。
  - `voice_worker`：采集→WAV→ASR→（stub 润色）文本回传。
  - `ffi_voice`：10 个符号的空指针安全性（null engine → 安全默认值）。
  - C++ 薄壳：`handleVoiceHotkey` 掩码比较（NumLock 等无关位忽略）、
    50 ms `addTimeEvent` 轮询、面板提示分支由上面的 Rust 测试 +
    编译期覆盖（shell 逻辑 ≤500 行，不单测）。

## 二、node02 服务侧

已按 `docs/NODE02-DEPLOY.md` 用 `scripts/deploy-asr-node02.sh` 部署
（asr:8000 + `--with-llm` 8001）的机器，先跑通：

```sh
curl -s http://node02:8000/v1/models | head -c 300; echo
curl -s http://node02:8000/v1/audio/transcriptions \
  -F model=Qwen/Qwen3-ASR-4B -F language=zh \
  -F response_format=text -F file=@/tmp/test.wav
```

期望：模型列表含 `Qwen3-ASR-4B`；wav 返回纯文本汉字。

## 三、真机清单（xdotool，GTK 前端）

前置：重装 addon（`cmake --build build && sudo cmake --install build`
+ `fcitx5 --replace &`），目标窗口聚焦后再 `fcitx5-remote -s ior`
（沿用 M2 备忘：仅当前聚焦 IC 生效，需断言 `fcitx5-remote -n` = `ior`）。

| # | 操作 | 期望 |
|---|------|------|
| 1 | `xdotool key --clearmodifiers ctrl+grave`，说话 ≥2 s，再按一次前等待 | 面板显示 `[voice] listening... press ctrl+\` again to cancel`，候选区隐藏 |
| 2 | 说话 2 s 后再按 `ctrl+grave`（toggle 取消） | 面板清空，无文字上屏 |
| 3 | 说话 2 s 后停 3 s（等转写） | 先显示 `[voice] transcribing...`，随后整句汉字上屏（不经候选） |
| 4 | 预编辑中途（如输入 `nihao` 未上屏）按热键说完 | 陈旧拼音被 reset，只上屏语音文本 |
| 5 | 配置 `hotkey = "ctrl+shift+u"` 重启后重复 #3 | 新热键生效；`ctrl+grave` 不再拦截 |
| 6 | 配置 `hotkey_mode = "hold"` 重启：按住说话、松开 | 按住即录，松开即转写上屏 |
| 7 | `voice.enabled = false` 重启后按热键 | 完全透传给应用（打出 `` ` ``） |
| 8 | 拔网线/停 `ior-asr.service` 再说一次 | 面板回 idle，不卡死不崩溃（超时 5 s） |
| 9 | `max_duration_ms = 3000`，一直说 | ~3 s 自动停并转写（到顶保护） |
| 10 | `[llm] enabled = true`（润色开）重复 #3 | 上屏为润色后文本；停 8001 服务则回退原文 |

截图/输出记录贴到本节下方（真机执行后补）。
