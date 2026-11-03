# 2026-11-03 · 语音"关不掉"修复——toggle 热键自动重复 + 非法热键静默回退

- **一句话**：toggle 模式按住热键被 X11 自动重复连发 toggle（取消后随机停在录音），且 `hotkey` 写错/写 `"none"` 会静默复活默认 ctrl+grave——两处叠加即"voice 怎么都关不掉"；已修复并真机部署。
- **根因定位**：fcitx5 KeyEvent 无 repeat 标志（event.h 核对）；`handleVoiceHotkey` toggle 分支无状态守卫（对照 hold 分支注释"ignore the repeat press"只护了 hold）；`settings_from_config` 的 `unwrap_or(fallback)` 是旧行为文档化的回退。真机证据链：journal 无配置告警、config 解析探针 `enabled=false → FFI (0,0)` 通过、用户 config 实为 `enabled=true`——排除配置路径故障，锁定重复键与热键回退。
- **变更点**：
  - cpp：`voiceHotkeyDown_` 按下/释放配对吞重复 press；该键位任何 release（含修饰键不匹配，如先松 Ctrl）一律先复位标志（评审二轮补——否则同焦点内热键被永久吞掉）；`reset()` 兜底清标志（防漏 release 后吞掉下一次真实按键——Shift 跟踪同款暴露面，语音侧主动收敛）。
  - Rust：`settings_from_config` `"none"`/解析失败 → `Hotkey{0,0}` + journal 诊断；`voice_state::hotkey()` 门条件 `enabled && keysym != 0`。
  - 文档：model.rs/ffi_voice.rs 契约、README `[voice]` 样例注明 `"none"`。
- **不变量影响**：ABI 零变化；行为变化两处——(1) 按住热键只生效首按；(2) 非法热键从"回退默认"改为"关闭热键"（原语义在 CHANGELOG 声明反转，用户痛点驱动）。
- **验证**：`cargo test --workspace` 384 passed / fmt / clippy `-D warnings` / 行数门禁全绿；lib `cmake --build && sudo cmake --install` + `systemctl --user start fcitx5`（本机用户级 unit 在位、Restart=on-failure；裸 setsid/-d 会被会话回收不可用），/proc/PID/maps inode 26118184 与磁盘一致确认新库已映射；journal 无 ior 告警。按键序列（按住只切一次/再按取消/Ctrl 先松自愈/none 透传）待人工时间窗执行。
- **评审**：两轮——一轮自查（重复键守卫与 Shift 模式同构）；二轮发现两处未闭环：修饰键先松 release 不复位标志（已修：任何该键位 release 先复位）与真机行为声明失实（已降级为待人工验证）。结论 not ready → P0/P1 落地后复核。
- **遗留**：无运行时开关（托盘 Voice on/off，类似 Full-width 动作）未做，config+重启或 `hotkey="none"` 可关；如需即时开关可后补 FFI `voice_set_enabled` + 状态区动作。
