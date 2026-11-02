# M1 批次真机回归记录（2026-11-01）

环境：Ubuntu noble / X11（DISPLAY=:0）/ fcitx5 5.1.7 / GTK 前端（mousepad，
`GTK_IM_MODULE=fcitx`）/ xdotool 合成键序 / scrot 截图 / userdb 落盘与
文本缓冲区双判据。本批验证对象：P0/P5-lite/P3 已提交代码（含 b56423c 内的 saveUserdb
四路落盘修复与 P1 拼式管线）。

## 安装与激活

```sh
cmake -S crates/fcitx5-addon/cpp -B build -G Ninja -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build && sudo cmake --install build
# fcitx5 只能切换到当前 profile 组内的 IM：编辑 ~/.config/fcitx5/profile，
# 在 [Groups/0/Items/*] 增加 Name=ior 并重启 fcitx5（-r 不重载 profile）。
fcitx5-remote -s ior   # 聚焦窗口后切换
```

## 用例与证据

最终文本缓冲区（/tmp/ior-pad3.txt，经 ctrl+s 落盘读回）：`先，“”abc今天`

| 用例 | 键序 | 期望 | 实测 | 判据 |
|------|------|------|------|------|
| 全拼候选+首选 | `nihao␣` | 你好 | ✅ | 缓冲区 + userdb `ni'hao 你好` |
| 翻页+页内数字选词 | `xian` → PageDown → PageUp → `2` | 先 | ✅ | 缓冲区（第二页语义不串位） |
| 中文标点 | `,` | ， | ✅ | 缓冲区 |
| 配对引号交替 | `"` `"` | “ ” | ✅ | 缓冲区（开→闭） |
| Shift 切英文 | Shift 单击 → `abc` | abc 原样透传 | ✅ | 缓冲区 |
| Shift 切回中文 | Shift 单击 → `jintian␣` | 今天 | ✅ | 缓冲区 + userdb `jin'tian 今天` |
| userdb 键盘提交落盘 | 每次提交 | 快照写盘 | ✅ | userdb.tsv 内容 |
| userdb 数字/点选路径落盘 | `2` 选词 | 快照写盘 | ✅ | `xian 先` 入库 |
| userdb 跨重启持久化 | 换进程重测 | 频次保留 | ✅ | `ni'hao 你好 2`（上一进程遗留） |

截图证据：`docs/m1-candidates-page1.png`、`m1-candidates-page2.png`（翻页
前后）、`m1-gtk-candidates.png`（GTK 候选窗）。

## 过程中发现并解决的问题

1. **saveUserdb 单点落盘**（评审发现）：仅 Shift 路径保存；补齐键盘提交/
   候选点选/析构兜底三路后真机复测通过。
2. **`fcitx5-remote -s ior` 静默不生效**：ior 不在 profile 组时无法切换且
   返回成功码；已在上文激活步骤中注明，可考虑后续在 README 提示。
3. 环境噪音：测试沙箱会在命令调用边界清理守护进程，fcitx5 必须与键序
   同一调用内启动（曾导致一轮「击键全部消失」的假阳性，非代码缺陷）。

## 结论

P0/P5-lite/P3 批次（含 ABI v2 18 符号、四路 userdb 落盘）真机回归通过，
未见崩溃（全程存活探针 alive=1）。


## 删除自造词（candidate_delete，ABI v2 第 32 符号）回归：2026-11-02

| 步骤 | 命令/操作 | 期望 | 结果 |
|---|---|---|---|
| ABI harness（C 消费者） | `g++` 链 `libior.so`：learn×2→save→新引擎 load→`candidate_delete(0)=0`→preedit 保留→save | 学习词重载置顶、删除后状态码 0/1/2、文件变小、第三引擎不复活 | ✅ HARNESS-OK |
| 真机组码/选词/落盘 | zenity `nihao`+空格 → `fcitx5-remote`/`stat userdb.tsv` | 上屏「你好」，userdb mtime 即时更新 | ✅（输出=你好，mtime +Δ） |
| 托盘菜单删除入口 | classicui 5.1.7 源码复核 | StatusArea 动作渲染于托盘（xcbtraywindow 右键菜单），候选窗 hover 无 IME 回调、仅左键转发 select | ✅ 设计约束确认（菜单项为 stock SimpleAction 管线，人工点选一步待复核） |

结论：删除自造词引擎语义、C ABI 与真机选词持久化实证通过；托盘
`Del word` 菜单点击→点词删除留一条人工复核。

## Shift 防误上屏修复（b6b407d）回归：2026-11-02

| 步骤 | 命令/操作 | 期望 | 结果 |
|---|---|---|---|
| engine 默认（四层） | `cargo test --workspace` | engine_state/ffi/paging_punct/model 断言默认丢弃、点选后切换无残留 | ✅ 371 passed |
| 已安装库 ABI 探针 | `g++ /tmp/shift_probe.cc -L/usr/lib/x86_64-linux-gnu/fcitx5 -lior`：`process_key("nih")×3 → set_ascii_mode(切换) → take_commit`（即 C++ Shift 释放路径的精确序列） | 新库返回 `[]`（旧库返回 `nih`） | ✅ committed=[]（build/ 与已安装副本双验一致） |
| 真机 GUI 自动化 | zenity/tkinter 探针 | 字段仅含后续 ASCII | ⚠️ 未采信：同屏并发会话亦在跑 GUI 验证（焦点互抢、按应用 IM 记忆漂移到 rime），键入分流到对方窗口，空字段为假阴性 |

结论：修复在引擎语义、ABI 契约与已安装产物三层实证通过；GUI 端到端
一步因共享 X 会话焦点争用无法自动化，留人工复核：桌面空闲时输入半截
拼音→按 Shift，预期**无任何上屏**、仅模式切换（提示 English）。
