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

## 四、实际部署与验证结果（2026-11-01，headless）

### node02 服务（真实部署，非桩）

node02 实为 8GB RTX 4060 Ti **共享节点**（llama-server 常驻 5.4GB +
桌面应用 0.5GB，仅剩 ~1.4GB 空闲），跑不动默认的 Qwen3-ASR-4B。按
`docs/NODE02-DEPLOY.md` 的 8GB 配方部署 `openai-mirror/whisper-small`：

```sh
scripts/deploy-asr-node02.sh node02 \
  --asr-model openai-mirror/whisper-small \
  --gpu-util 0.17 \
  --asr-args "--enforce-eager --max-model-len 448 --max-num-seqs 1 --max-num-batched-tokens 1600" \
  --pip-index https://mirrors.aliyun.com/pypi/simple/
```

- vLLM 0.31.0 / `ior-asr.service` active：weights 0.56GiB + KV 0.7GiB，
  恰好落在 0.17×7.6GiB 预算内（日志：`Free memory ... 1.39/7.6 GiB`）。
- 冒烟（本机直连，含 4.2 s 中文 wav）：

  ```
  $ curl http://node02:8000/v1/models
  {"object":"list","data":[{"id":"openai-mirror/whisper-small",...}]}
  $ curl http://node02:8000/v1/audio/transcriptions \
      -F model=openai-mirror/whisper-small -F language=zh \
      -F response_format=text -F file=@/tmp/ior-voice-test.wav
  {"text":"(字幕製作:貝爾)","usage":{"type":"duration","seconds":5}}   # 2.0 s
  ```

  注意两点：vLLM 0.11+ 对 `response_format=text` 仍回 JSON 信封（已让
  `voice::asr` 兼容两种响应，见回归测试 `transcribe_extracts_text_from_
  a_json_envelope`）；whisper-small 对 espeak 合成的机器音会幻觉（识别成
  字幕署名），**管线正确、音源受限**——生产质量按默认配置走
  Qwen3-ASR-4B / 大显存，或真人语音 + whisper-large-v3-turbo。

### 宿主直驱 C ABI（不依赖 fcitx5/桌面）

`target/release/libior_engine.so`（cdylib）+ Python ctypes 按薄壳完全
相同的调用序驱动，配置即 `~/.config/fcitx5/ior/config.toml`
（`capture_command = "cat /tmp/ior-voice-test.wav"` 桩麦克风、真实
node02 端点）：

| 检查 | 结果 |
|------|------|
| 11 个 voice 符号 null-engine 调用不崩 | PASS |
| hotkey 元数据（keysym 0x60 / mods 4 / toggle / `ctrl+\``） | PASS |
| toggle→recording(1)→stop→transcribing(2)→poll→idle(0) | PASS |
| 转写文本经 take_text 取出（JSON 信封正确解出） | PASS |
| take_text 后 poll 保持 0（one-shot 语义由 poll 门控） | PASS |
| cancel：toggle→cancel→idle，poll 不产出 | PASS |

### 待办（需用户配合）

- 第三节十项真机清单：等用户给时间窗执行（期间不重启其 fcitx5）。
- `/usr/lib/.../libior.so` 已更新为含 JSON 信封解析的构建
  （md5 `2c599d2a`），但**运行中的 fcitx5 仍映射旧版**，下次自然重启
  生效；验证清单执行时一并 `fcitx5 --replace`。

## 五、2026-11-03 对接收尾（t2s + node02 修复，headless 全证）

- **node02 修复**：`ior-asr.service`（whisper-small）此前缺 `vllm[audio]`
  extra——`soundfile` ImportError 导致转写空响应且单序列阻塞（`--max-num-seqs 1`
  下事件循环被占满，连 `/v1/models` 都挂起）。已补装 `soundfile`+`librosa`
  并重启；`deploy-asr-node02.sh` 同步改为 `vllm[audio]>=0.11` 根治。
- **音源结论**：延迟与幻觉都由音源质量决定——espeak 机器音 8.9s 且幻觉
  （字幕署名）；piper 神经中文语音（huayan-medium，重采样 16k mono s16）
  转写 12/14 字正确、延迟 0.6s（"散步"→"三步"近音，whisper-small 上限）。
- **whisper-small 繁体输出**：`language=zh` 仍出繁体（`天氣/我們/公園`）。
  新增 `crates/t2s`（dict-tools 的 OpenCC t2s 提炼为共享 crate）接入
  `voice_worker`：转写文本先简体化再上屏/润色。dict-tools 改薄 re-export。
- **宿主直驱复测（已安装 libior.so，真实 node02 端点）**：
  hotkey 0x60/4；toggle→recording(1)→stop→transcribing(2)→**poll 门控**
  →idle(0)；`take_text` = `今天天气很好,我们一起去公园三步吧!`（简体），
  toggle-toggle 第二轮一致。注意探针必须循环调 `voice_poll`，只读
  state 不会迁移（C++ 每 50ms 轮询即此语义）。
- **配置**：`timeout_ms` 8000→30000（幻觉长生成的保护余量）；
  `capture_command` 恢复 `auto`（pw-record 在位）。
- **第三节 GUI 清单**：仍未执行——本日 4 次尝试均被实时桌面使用打断
  （用户键入侵入 zenity、fcitx5 被外部重启、窗口即时关闭）。链路已由
  上条宿主直驱全证，GUI 仅剩 C++ keyEvent 拦截一跳，待用户时间窗。
