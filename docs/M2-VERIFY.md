# M2 批次真机回归记录（2026-11-01）

环境：Ubuntu noble / X11（`DISPLAY=:0`）/ fcitx5 5.1.7 / GTK 前端
（mousepad，`GTK_IM_MODULE=fcitx`）/ xdotool 合成键序。验证对象：
luna_pinyin + rime preset vocabulary（`essay.txt`）编译链路、双拼内置表、
全量 `.gram` 训练/加载，以及 addon 的启动配置/词典/语言模型加载。

## 根因复核：不是 `.ior` 读取 bug

旧现象 `tianqi → 蚕戚` 先被怀疑为 `.ior` 解析错误。复核结果：

- `engine-core` `parse_ior` / `Dictionary::from_ior_bytes` 与 dict-tools
  `export` 对同一份 429,870 条词库逐行一致（排序后 `cmp` 无差异）；
- `tianqi` 缺首选短语的真因是 luna_pinyin 只发字词；Rime 在部署期用
  `essay.txt` 的 preset vocabulary 编码日常短语；
- 旧候选 `蚕戚` 由真实字频（蚕 can 1% / tian 99%）且缺少 `天氣` 短语造成。

## 编译与安装

```sh
scripts/install-core-dict.sh   # compile + export + train + install
cmake -S crates/fcitx5-addon/cpp -B build -G Ninja -DCMAKE_INSTALL_PREFIX=/usr
cmake --build build && sudo cmake --install build
```

安装脚本输出（可复现）：

```
compiled core.ior: 293168 keys, 429870 entries, 9444158 bytes (.ior v2)
preset vocabulary data/dicts/essay.txt: 442685 phrase(s), 372241 encoded entrie(s)
exported /tmp/…: 429870 entries
trained core.gram: 38979 unigram / 279175 bigram / 297665 trigram keys,
  total weight 74911345415, 6270034 bytes
installed: /usr/share/fcitx5/ior/core.ior
installed: /usr/share/fcitx5/ior/core.gram
```

同一命令重编前后 md5 相同；`export` 与 `dump_ior` 各 429,870 行，排序后
逐字节一致。安装后词典/模型位于 `/usr/share/fcitx5/ior/core.{ior,gram}`
（md5 `bf50b9b93f179eec3b6df6dde4623a62` /
`51ff74f0bfc78b8961aa45c9065de8c9`），addon 位于
`/usr/lib/x86_64-linux-gnu/fcitx5/libior.so`。

## 全量 `.gram` 训练与加载

全量词库的 raw 总权重为 74,911,345,415，超过 `.gram` v1 头的 `u32` 上限。
writer 在超限时对**所有**计数统一除以同一除数（非零值保底 1），序列化头因此为
4,161,752,858；reader 仍严格校验「头值 == unigram 值之和」。engine-core 与
dict-tools 的 `lminspect` 均能读取安装后的模型，跨 crate 测试另覆盖了合成溢出
样本，防止两端各自“修好”却语义分叉。

addon 启动按 `$XDG_DATA_HOME/fcitx5/ior/core.gram` →
`/usr/share/fcitx5/ior/core.gram` 查找；缺失时静默使用无 LM 路径，损坏时打印
诊断并回退。模型由安装脚本从**已编译** `core.ior` 导出训练，preset 短语不会漏训。

## 引擎级候选（`engine-core/examples/probe`）

| 输入 | 首选 | 频次 | 次选 |
|------|------|-----:|------|
| `tian'qi` | 天氣 | 875,100 | 天啓 133,400 |
| `jin'tian` | 今天 | 11,073,100 | 金田 9,700 |
| `ni'hao` | 你好 | 2,149,300 | 妳好 268,600 |
| `zhong'guo` | 中國 | 17,482,400 | 種過 51,600 |
| `de` | 的 | 482,148,112 | 得 8,752,116 |

全拼 `tianqi/jintian/nihao/zhongguo` 与自然码 `jntm/vsgo/nihk` 的
session 级候选均以表中首选开头。加载 `core.gram` 后整句解码：

```
jintian'tianqi'bu'cuo → 今天天氣不錯
wo'xiang'qu'bei'jing  → 我想去北京
```

复现：`cargo run -p engine-core --example probe -- \
  /usr/share/fcitx5/ior/core.ior /usr/share/fcitx5/ior/core.gram`。

## fcitx5 真机键序

单次 bash 调用内启动 fcitx5 + mousepad，聚焦窗口后 `fcitx5-remote -s ior`，
键入并 `Ctrl+S` 落盘，再从文件读回：

| 轮次 | 配置 | 键序 | 落盘结果 |
|------|------|------|----------|
| 全拼 | 无 `config.toml`（默认 off） | `tianqi␣` / `jintian␣` / `nihao␣` / `zhongguo␣` | 天氣 / 今天 / 你好 / 中國 |
| 自然码 | `[engine] double_pinyin = "natural"` | `jntm␣` / `vsgo␣` / `nihk␣` | 今天 / 中國 / 你好 |
| 全拼整句 + LM | 安装 `core.gram`（默认路径） | `jintiantianqibucuo␣` / `woxiangqubeijing␣` | 今天天氣不錯 / 我想去北京 |

证据缓冲区：`/tmp/ior-m2-full2.txt`、`/tmp/ior-m2-natural2.txt`（UTF-8）。
测试前清空了 `$XDG_DATA_HOME/fcitx5/ior/userdb.tsv`：用户学习命中会排在静态
词库前，遗留的 `tian'qi 蚕戚` 会把旧候选顶到首位，这是 userdb 的预期优先级，
不是词库回归。测试结束后删除测试 userdb 与临时配置，profile 恢复默认 rime、
保留 ior 为组内可选 IM。

## 门禁

```text
cargo test --workspace                 282/282 passed
cargo clippy --workspace --all-targets -- -D warnings   clean
cargo fmt --check                      clean
scripts/check_line_limits.sh           OK
```

新增 preset 编码单测覆盖 5% 读音过滤、最长匹配、显式短语优先、零权重剔除、
百分比按 essay 权重缩放；`crates/bench/tests/preset_interlock.rs` 覆盖
dict-tools 编码 → `.ior` v2 → engine-core 读回（含 >127 值 key）。
`crates/bench/tests/gram_interlock.rs` 另覆盖 `u32` 溢出缩放、引擎/会话 LM
路由与整句选择；addon 单测覆盖 `core.gram` 路径顺序及缺失/损坏文件回退。
