# 2026-11-03 · 语音对接收尾——t2s 简体化 + node02 vllm[audio] 修复（提交批次）

- **一句话**：node02 ASR 服务补齐 audio 依赖可真实转写中文；whisper-small 繁体输出经新 `t2s` crate 统一转简体上屏。
- **变更点**：
  - 新 crate `crates/t2s`：dict-tools `simplify.rs` 的表与算法原样提炼（零依赖、纯函数）；dict-tools 改薄 re-export 仅留 `format::Entry` 胶水。
  - `voice_worker`：转写文本先 `simplify_transcript` 再上屏/润色（OnceLock 惰性建表，malformed 表回退原文）。
  - `deploy-asr-node02.sh`：`vllm>=0.11` → `vllm[audio]>=0.11`（soundfile/librosa 缺失曾致空响应 + 单序列阻塞）。
  - 文档：VOICE-VERIFY 第五节回填（含宿主直驱 poll 门控探针要点）。
- **不变量影响**：workspace 7→8 成员；dict-tools 对外 API 不变（pub use）；voice 传输层保持原样（转换在 addon 策略层）。
- **验证**：fmt / clippy -D / test 380 passed / 行数门禁全绿；已装 libior.so 宿主直驱（真实 node02）：两轮 toggle 流程输出简体 `今天天气很好,我们一起去公园三步吧!`；piper 神经音源转写 0.6s。
- **评审**：无独立评审轮（补丁型批次，以门禁+宿主直驱实证为准）。
- **遗留**：GUI 第三节十项清单待用户时间窗（仅剩 C++ keyEvent 拦截一跳未实证）；whisper-small 近音字上限（散步→三步）。
