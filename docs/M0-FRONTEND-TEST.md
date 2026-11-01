# M0 三前端验收记录

- 日期：2026-10-31
- 环境：Ubuntu noble（24.04）、X11 `:0.0`（XFCE 会话）、fcitx5 5.1.7、
  Rust 1.99.1、GTK3 前端（fcitx5-frontend-gtk3 5.1.1）、Qt5 前端
  （fcitx5-frontend-qt5 5.1.4）、XIM（fcitx5 内置 xim addon）
- 方法：xdotool 合成按键驱动真实应用，scrot 全屏截图存证；
  fcitx5 以 `--replace` 重启，`fcitx5-remote -s ior` 显式切换当前输入法
- 安装布局（`sudo cmake --install`）：
  - `/usr/lib/x86_64-linux-gnu/fcitx5/libior.so`（Rust 引擎静态链入）
  - `/usr/share/fcitx5/addon/ior.conf`（addon 元数据，`Library=libior`）
  - `/usr/share/fcitx5/inputmethod/ior.conf`（输入法注册）

## 用例与结果

| 前端 | 载体 | 用例 | 期望 | 结果 |
|------|------|------|------|------|
| GTK3 | `zenity --entry` | `nihao` + Enter | 上屏 `你好`，无原文残留 | ✅ `你好`（rc=0） |
| Qt5 | `kdialog --inputbox` | `nihao` + Space + Enter | 上屏 `你好` | ✅ `你好`（rc=0） |
| XIM | `xterm`（`XMODIFIERS=@im=fcitx`） | `nihao` + Space | 上屏 `你好` | ✅ |
| XIM | 同上 | `xian` + `2` | 数字选词上屏 `先` | ✅ |
| XIM | 同上 | `vk` + Space | 不可解码 → ASCII 原样透传 `vk` | ✅ |

组合断言（XIM 单行）：`你好先vk` 逐字节一致（`GOT '你好先vk\n'`）。

候选窗截图（含 preedit 与候选列表）：

- `docs/m0-candidates-gtk.png`
- `docs/m0-candidates-qt.png`
- `docs/m0-candidates-xim.png`

> 说明：以上为 M0 验收时的历史截图，当时 preedit 使用 client preedit
> （`panel.setClientPreedit`），拼音会内联绘制在客户端文本域。2026-11-01
> 起已改为仅在候选窗顶部使用 `panel.setPreedit`，客户端文本域不再内联回显；
> 现行行为见 `docs/M2-VERIFY.md` 的「候选窗 preedit」小节。

## 实测中发现并修复的问题

1. **双重投递（关键缺陷）**：薄壳 `keyEvent` 只调用 `KeyEvent::filter()`
   时，按键原文仍回传应用，与上屏文本叠加（GTK/XIM 均复现
   `nihao你好`）。修复：消费按键处改用 `KeyEvent::filterAndAccept()`。
2. **addon 命名与布局**：fcitx5 约定模块名为 `lib<name>.so` 且
   `Library=lib<name>`；元数据在 `<datadir>/fcitx5/addon`（Debian 系），
   并非与 .so 同目录。初始版本两处均不符，导致
   `Group Item ior ... is not valid. Removed`。
3. **fcitx5 5.1.7 API 偏差**：`reset(entry, InputContextEvent&)`、
   `AddonFactory::create(AddonManager*)`、`FCITX_ADDON_FACTORY(单参)`、
   `CandidateWord` 为抽象类（需实现 `select` 的子类，顺带获得候选点选提交）。
4. **`FCITX_ADDON_DIRS` 语义**：是覆盖系统扫描路径而非追加，
   指向仅含 ior 的目录会导致 `libwayland.so` 等系统 addon 全部失载。
5. **Enter 语义**：预编辑激活时 Enter 被引擎消费用于上屏（不透传），
   自动化脚本需在提交后再补一次 Enter 关闭对话框/结束行。
6. **用户级安装限制**：`~/.local/lib/fcitx5` 不在 5.1.7 的 addon 库
   搜索路径中（元数据目录 `~/.local/share/fcitx5/addon` 可被发现但库
   定位失败），故采用系统级 `/usr` 安装。

## 遗留（M1 待办，非阻塞）

- 多输入上下文共享单引擎实例（焦点切换可能串扰，M1 改 per-IC 状态）。
- 候选窗翻页（引擎已支持 10 候选，UI 页大小 5，翻页键路由待接）。
- GTK4/Qt6 前端未测（本机装了对应包，但 M0 验收以 GTK3/Qt5 为准）。

## 结论

M0 验收标准（按键 → preedit → 候选 → 上屏，GTK/Qt/XIM 三前端）**全部通过**。
