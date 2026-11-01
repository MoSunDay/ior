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
| `assets/ior.svg` / `ior-48.png` | `/usr/share/icons/hicolor/{scalable,48x48}/apps/` | `Icon=ior` 的主题图标 |

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
#   Shift 单击（无修饰组合）→ 中/英切换    , → ，（中文标点）
#   Page_Down / Page_Up → 候选翻页（编码激活时）
```

## C ABI（v2）

`ior_engine_abi_version()` 返回 **2**；共 **18 个** `ior_` 符号
（声明见 `ioraddon.h`，Rust 侧源 `crates/fcitx5-addon/src/ffi.rs`）。
v2 相对 v1 的变化：

- **候选只暴露当前页**：`ior_engine_candidate_count` / `candidate`
  上限从平铺 10 改为 `page_size`（默认 5）内的当前页；翻页由
  `ior_engine_page_next` / `page_prev`（循环，返回 1=页变化、0=无
  操作）驱动，`ior_engine_page_info` 返回 `(当前页+1)<<16 | 总页数`
  （无候选=0）。
- **候选点击按页内索引提交**：`ior_engine_select(engine,
  page_local_index)` 直接提交当前页第 N 个候选（返回提交文本，无效
  索引返回 `""`），学习/清码与数字选词一致。替代 v1 的「合成数字
  键」方案，修复翻页后点击错选。
- **中英切换**：`ior_engine_ascii_mode`（0 中文 / 1 英文）与
  `ior_engine_set_ascii_mode`（变化时走 core Shift 语义处理未上屏
  编码：CommitRaw/CommitBest/Clear 可配；返回新状态）。
  `ior_engine_process_key` 的 `Shift_L`/`Shift_R` keysym 也会切换
  （**按下即触发**）；本薄壳走「释放时切换 + 修饰检测」路径，不把
  Shift 透传给 `process_key` —— 两种机制**择一使用**，同时启用会
  切换两次。
- **中文标点**：中文模式下可打印 ASCII 标点经引擎映射后上屏
  （`，。；：？！（）【】《》……——` 等，引号 `"`/`'` 交替开闭）；
  编码未上屏时先提交首选候选再接标点；`chinese_punct` 关闭或会话
  `ascii_punct` 打开则原样透传。
- **用户词典持久化**（v2 内纯追加，ABI 仍为 2）：
  `ior_engine_userdb_save / load / decay` 三符号。薄壳把学习词典存到
  `$XDG_DATA_HOME/fcitx5/ior/userdb.tsv`（未设 `XDG_DATA_HOME` 则
  `$HOME/.local/share/fcitx5/ior/userdb.tsv`，目录首次访问即建）；
  引擎初始化时 `load`（返回 1=文件不存在，是正常冷启动，非错误；
  引擎自动识别快照/append-only log 格式并**叠加**进当前词典），成功后
  按文件 mtime 与当前时间差调 `decay(delta, 30 天秒数)` 做时间衰减
  （半衰期 **30 天**、整次减半，mtime 不可得则跳过）；每次上屏
  （空格/数字提交、候选点选、Shift 切换附带提交）后即 `save`——保存
  就是写快照（= 压实），失败静默，持久化绝不阻断输入。save=0/2(IO)/
  -1(null)；load=0/1(不存在)/2(IO)/3(格式)/-1；decay=0/-1
  （`half_life=0` 合法=不变）。

无头/无 X 环境用示例自测引擎链路：
`cargo run -p fcitx5-ior --example m0_demo`。

## 验证清单（按前端逐个测）

| 前端 | 验证方法 | 期望 |
|------|----------|------|
| GTK3 | `zenity --entry`（`GTK_IM_MODULE=fcitx`） | `nihao` preedit `ni'hao`，候选含「你好」，空格上屏 |
| Qt5 | `kdialog --inputbox t`（`QT_IM_MODULE=fcitx`） | 同上；数字 1-9/0 选词；候选可鼠标点选提交 |
| XIM | `xterm`（`XMODIFIERS=@im=fcitx`） | 同上；退格缩短 preedit，Esc 清空 |

补充：`xian` 应出「现 / 先 / 西安」（音节歧义）；预编辑激活时 Enter
被消费用于上屏拼音原码（默认 `engine.enter_action = "raw"`，预期行为）。

## 薄壳实现要点（防止回归）

- 消费按键必须 `keyEvent.filterAndAccept()`：仅 `filter()` 会让原文
  回传应用，造成「原文+上屏」双重投递（实测踩过）。
- preedit 必须用 `panel.setPreedit(...)`，不要用 `setClientPreedit(...)`：
  后者把拼音内联进客户端文本域，输入时应用输入框会回显拼音（实测 bug）；
  改后拼音只在候选窗顶部显示，候选「你好 / 逆号 / 拟好」位于其下。
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
