# 2026-11-06 · 实时预览 live 默认开启

- **一句话**：`voice.live` 默认改 `true`——ctrl+grave 一按开始录音、面板
  实时显示 provisional 转写，再按一次停止并对完整录音走引擎链上屏，
  转写中再按取消；与"实时获取语音输入、再按关闭"的预期交互一致。
- **变更点**：仅 `ior-config` 默认值翻转（model.rs `live: true`）+ 文档
  （README 配置样例/能力表、VOICE-VERIFY live 子清单改"默认开启"）+
  voice_settings 默认断言。运行时逻辑零改动：live 路径、partial 节流、
  trailing 30 s 窗口、二按全量转写均为既有实现。
- **不变量影响**：纯本地链（无远端引擎）自动回退纯提示；远端不可达时
  partial 静默失败；`voice.live = false` 恢复旧行为。C ABI 零变化。
- **验证**：`cargo test --workspace` 423 passed；fmt/clippy/行数门禁过；
  addon 重装 + fcitx5 重启后真机生效。
