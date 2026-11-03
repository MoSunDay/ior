# 2026-11-03 · 中英切换等同回车——带出未上屏内容，shift_action 退役

- **一句话**：中文模式带未上屏内容（半截拼音 / 暂存选字）切英文时，不再按 `shift_action`（默认丢弃）处理，而是完全走 Enter 逻辑（`enter_action`）：默认原码上屏，`candidate` 配置时上屏首选并学习；`engine.shift_action` 配置轴整体移除。
- **变更点**：
  - engine-core：`session::process_shift` 切英文分支改调 `handle_enter`（span 暂存汉字仍携带上屏，防 d32ce87 同类回归）；`config::ShiftAction` 枚举/字段/默认值删除。
  - ior-config：model/schema/patch/lib 的 `ShiftAction` 镜像删除；旧配置残留 `shift_action` 键经 unknown-key 警告通道降级，不阻断加载。
  - fcitx5-addon：`engine_state::to_engine_config` 删映射；ffi.rs/ioraddon.h/m0_demo 注释契约更新（"pending preedit committed exactly like Enter"）。
  - 测试改写：paging_punct（默认原码上屏 / candidate 学习 / 空缓冲静默）、sentence_staging（暂存带出两例）、ffi+engine_state 的"丢弃"断言改"上屏"（`shift_keysym_toggles_mode_and_commits_like_enter`、`set_ascii_mode_*`）。
  - 文档：README 配置样例删 `shift_action` 行；CHANGELOG Unreleased 新条目。
- **不变量影响**：ABI 零变化（32 符号、签名不动，仅 commit 内容语义变化）；**行为反转 b6b407d**（防误上屏 → 切换即回车，与搜狗等主流 IME 切英文带出原码一致，用户决策）。
- **验证**：fmt / build / clippy `-D warnings` / `cargo test --workspace` 378 passed / `check_line_limits.sh` 全绿（ffi.rs 801→800 行内收口）。
- **评审**：单轮自查（Shift/托盘/FFI 三路径同源 `process_shift`，无旁路；`set_ascii_mode` 幂等门保留防幽灵上屏）。
- **遗留**：无；GUI 真机（Shift 释放路径 `set_ascii_mode` 序列）与单元同源，未单测端到端。
