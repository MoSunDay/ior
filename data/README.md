# data 目录说明

本目录存放 ior 输入法的词库源文件与格式文档。二进制产物（`.ior`）不入库
（见根目录 `.gitignore`），由 `dict-tools` 从 TSV 构建。

## 目录结构

```
data/
├── dicts/
│   ├── sample-extra.tsv         # IT 领域示例词库（20 条，M0 冒烟测试用）
│   ├── luna_pinyin.dict.yaml    # rime 明月拼音原词库（P0 数据导入）
│   └── essay.txt                # rime preset vocabulary（短语权重）
├── double-pinyin/
│   ├── rime/                    # 上游 schema 原文（含 algebra 键位规则）
│   │   ├── natural.schema.yaml  # 自然碼
│   │   ├── flypy.schema.yaml    # 小鶴
│   │   └── mspy.schema.yaml     # 微軟
│   ├── natural.tsv              # 转出的 ior 键位表（引擎加载格式）
│   ├── flypy.tsv
│   └── mspy.tsv
└── README.md                    # 本文件
```

`data/dicts/core.ior`（luna + essay 编译产物，约 9.4 MB / 293,168 keys /
429,870 词条）**不入库**，由
`cargo run -p dict-tools --release -- compile data/dicts/luna_pinyin.dict.yaml data/dicts/core.ior`
生成（`compile` 会按 dict 头部的 `use_preset_vocabulary: true` 自动读取同目录
`essay.txt`）；安装用 `scripts/install-core-dict.sh`。内置的
`crates/engine-core/assets/core.tsv`（~90 条）仅作为测试/无数据时的兜底。

## 双拼键位表（P1 数据导入落地）

三套真实双拼键位表由 `scripts/convert-rime-double-pinyin.py` 从上游
schema 的 `algebra` 规则**求值**得到，不靠记忆编造：脚本对
`crates/engine-core/src/syllable.rs` 的 404 音节逐个套用 rime 规则
（erase/derive/xform/xlit），再反演出 `final<TAB>key`、`zh|ch|sh<TAB>key`
与 `0syllable<TAB>key` 三种行。转换零冲突；engine-core 的单测再把三张表
逐音节回环校验（仅 `lo`/`luo` 属方案固有碰撞，显式登记）。

- 上游：<https://github.com/rime/rime-double-pinyin>（GPL-3.0）。
  键位映射属于功能性事实数据，本仓库保留出处与许可证说明。
- 重新生成：`python3 scripts/convert-rime-double-pinyin.py`（幂等）。

## preset vocabulary（essay.txt）

luna_pinyin 的 dict.yaml 只收录字词，常用多字短语依赖 Rime 部署期的
preset vocabulary：`use_preset_vocabulary: true`（或显式 `vocabulary: essay`）
时，`compile` 读取输入同级目录的 `essay.txt`，用 dict 中每个字的读音给短语
编码。编码规则对齐 librime：

- 只使用占该字总权重 ≥5% 的读音；
- 最长匹配 DFS，每短语最多 32 个码组合、最长 32 字；
- dict.yaml 中已存在的短语直接跳过（显式词条优先）；
- 权重为 0 或低于 `min_phrase_weight` 的短语丢弃；
- 启用 preset 后所有 rime 权重以 `WEIGHT_SCALE=100` 定点化；百分比行按
  `essay_weight * pct / 100` 缩放。

因此 `.ior` 里除了源 dict.yaml 的 70,055 条，还会多出约 37 万条 essay
编码短语。`essay.txt` 正文来自 rime-essay master，保留 3 行来源/许可头。

## TSV 词库格式

每行一条词条：

```
pinyin<TAB>hanzi<TAB>freq
```

- `pinyin`：小写全拼，音节分隔符统一为 `'`（如 `shu'ju'ku`）。
  源文件中用空格或 `'` 分隔均可，`dict-tools build` 会自动归一化为 `'`。
- `hanzi`：候选词文本（UTF-8）。
- `freq`：可选，词频，非负整数，默认 `1`。
- `#` 开头的注释行与空白行会被忽略。
- 解析错误会带行号报告（退出码 3）。

归一化规则（`dict_tools::format::normalize`）：

- 按 `(pinyin, hanzi)` 去重，freq 求和；
- 排序：`pinyin` 升序 → `freq` 降序 → `hanzi` 升序，保证输出确定。

示例：

```tsv
# 注释
kai'fa	开发	500
cheng'xu	程序
```

## `.ior` 二进制格式（小端序）

由 `crates/dict-tools/src/format.rs` 定义。当前写出版本为 **v2**；
读取端同时兼容 v1（`dict-tools deserialize` 与 `engine-core
dict::Dictionary::from_ior_bytes` 行为逐字节一致，两边常量各自实现）。

### v2（当前版本）

头 13 字节：

| 偏移 | 长度 | 字段        | 说明 |
|-----:|-----:|-------------|------|
| 0    | 4    | MAGIC       | 固定 `"IORD"` |
| 4    | 1    | VERSION     | `u8`，当前为 `2` |
| 5    | 4    | entry_count | `u32` LE，(key,hanzi) 总对数 |
| 9    | 4    | key_count   | `u32` LE，去重后的 key 数 |

之后是两个区域：

1. **key 索引区**：`key_count` 条记录，每条
   `varint(key_len) + key UTF-8 字节 + varint(values_len)`。
   key 按字节序升序、已去重（多个词条合并到同一 key 下）。
2. **值区**：按 key 索引顺序，每个 key 连续 `values_len` 条
   `varint(hanzi_len) + hanzi UTF-8 字节 + varint(freq)`。
   同一 key 内按 freq 降序、hanzi 升序（与 `normalize` 排序一致）。

**varint** = LEB128 无符号 u32：每字节低 7 位为负载，最高位是连续
标志；最多 5 字节，第 5 字节只允许低 4 位（实现见
`crates/dict-tools/src/varint.rs`）。

### v1（旧版，仅读取兼容）

| 偏移 | 长度 | 字段          | 说明 |
|-----:|-----:|---------------|------|
| 0    | 4    | MAGIC         | 固定 `"IORD"` |
| 4    | 1    | VERSION       | `u8` = `1` |
| 5    | 4    | entry_count   | `u32`，词条数 |
| 之后 | 变长 | entries       | 每条记录依次排列（见下） |

每条 entry 记录：

```
pinyin_len (u32) | pinyin UTF-8 字节 | hanzi_len (u32) | hanzi UTF-8 字节 | freq (u32)
```

所有多字节整数均为小端序。两个版本的读取都校验 magic、版本号、
截断、UTF-8 合法性（v2 另校验 entry_count 与值区条数一致）以及结尾
多余字节，错误信息为带偏移量/条目号的可读字符串。

## 词库工具

```sh
cargo run -p dict-tools -- build    data/dicts/sample-extra.tsv /tmp/extra.ior
cargo run -p dict-tools -- inspect  /tmp/extra.ior
cargo run -p dict-tools -- export   /tmp/extra.ior /tmp/extra-back.tsv
cargo run -p dict-tools -- merge    /tmp/a.ior /tmp/b.ior -o /tmp/merged.ior
cargo run -p dict-tools -- compile  data/dicts/sample-extra.tsv /tmp/extra.ior
cargo run -p dict-tools -- validate data/dicts/sample-extra.tsv --dump-syllables
```

`compile`/`validate` 的输入自动识别两类格式：

- ior TSV：`pinyin<TAB>hanzi[<TAB>freq]`（pinyin 在前）；
- rime dict.yaml 表：存在单独一行内容恰为 `...` 即按 rime 处理，
  该行之前全部跳过，其后每行 `hanzi<TAB>pinyin[<TAB>weight]`
  （hanzi 在前；无 tab 时按空格分列）。

`validate` 的结构校验规则：pinyin 仅允许 `[a-z]` 与 `'`，不得有
连续/首尾 `'`，音节非空；hanzi 非空且不含制表/换行；`freq ≥ 1`；
重复 `(pinyin,hanzi)` 汇总求和并打印提示而非报错。违规以退出码 3
报告并带行号。`--dump-syllables` 打印去重音节清单（dict-tools 不依赖
engine-core、不内嵌 403 音节表，需人工比对）。

## 词库来源标注模板

进入仓库的每个词库源文件头部必须注明来源与许可证，模板如下
（rime luna_pinyin 导入示例）：

```tsv
# source: rime-luna-pinyin (https://github.com/rime/rime-luna-pinyin)
# license: LGPL-3.0 (词库数据部分以各自上游许可为准)
# imported: cargo run -p dict-tools -- compile luna_pinyin.dict.yaml data/dicts/core.ior

已导入的源文件（原样保留上游头部注释，含出处/许可）：

| 文件 | 上游 | 规模 | 许可证 |
|------|------|------|--------|
| `dicts/luna_pinyin.dict.yaml` | rime-luna-pinyin | 70,055 词条（19,920 键） | LGPL-3.0 |
| `dicts/essay.txt` | rime-essay | 442,685 短语（5,887,490 B） | LGPL-3.0 |
| `double-pinyin/rime/*.schema.yaml` | rime-double-pinyin | 三套 schema | GPL-3.0 |
```

## 二期模型清单

语音与 LLM 模型不进入本仓库（`data/models/`、`*.onnx`、`*.gguf` 均已
在 `.gitignore` 中忽略）。计划清单见 `crates/voice`（`default_model_manifest`）
与 `scripts/download_models.sh`（M0 仅打印计划，V4 实现下载）：

| 模型 | 用途 | 大小（约） |
|------|------|-----------:|
| silero-vad | 语音活动检测（VAD） | 2 MB |
| sherpa-onnx zipformer 中英双语流式 | 流式 ASR | 100 MB |
| whisper-small | 离线精解码 | 460 MB |
| whisper-turbo | 离线快速解码 | 750 MB |
| qwen2.5-1.5b/3b/7b-instruct（Q4_K_M） | 本地润色/标点 | 1-4.5 GB |

## 数据版权与来源（重要）

- **搜狗细胞词库（`.scel`/`.ssf`）不可分发**：转换自搜狗词库的数据
  不得提交到本仓库或随发布包分发。
- 可用来源：
  - [rime luna_pinyin](https://github.com/rime/rime-luna-pinyin)：开源词库；
  - [CC-CEDICT](https://cc-cedict.org/)：CC-BY-SA 4.0，需署名并以相同
    协议共享衍生词库；
  - 维基百科/维基词典语料自建词库：注意遵循各自许可与署名要求。
- 任何进入本仓库的词库文件必须在其头部注释中注明来源与许可证。
