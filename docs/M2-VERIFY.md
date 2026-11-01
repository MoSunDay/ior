# M2 批次真机回归记录（2026-11-01）

环境：Ubuntu noble / X11（`DISPLAY=:0`）/ fcitx5 5.1.7 / GTK 前端
（mousepad，`GTK_IM_MODULE=fcitx`）/ xdotool 合成键序。验证对象：
luna_pinyin + rime preset vocabulary（`essay.txt`）编译链路、双拼内置表，
以及 addon 的启动配置/词典加载。

## 根因复核：不是 `.ior` 读取 bug

旧现象 `tianqi → 蚕戚` 先被怀疑为 `.ior` 解析错误。复核结果：

- `engine-core` `parse_ior` / `Dictionary::from_ior_bytes` 与 dict-tools
  `export` 对同一份 429,870 条词库逐行一致（排序后 `cmp` 无差异）；
- `tianqi` 缺首选短语的真因是 luna_pinyin 只发字词；Rime 在部署期用
  `essay.txt` 的 preset vocabulary 编码日常短语；
- 旧候选 `蚕戚` 由真实字频（蚕 can 1% / tian 99%）且缺少 `天氣` 短语造成。

## 编译与安装

```sh
cargo run -p dict-tools --release -- compile \
  data/dicts/luna_pinyin.dict.yaml data/dicts/core.ior
scripts/install-core-dict.sh
cmake -S crates/fcitx5-addon/cpp -B build -G Ninja -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build && sudo cmake --install build
```

编译输出（可复现）：

```
293168 keys, 429870 entries, 9444158 bytes (.ior v2)
preset vocabulary data/dicts/essay.txt: 442685 phrase(s), 372241 encoded entrie(s)
compile md5 bf50b9b93f179eec3b6df6dde4623a62
```

同一命令重编前后 md5 相同；`export` 与 `dump_ior` 各 429,870 行，排序后
逐字节一致。安装后词典位于 `/usr/share/fcitx5/ior/core.ior`，addon 位于
`/usr/lib/x86_64-linux-gnu/fcitx5/libior.so`。

## 引擎级候选（`engine-core/examples/probe`）

| 输入 | 首选 | 频次 | 次选 |
|------|------|-----:|------|
| `tian'qi` | 天氣 | 875,100 | 天啓 133,400 |
| `jin'tian` | 今天 | 11,073,100 | 金田 9,700 |
| `ni'hao` | 你好 | 2,149,300 | 妳好 268,600 |
| `zhong'guo` | 中國 | 17,482,400 | 種過 51,600 |
| `de` | 的 | 482,148,112 | 得 8,752,116 |

全拼 `tianqi/jintian/nihao/zhongguo` 与自然码 `jntm/vsgo/nihk` 的
session 级候选均以表中首选开头，见 `cargo run -p engine-core --example probe`。

## fcitx5 真机键序

单次 bash 调用内启动 fcitx5 + mousepad，聚焦窗口后 `fcitx5-remote -s ior`，
键入并 `Ctrl+S` 落盘，再从文件读回：

| 轮次 | 配置 | 键序 | 落盘结果 |
|------|------|------|----------|
| 全拼 | 无 `config.toml`（默认 off） | `tianqi␣` / `jintian␣` / `nihao␣` / `zhongguo␣` | 天氣 / 今天 / 你好 / 中國 |
| 自然码 | `[engine] double_pinyin = "natural"` | `jntm␣` / `vsgo␣` / `nihk␣` | 今天 / 中國 / 你好 |

证据缓冲区：`/tmp/ior-m2-full2.txt`、`/tmp/ior-m2-natural2.txt`（UTF-8）。
测试前清空了 `$XDG_DATA_HOME/fcitx5/ior/userdb.tsv`：用户学习命中会排在静态
词库前，遗留的 `tian'qi 蚕戚` 会把旧候选顶到首位，这是 userdb 的预期优先级，
不是词库回归。测试结束后删除测试 userdb 与临时配置，profile 恢复默认 rime、
保留 ior 为组内可选 IM。

## 门禁

```text
cargo test --workspace                 279/279 passed
cargo clippy --workspace --all-targets -- -D warnings   clean
cargo fmt --check                      clean
scripts/check_line_limits.sh           OK
```

新增 preset 编码单测覆盖 5% 读音过滤、最长匹配、显式短语优先、零权重剔除、
百分比按 essay 权重缩放；`crates/bench/tests/preset_interlock.rs` 覆盖
dict-tools 编码 → `.ior` v2 → engine-core 读回（含 >127 值 key）。
