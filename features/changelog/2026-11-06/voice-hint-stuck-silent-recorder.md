# 2026-11-06 · ctrl+grave 提示滞留修复（卡死录音器 + 面板不重绘）

- **一句话**：`ctrl+grave` 语音提示长期不消失、候选区不可见，根因是
  （1）零字节录音器让 worker 永久阻塞在 `read()`，二按/三按都无法解除；
  （2）worker 失败或空转写折回 idle 时 `pollVoice` 不重绘面板。
- **变更点**：
  - `voice::capture`：`Recorder.child` 改为 `Arc<Mutex<Child>>`；新增
    `start_guarded(argv, deadline, should_kill)`——kill 看门狗线程每 100 ms
    查一次 `should_kill()` / `deadline`，命中即杀子进程（EOF 解除阻塞读），
    `Recorder` 析构（或 `stop`）丢弃 channel 解除武装。`start` 行为不变。
  - `fcitx5-addon::voice_worker::start_recorder`：deadline =
    `max_duration_ms + 5 s` 宽限（健康录音仍由字节上限准点停止），
    `should_kill = signals.stopped()`（stop 或 cancel）。
    `run_event`/`run`/`voice_live::run`/`start_recorder` 签名改传
    `&Arc<WorkerSignals>`（内部 API，C ABI 零变化）。
  - C++ `pollVoice`：`ior_engine_voice_poll` 返回 0 但状态非 0 → 0（worker
    出错/取消折回 idle）时重绘；空转写结束同样重绘，不再等到下一次按键。
- **不变量影响**：健康录音器路径不变（字节上限仍准点触发自动停）；stop 与
  cancel 语义不变，只是现在对零字节录音器也真正生效；`stop` 双重 kill 幂等
  （zombie kill 成功、二次 wait 返回缓存状态，与既有模式一致）。
- **验证**：`cargo test --workspace`（423 passed）；`cargo fmt/clippy`、
  行数脚本通过；addon `cmake --build build` 通过。新增
  `voice_state::tests::stop_unblocks_a_recorder_that_never_writes`（修复前
  死锁，现 0.01 s 通过）与 `capture::tests::guarded_recorder_*`。真机复现
  `capture_command = "sleep 30"` + ctrl+grave 二按，提示应立即清除。
