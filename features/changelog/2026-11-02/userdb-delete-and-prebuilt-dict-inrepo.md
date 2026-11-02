# 2026-11-02 · 候选窗删词 + 全角半角 + 预编译词库入库（提交批次）

- **一句话**：候选窗可删除自造词（学习键严格镜像）；全角/半角独立开关、默认半角标点；预编译 `core.ior`/`core.gram` 入库并移除 rime 原始件。
- **变更点**：
  - engine-core：`user_dict::forget`、`session::delete_flat`+`DeleteOutcome`；`width.rs` 新增（全角只拦字母/数字/空格，不碰标点）；`chinese_punct` 默认 false（engine-core 与 ior-config 同步）；`FLAT_CANDIDATE_LIMIT`/`max_candidates` 10→50。
  - fcitx5-addon：`ffi_userdict.rs`（`ior_engine_candidate_delete`，第 32 个 `ior_` 符号）、`ffi_width.rs`（ABI v2 纯增补）；C++ `ior-delword` 状态区动作（悬停高亮 + 一次性删除模式，stock classicui API）；Shift 按下/释放完全接管（AltTrigger 全局抢键实证）。
  - data：`data/dicts/core.ior`(9.4MB)/`core.gram`(5.9MB) 入库（`.gitignore` 加 `!data/dicts/*` 例外）；删 `luna_pinyin.dict.yaml`、`essay.txt`、`data/double-pinyin/rime/`、转换脚本、`docs/ROADMAP-RIME-PARITY.md`；`install-core-dict.sh` 改纯复制安装。
  - 文档：`data/README.md` 两处"不入库"残留陈述于提交前修正；CHANGELOG 新增六个 Unreleased 小节。
  - 记忆：`repo-memory.md` 初始化（随本提交入库）。
- **不变量影响**：无破坏——FFI 纯增补、依赖/unsafe 纪律不变；新增测试 `userdict_delete`(4)、`candidate_spans`(7)，`paging_punct` 改为显式 `chinese_punct` opt-in。
- **验证**：fmt / build / clippy `-D warnings` / `cargo test --workspace` 365 passed / `check_line_limits.sh` 全绿（基线 ec3feed @ main 之上）。
- **评审**：无独立评审记录，以门禁结果为准。
