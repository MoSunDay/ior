# ior fcitx5 插件（M0）

本目录是 ior 输入法引擎的 fcitx5 宿主层：**C++ 壳 + CMake 构建**。
所有输入法逻辑（音节切分、词典、候选、用户词学习）都在 Rust 侧
（`crates/engine-core` 与 `crates/fcitx5-addon/src`，crate `ior_engine`），
C++ 只做事件转发与界面刷新的胶水。

> 说明：本壳针对 **fcitx5 5.1.x** 编写；本机没有 fcitx5 开发头文件和
> cmake，因此本目录只交付源码与构建文件，**不在这台机器上编译**。
> Rust 侧（C ABI）在本机完整编译并通过测试。

## 前置条件

- Rust（stable，cargo ≥ 1.70）
- libfcitx5-core-dev、libfcitx5-utils-dev（fcitx5 ≥ 5.1）
- cmake ≥ 3.16、ninja（或 make）
- fcitx5 运行时（用于实测）

Debian/Ubuntu 示例：

```sh
sudo apt install build-essential cmake ninja-build \
     libfcitx5-core-dev libfcitx5-utils-dev fcitx5
```

## 构建

在 `crates/fcitx5-addon/cpp` 目录下：

```sh
cargo build -p fcitx5-ior --release
cmake -B build -G Ninja ..
cmake --build build
sudo cmake --install build
```

`cmake` 会先通过 `add_custom_target(ior_rust)` 调 `cargo build --release
-p fcitx5-ior` 生成 `target/release/libior_engine.a`（在 workspace 根的
`target/` 下），再把它链接进模块 `ior.so`。可用缓存变量覆盖路径：

- `-DIOR_RUST_DIR=...`：fcitx5-ior 的 cargo 包目录（默认 `..`）
- `-DIOR_WORKSPACE_DIR=...`：workspace 根，即 `target/` 的父目录
  （默认 `../..`）

## 不安装的本地测试

```sh
# 方式一：直接指向构建产物目录
FCITX_ADDON_DIRS=/abs/path/to/crates/fcitx5-addon/cpp/build fcitx5 -r --replace
```

或把 `build/ior.so`、`build/ior.conf` 复制到
`~/.local/share/fcitx5/addon`（各发行版 addons 目录以
`fcitx5-diagnose` 输出为准），`inputmethod/ior.conf` 复制到
`~/.local/share/fcitx5/inputmethod`，然后：

```sh
fcitx5 -r --replace
```

## 启用 "ior"

1. 运行 `fcitx5-configtool`（配置工具）。
2. 「输入法」→「添加输入法」→ 取消勾选「仅显示当前语言」→ 找到
   `ior`（拼音）→ 添加，移动到列表里。
3. 用 Ctrl+Space 切换到 ior。

## 验证清单（按前端逐个测）

| 前端 | 验证方法 | 期望 |
| --- | --- | --- |
| GTK3/GTK4 | `gtk3-demo` 的文本框 | 输入 `nihao` 出现 preedit `ni'hao`、候选含「你好」，空格上屏 |
| Qt5/Qt6 | 任意 Qt 应用（如 `kdialog --inputbox test`） | 同上；数字键 1-9/0 选词 |
| XIM | `xterm`（需 `XMODIFIERS=@im=fcitx`） | 同上；退格缩短 preedit，Esc 清空 |

补充检查：

- 输入 `xian` 应出现「现 / 先 / 西安」（音节歧义切分）。
- 输入中途的逗号等非字母键：M0 约定为清空 preedit 并转发该键。
- 切换焦点/输入法时 preedit 被清空（`reset` 覆写）。

## 故障排查

- **加载报 undefined symbol（ior_engine_...）**：`ior.so` 没有链接
  `libior_engine.a`。确认 `cargo build -p fcitx5-ior --release` 已在
  workspace 根生成 `target/release/libior_engine.a`，且
  `-DIOR_WORKSPACE_DIR` 指向该 workspace 根；必要时
  `nm -D --defined-only <ior.so> | grep ior_engine_` 应能看到符号。
- **配置工具里没有 ior**：`inputmethod/ior.conf` 没装到
  `FCITX_INSTALL_INPUTMETHODDIR`；检查 `cmake --install` 输出路径，
  以及用户级目录 `~/.local/share/fcitx5/inputmethod/`。
- **切换到 ior 后无反应**：`FCITX_DEBUG=1 fcitx5 -r --replace` 看日志；
  确认 `ior.conf` 中 `Library=ior` 与实际 `ior.so` 文件名一致。
- **候选/上屏乱码**：确认终端/应用按 UTF-8 处理；FFI 返回的都是
  NUL 结尾的 UTF-8 字符串。
- **并发**：Rust 引擎按 fcitx5 单线程主循环假设编写，M0 全部输入
  上下文共享一个引擎实例（M1 改为每上下文一份状态）。

## 状态

- 目标平台：fcitx5 5.1.x。
- 本机未编译（无 fcitx5 开发头文件 / cmake）；Rust 侧
  `cargo test -p fcitx5-ior` 与 `cargo run -p fcitx5-ior --example
  m0_demo` 在本机通过。
