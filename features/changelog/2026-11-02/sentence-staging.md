# 2026-11-02 · 组句暂存——span 选字入缓冲，整段选完组合上屏（提交批次 d7fc513 + d32ce87）

- **一句话**：span 选字不再逐字上屏：整段选完一次性输出组合词并按组合键学习（`panjie`→选盼、选姐 → 上屏「盼姐」并记 `pan'jie→盼姐`）。
- **变更点**：
  - engine-core（session.rs）：`staged: Vec<(String,String)>` 缓冲；`staged_text/composed_text/composed_key`；`commit_candidate` span 分支改暂存返回 Editing、整覆盖分支提交组合词并按组合键学习；退格撤销暂存；space/Enter/Shift/标点守卫适配；`preedit` 前缀暂存汉字；`delete_flat` 组合键镜像。
  - 修复（d32ce87）：`process_shift` CommitBest 曾被尾部 `reset_composition` 吞掉暂存段静默丢整段——改为提交 `staged_text`；回归用例 `shift_commit_best_flushes_staged_when_best_is_a_span`。
  - fcitx5-addon：ffi.rs select 契约文档（span 暂存返回 `""`）；C++ 端空串守卫（ioraddon.cpp，并发会话在途改动，未随本批提交）。
  - 测试：新增 `sentence_staging.rs`（7 例）；改写 `candidate_spans.rs`/`userdict_delete.rs`。
  - CHANGELOG.md 两个 Unreleased 条目。
- **不变量影响**：ABI 零变化（32 符号）；select 返回值**语义扩充**——span 时返回空串且状态 Editing，C++ 端必须守卫空串勿清 preedit。
- **验证**：fmt / clippy `-D warnings` / `cargo test --workspace` 378 passed 全绿；已安装库 C ABI 探针（`/tmp/staging_probe.cc` 范式：g++ 直链 `-lior`）+ 真机 GUI 端到端（zenity 两轮判别法：`panjie`→5→1 上屏「盼姐」、userdb 记 `pan'jie→盼姐`、重打进第 1 页第 2 位——首位「盘结」频次 6 属正常频次竞争，非缺陷）。
- **评审**：review 技能两轮——第一轮发现 CommitBest 静默丢段（判阻断，d32ce87 修复）；第二轮五问全过判可上线。
- **遗留**：暂存后剩余段候选排序不感知前缀语境；双拼模式退格撤销暂存回退为全拼字母（session.rs:154 注释已声明）。
