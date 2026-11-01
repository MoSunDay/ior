# M1 批次真机回归记录（2026-11-01）

环境：Ubuntu noble / X11（DISPLAY=:0）/ fcitx5 5.1.7 / GTK 前端（mousepad，
`GTK_IM_MODULE=fcitx`）/ xdotool 合成键序 / scrot 截图 / userdb 落盘与
文本缓冲区双判据。本批验证对象：P0/P5-lite/P3 已提交代码（HEAD b56423c
之后的 saveUserdb 四路落盘修复）。

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
