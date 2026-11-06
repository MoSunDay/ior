# 2026-11-06 · toggle 热键二按停止并转写

- **一句话**：默认 `hotkey_mode = "toggle"` 下，`ctrl+grave` 按一次开始录音，
  再按一次停止采集并转写；转写完成自动上屏，转写中再按一次取消。`hold`
  仍是按住说话、松开停止并转写。
- **变更点**：
  - `voice_state::toggle`：`STATE_RECORDING if hold_mode => phase`（hold
    录音中的按键 no-op，松开才停）；`STATE_RECORDING => stop_and_transcribe`；
    `STATE_TRANSCRIBING => cancel`。`stop_and_transcribe` 同时服务 toggle
    二按与 hold 松开，`stop`/`cancel` 仍分离。
  - C++ `handleVoiceHotkey`：toggle 录音中再按走 `ior_engine_voice_toggle`
    （内部停止并转写）；toggle/hold 转写中按热键走 `cancel`；hold 录音中
    按键忽略，松开调 `stop`。面板提示 toggle 录音中为
    `again to stop and transcribe`，hold 录音中为
    `release to stop and transcribe`，转写中为
    `transcribing... press … to cancel`。
- **不变量影响**：C ABI 零变化；`stop` 与 `cancel` 语义不变；toggle 模式
  到 `max_duration_ms` 仍会自动停并转写；上屏路径不变。
- **验证**：`cargo test -p fcitx5-ior voice_`。慢速采集桩
  （每次 3.2 KB + sleep 0.1 s，`max_duration_ms = 60_000`）让
  `toggle_second_press_stops_capture_and_transcribes` 断言第二按前 ASR
  桩未收到请求、第二按后进入 `TRANSCRIBING` 并收到请求，转写结果上屏；
  `toggle_while_transcribing_cancels_without_late_text` 断言第三按取消、
  迟到文本不上屏；`hold_toggle_while_recording_is_a_noop` 覆盖 hold
  录音按键 no-op。目标机按键序列待人工验证。
