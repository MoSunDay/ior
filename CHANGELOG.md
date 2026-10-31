# Changelog

本项目所有显著变更记录于此。格式参考 Keep a Changelog，版本遵循 SemVer。

## [0.1.0-m0] - 2026-10-31

M0 里程碑：引擎骨架 + fcitx5 薄壳 + 词级转换全链路打通，GTK/Qt/XIM 三前端
实测通过（Ubuntu noble / X11 / fcitx5 5.1.7）。

### 新增
- `engine-core`：403 音节表、后缀 DP 全切分、模糊音（8 组）、内置 93 词条
  词库、词级解码（整词→用户词→组合）、用户词典自学习、会话状态机
  （字母/空格/数字选词/退格/ESC/非法拼音 ASCII 透传）。
- `fcitx5-ior`：C ABI 桥（9 个 `ior_engine_*` 符号，X11 keysym 映射、
  空指针安全、`ior_engine_abi_version`）+ fcitx5 5.1.7 C++ 薄壳
  （`filterAndAccept` 拦截回传、可点选 `CandidateWord`）+ CMake 安装布局。
- `dict-tools`：TSV ↔ `.ior` 二进制（build/inspect/export/merge）。
- `voice` / `llm`：二期规划面（管线清单、延迟预算与降级策略、prompt 模板）。
- `data/`：样例词库、格式与数据版权说明（搜狗 scel/ssf 不可分发）。
- `docs/M0-FRONTEND-TEST.md`：三前端验收记录与证据截图。

### 修复（实测过程中发现）
- 薄壳 `keyEvent` 仅 `filter()` 不足以阻止按键回传应用，导致「原文+上屏」
  双重投递；改用 `filterAndAccept()`。
- addon 命名/布局：模块需为 `libior.so`（`Library=libior`），元数据装
  `<datadir>/fcitx5/addon`，而非与 .so 同目录。
- 非法拼音空格上屏时剔除用户敲入的 `'` 分隔符。

### 验证
- `cargo test --workspace`：78/78 通过；clippy `-D warnings`、fmt 全绿。
- 三前端端到端：GTK `你好`、Qt `你好`、XIM `你好/先/vk`（选词与透传）。
