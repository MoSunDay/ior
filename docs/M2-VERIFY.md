# M2 批次真机回归记录（2026-11-01）

环境：Ubuntu noble / X11（`DISPLAY=:0`）/ fcitx5 5.1.7 / GTK 前端
（mousepad，`GTK_IM_MODULE=fcitx`）/ xdotool 合成键序。验证对象：
luna_pinyin + rime preset vocabulary（`essay.txt`）编译链路、编译期简体
（OpenCC t2s）、不完整输入前缀回退、双拼内置表、全量 `.gram` 训练/加载，
以及 addon 的启动配置/词典/语言模型加载。

## 根因复核：不是 `.ior` 读取 bug（简体化前历史记录）

旧现象 `tianqi → 蚕戚` 先被怀疑为 `.ior` 解析错误。复核结果：

- `engine-core` `parse_ior` / `Dictionary::from_ior_bytes` 与 dict-tools
  `export` 对同一份 429,870 条词库逐行一致（排序后 `cmp` 无差异）；
- `tianqi` 缺首选短语的真因是 luna_pinyin 只发字词；Rime 在部署期用
  `essay.txt` 的 preset vocabulary 编码日常短语；
- 旧候选 `蚕戚` 由真实字频（蚕 can 1% / tian 99%）且缺少 `天氣` 短语造成。
  以下编译计数为简体化前一轮；本轮简体结果见后续章节。

## 编译与安装

```sh
scripts/install-core-dict.sh   # compile + export + train + install
cmake -S crates/fcitx5-addon/cpp -B build -G Ninja -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build && sudo cmake --install build
```

安装脚本输出（可复现）：

```
compiled core.ior: 293168 keys, 427661 entries, 9424662 bytes (.ior v2)
preset vocabulary data/dicts/essay.txt: 442685 phrase(s), 372241 encoded entrie(s)
simplified 268003 entrie(s) with bundled OpenCC t2s tables
exported /tmp/…: 427661 entries
trained core.gram: 38176 unigram / 276582 bigram / 297084 trigram keys,
  total weight 74911345415, 6235618 bytes
installed: /usr/share/fcitx5/ior/core.ior
installed: /usr/share/fcitx5/ior/core.gram
```

同一命令重编前后 md5 相同；本轮 `export` 为 427,661 行，简体化前的
往返比对同为逐字节一致。安装后词典/模型位于
`/usr/share/fcitx5/ior/core.{ior,gram}`
（md5 `38553bed48c87835ab23d8ebd5d731d8` /
`78e9434cf5bdd05b074dea4d5f05c6d9`），addon 位于
`/usr/lib/x86_64-linux-gnu/fcitx5/libior.so`
（md5 `31f32e0d2a86bef75cb271cf66bf317e`）。

## 默认简体（`compile --simplify`）

`data/opencc/` 内置 OpenCC ver.1.4.2 的 `TSPhrases.txt` 与
`TSCharacters.txt`（Apache-2.0，哈希见 `data/opencc/README.md`）。`compile`
在构建期按「最长短语 → 单字」转换，取每行首值，再按简化后的
`(key, hanzi)` 归一化去重；引擎与运行时没有 OpenCC 依赖。本轮改写
268,003 词条，同音同形合并使 429,870 → 427,661 词条，`keys` 仍为
293,168，产物 9,424,662 字节。`scripts/install-core-dict.sh` 默认启用
`--simplify`。`妳` 不在上游单字表，所以 `妳好` 作为候选保留，但首选仍是
`你好`。

## 全量 `.gram` 训练与加载

全量词库的 raw 总权重为 74,911,345,415，超过 `.gram` v1 头的 `u32` 上限。
writer 在超限时对**所有**计数统一除以同一除数（非零值保底 1），序列化头因此为
4,161,752,842；reader 仍严格校验「头值 == unigram 值之和」。engine-core 与
dict-tools 的 `lminspect` 均能读取安装后的模型，跨 crate 测试另覆盖了合成溢出
样本，防止两端各自“修好”却语义分叉。

addon 启动按 `$XDG_DATA_HOME/fcitx5/ior/core.gram` →
`/usr/share/fcitx5/ior/core.gram` 查找；缺失时静默使用无 LM 路径，损坏时打印
诊断并回退。模型由安装脚本从**已编译** `core.ior` 导出训练，preset 短语不会漏训。

## 引擎级候选（`engine-core/examples/probe`）

| 输入 | 首选 | 频次 | 次选 |
|------|------|-----:|------|
| `tian'qi` | 天气 | 875,100 | 天启 133,400 |
| `jin'tian` | 今天 | 11,073,100 | 金田 9,700 |
| `ni'hao` | 你好 | 2,149,300 | 妳好 268,600 |
| `zhong'guo` | 中国 | 17,482,400 | 种过 51,600 |
| `de` | 的 | 482,148,112 | 得 8,752,116 |

不完整输入在常规解码无候选时走前缀回退，完整输入顺序不变：

| 输入 | 候选（前 6 条） |
|------|------------------|
| `n` | 你、那、呢、那么、能、你的 |
| `nih` | 你会、你好、你会发现、你还、你还是、你还要 |
| `niha` | 你哈 |
| `nihao` | 你好、妳好、逆号、拟好（完整输入，不走回退） |

全拼 `tianqi/jintian/nihao/zhongguo` 与自然码 `jntm/vsgo/nihk` 的
session 级候选均以表中首选开头。加载 `core.gram` 后整句解码：

```
jintian'tianqi'bu'cuo → 今天天气不错
wo'xiang'qu'bei'jing  → 我想去北京
```

复现：`cargo run -p engine-core --example probe -- \
  /usr/share/fcitx5/ior/core.ior /usr/share/fcitx5/ior/core.gram`。

## fcitx5 真机键序

单次 bash 调用内启动 fcitx5 + mousepad，聚焦窗口后 `fcitx5-remote -s ior`，
键入并 `Ctrl+S` 落盘，再从文件读回：

| 轮次 | 配置 | 键序 | 落盘结果 |
|------|------|------|----------|
| 全拼 | 无 `config.toml`（默认 off） | `tianqi␣` / `jintian␣` / `nihao␣` / `zhongguo␣` | 天气 / 今天 / 你好 / 中国 |
| 自然码 | `[engine] double_pinyin = "natural"` | `jntm␣` / `vsgo␣` / `nihk␣` | 今天 / 中国 / 你好 |
| 不完整输入 + 整句 | 安装 `core.gram`（默认路径） | `nih␣` / `n␣` / `jintiantianqibucuo␣` | 你会 / 你 / 今天天气不错 |

证据缓冲区：`/tmp/ior-final-full.txt`、`/tmp/ior-final-natural.txt`、
`/tmp/ior-final-prefix.txt`（UTF-8；本轮简体安装，单次 bash 调用完成）。
测试前清空了 `$XDG_DATA_HOME/fcitx5/ior/userdb.tsv`：用户学习命中会排在静态
词库前，遗留的 `tian'qi 蚕戚` 会把旧候选顶到首位，这是 userdb 的预期优先级，
不是词库回归。测试结束后删除测试 userdb 与临时配置，profile 恢复默认 rime、
保留 ior 为组内可选 IM。

## 门禁

```text
cargo test --workspace                 292/292 passed
cargo clippy --workspace --all-targets -- -D warnings   clean
cargo fmt --check                      clean
scripts/check_line_limits.sh           OK
```

新增 preset 编码单测覆盖 5% 读音过滤、最长匹配、显式短语优先、零权重剔除、
百分比按 essay 权重缩放；`dict-tools/src/simplify.rs` 的字/短语转换与
`crates/engine-core/src/prefix_index.rs` 的前缀查找各有单测，
`crates/fcitx5-addon/src/ffi.rs` 另覆盖 `n`/`nih` 的候选暴露；`crates/bench/tests/preset_interlock.rs` 覆盖
dict-tools 编码 → `.ior` v2 → engine-core 读回（含 >127 值 key）。
`crates/bench/tests/gram_interlock.rs` 另覆盖 `u32` 溢出缩放、引擎/会话 LM
路由与整句选择；addon 单测覆盖 `core.gram` 路径顺序及缺失/损坏文件回退。
