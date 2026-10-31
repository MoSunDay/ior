# ior fcitx5 addon（C++ 薄壳 + 构建安装）

薄壳只做胶水：把 fcitx5 5.1.x 的按键/焦点事件经 C ABI 转给 Rust 引擎，
再把 preedit / 候选 / 上屏文本刷回输入面板。全部输入法逻辑在
`crates/fcitx5-addon/src`（crate `ior_engine`）。

已在 fcitx5 **5.1.7**（Ubuntu noble）编译并三前端实测通过，
见 `docs/M0-FRONTEND-TEST.md`。

## 前置依赖

```sh
sudo apt-get install -y --no-install-recommends \
  libfcitx5core-dev libfcitx5utils-dev libfcitx5config-dev \
  cmake ninja-build pkg-config
```

## 构建与安装（系统级，实测路径）

```sh
cd crates/fcitx5-addon/cpp
cmake -S . -B build -G Ninja -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build            # 自动先 cargo build --release -p fcitx5-ior
sudo cmake --install build
```

安装布局（fcitx5 约定）：

| 文件 | 目标 | 说明 |
|------|------|------|
| `libior.so` | `/usr/lib/<arch>/fcitx5/` | 模块名必须带 `lib` 前缀（`Library=libior`） |
| `ior.conf` | `/usr/share/fcitx5/addon/` | addon 元数据 |
| `inputmethod/ior.conf` | `/usr/share/fcitx5/inputmethod/` | 输入法注册 |

注意：`cmake --install --prefix` 改不动 configure 期固化的安装目录，
请务必在 configure 时传 `-DCMAKE_INSTALL_PREFIX`。用户级
（`~/.local`）安装不可用：`~/.local/lib/fcitx5` 不在 fcitx5 5.1.7 的
addon 库搜索路径中。

## 启用与快速验证

```sh
fcitx5 --replace &             # 重载 addon（或注销重登）
fcitx5-remote -s ior           # 切到 ior（也可用 fcitx5-configtool）
# GTK3 / Qt5 / XIM 三前端任意应用中输入：
#   nihao + Space → 你好    xian + 2 → 选次候选    vk + Space → 原样透传
```

无头/无 X 环境用示例自测引擎链路：
`cargo run -p fcitx5-ior --example m0_demo`。

## 验证清单（按前端逐个测）

| 前端 | 验证方法 | 期望 |
|------|----------|------|
| GTK3 | `zenity --entry`（`GTK_IM_MODULE=fcitx`） | `nihao` preedit `ni'hao`，候选含「你好」，空格上屏 |
| Qt5 | `kdialog --inputbox t`（`QT_IM_MODULE=fcitx`） | 同上；数字 1-9/0 选词；候选可鼠标点选提交 |
| XIM | `xterm`（`XMODIFIERS=@im=fcitx`） | 同上；退格缩短 preedit，Esc 清空 |

补充：`xian` 应出「现 / 先 / 西安」（音节歧义）；预编辑激活时 Enter
被消费用于上屏（预期行为）。

## 薄壳实现要点（防止回归）

- 消费按键必须 `keyEvent.filterAndAccept()`：仅 `filter()` 会让原文
  回传应用，造成「原文+上屏」双重投递（实测踩过）。
- 5.1.x API：`reset(entry, InputContextEvent&)`、
  `create(AddonManager*)`、`FCITX_ADDON_FACTORY(单参)`、
  `CandidateWord` 抽象类需子类实现 `select`。
- FFI 返回指针仅在下一 `ior_*` 调用前有效：先拷入 `std::string`
  再构造 `fcitx::Text` / 候选列表。
- M0 已知简化：全部输入上下文共享一个引擎实例（M1 改 per-IC 状态）。

## 故障排查

- **`Group Item ior ... is not valid. Removed`**：`inputmethod/ior.conf`
  未装到位，或 addon 元数据（`/usr/share/fcitx5/addon/ior.conf`）缺失，
  或 `Library=` 与 .so 文名不符（应为 `libior` / `libior.so`）。
- **`Could not locate library libior.so`**：模块未装入 fcitx5 扫描的
  lib 目录（Debian 系为 `/usr/lib/<arch>/fcitx5`）。
- **`FCITX_ADDON_DIRS` 陷阱**：它是覆盖而非追加系统扫描路径，
  指向单目录会导致系统 addon 全部失载，排查时慎用。
- **切换到 ior 无反应**：`fcitx5-remote -n` 确认当前 IM；日志
  `fcitx5 --replace` 前台跑看 stderr。
- **符号缺失**：`nm -D /usr/lib/<arch>/fcitx5/libior.so | grep ior_engine_`；
  引擎静态链入，正常应只有 `fcitx_addon_factory_instance` 等导出。
