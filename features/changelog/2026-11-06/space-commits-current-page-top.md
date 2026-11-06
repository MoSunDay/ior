# 2026-11-06 · 空格上屏当前页首选——翻页后不再选回第一页的词

- **一句话**：空格（及 `enter_action = candidate` 的回车）上屏的"首选"从
  **扁平列表第 0 个**改为**当前显示页的第 1 个候选**——翻页后按空格，
  上屏的是翻过去那页的首词，不再是翻页前的首词。
- **根因**：`session::handle_commit_or_passthrough` 用 `cands.first()`
  取候选，完全忽略 `s.page`；而数字选字 `handle_digit` 一直是
  `s.page * page_size + offset` 的页内寻址，鼠标点选 `select_page_local`
  也按当前页寻址——唯独空格路径漏了页偏移，用户所见与所得不一致。
- **变更点**：
  - engine-core：`handle_commit_or_passthrough` 改为
    `cands.get(s.page * page_size(s)).or_else(|| cands.first())`，
    与数字选字同一寻址；`.or_else(first)` 兜底页索引越界的理论角落
    （公开 API 下翻页会回绕、任何输入变更都置 `page = 0`，正常到不了），
    候选存在时空格必上屏的旧契约保持。无候选时的原码透传分支不动。
- **不变量影响**：ABI 零变化；页 0 行为与旧版完全一致（首选不变），
  行为变化仅在 `page > 0` 时空格/CommitBest 回车上屏当前页首词。
- **验证**：`cargo test -p engine-core`（120 单测 + 集成，含新增回归
  `space_after_paging_commits_current_page_top`、
  `space_on_first_page_commits_top_candidate`）、`cargo test -p fcitx5-ior`
  FFI 全绿；fmt / clippy `-D warnings` / 行数门禁通过。真机部署：
  `cmake --build build && sudo cmake --install` +
  `systemctl --user restart fcitx5`（PID 1042433 active），新库
  /usr/lib/x86_64-linux-gnu/fcitx5/libior.so（inode 26088144）与
  /proc/PID/maps 映射一致，journal 仅 "Loaded addon ior" 无告警；
  `m0_demo` ABI 冒烟：翻页 + 页内点选路径正常。GUI 按键端到端
  （输拼音→翻页→空格）留人工。
- **评审**：单轮自查——空格、CommitBest 回车、Shift 切英文带出三条路径
  同源 `handle_commit_or_passthrough`，无旁路；点选/数字/空格三者页内
  寻址已对齐。
- **遗留**：GUI 按键端到端待人工时间窗执行。另发现（与本修复无关，
  HEAD 复现）：`m0_demo` 尾部 `',' → '，'` 断言在干净 HEAD 也失败，
  系此前标点/Shift 行为变更后示例未跟进，待单独处理。
