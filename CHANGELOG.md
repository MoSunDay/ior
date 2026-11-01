# Changelog

本项目所有显著变更记录于此。格式参考 Keep a Changelog，版本遵循 SemVer。

## [Unreleased] - Rime 对标第一批（P0 / P5-lite / P3）

规划全貌见 `docs/ROADMAP-RIME-PARITY.md`；本批落地「若只能先做三件事」的全部三项。

### P0 = M1：词库二进制与加载
- `.ior` 格式升级 **v2**：13 字节头 + key 索引区（varint 长度前缀、key 升序去重）+ 值区
  （freq 降序），freq 采用 LEB128 varint；读取端 v1/v2 双兼容，损坏输入报偏移与条目上下文。
- `engine-core`：`Dictionary::from_ior_bytes` / `from_ior_layers`（base→领域→用户多词典叠加，
  freq 求和）；core 自带最小 reader，仍零依赖、无 unsafe；`builtin` TSV 保留为回退。
- `dict-tools`：新增 `compile`（ior TSV / rime dict.yaml 表自动识别 → .ior v2）与
  `validate`（结构校验，`--dump-syllables` 导出音节清单）；新增 `source.rs` / `varint.rs` 模块。
- 新增 `ior-bench` crate（自研 Instant 计时，不引 criterion）：10 万词编译 **125 ms**
  （目标 <1s）、加载中位 26.7 ms、lookup **P99 0.39 µs**（目标 <10 µs）。
- `data/README.md`：v2 字节布局规范与 rime 词库导入示例。

### P5-lite = M4 先行项：分页 / Shift 中英 / 中文标点
- `engine-core`：`Session` 增加分页状态（`page_count/current_page/next_page/prev_page/
  select_flat`，`page_size` 真正生效，数字键按当前页偏移选词，改码页归零）；
  `process_shift` 中英切换（`ShiftAction::{CommitRaw,CommitBest,Clear}` 三态可配）；
  新增 `punct.rs` 数据驱动中文标点（含 “” ‘’ 配对交替、全角映射，`chinese_punct` 开关）。
- FFI 升级 **ABI v2**（9→18 符号）：翻页 `page_next/prev/page_info`、候选点击改
  `ior_engine_select`（页内索引直提，修复翻页错位）、`ascii_mode/set_ascii_mode`、
  Shift/PageUp/PageDown keysym、userdb 三符号（见 P3）。
- C++ 薄壳：候选点击不再合成数字键；Page_Up/Down 翻页；Shift 释放且未作修饰时切换中英；
  候选列表页大小统一为引擎当前页（消除 5 vs 10 不一致）。

### P3 = M3：用户词典持久化
- `engine-core`（无 I/O，只收/回 bytes）：`user_dict_store.rs` —— 快照
  `to_snapshot/from_snapshot`（`# ior userdb v1` TSV）、append-only 日志
  `log_line/replay`、压实 `compact`（幂等）、时间衰减 `decay`（整数次减半、零频清除）、
  云备份 `export_tsv/import_tsv`、合并 `absorb`。
- addon 层：`ior_engine_userdb_save/load/decay`；C++ 侧 userdb 路径
  `$XDG_DATA_HOME/fcitx5/ior/userdb.tsv`，启动按 mtime 差做 30 天半衰期衰减；
  全部四条学习路径（键盘提交/候选点选/Shift 切换附带提交/析构兜底）均落盘快照
  （快照即压实，失败静默不阻断输入）。

### P1 = M2a：拼式管线数据化（模糊音查询化 + 简拼 + 双拼框架）
- 模糊音从「构建期烘焙进词典 key」改为**查询侧等价类展开**（`spelling.rs`：
  `syllable_variants`/`key_variants`，声母×韵母独立组合、跨变体取最高频不叠加）；
  词典 key 保持全拼规范形，`from_tsv`/`from_ior_bytes` 系列去模糊参数。
- 简拼：`Dictionary` 新增声母首字母 BTreeMap 索引 + `lookup_initials` 前缀查询，
  简拼候选排全拼之后、每 key 一条（`jt`→今天/`jin'tian`）。
- 双拼框架：`DoublePinyin{Off,Custom}` + 可加载键位表（`double_pinyin.rs`
  `parse_tsv`/`to_full`，逐音节 `is_syllable` 校验，失败回退全拼）。真实键位表
  （自然码/小鹤/微软）因离线未编造，待网络恢复以数据导入。
- addon `candidates` 薄封装保持原签名，宿主零改动。

### 数据导入（P0 收尾，网络恢复后落地）
- 词库：导入 rime **luna_pinyin**（LGPL-3.0，出处与许可保留在文件头），
  `compile` 产出 **70,055 词条 / 19,920 键**，`data/dicts/core.ior`（约 730 KB，
  按仓库策略不入库，`scripts/install-core-dict.sh` 一键编译安装）。
- `dict-tools` 权重解析增强：rime 词库的百分比权重（如 `99.93%`）按万分比
  整数化；零权重（`0`/`0%`）按 rime 语义视为**剔除条目**而非权重 1。
- 双拼真实键位表：新增 `scripts/convert-rime-double-pinyin.py`，对上游
  rime-double-pinyin schema 的 `algebra` 规则逐音节**求值**，转出自然碼/
  小鶴/微軟三张键位表（`data/double-pinyin/*.tsv`，GPL-3.0 出处注明），零冲突。
- `engine-core` 双拼框架扩展：支持多字母声母行（`zh/ch/sh<TAB>key`）、
  同键多零声母音节（自然碼 `e/ei/en/eng/er` 共享 e 键）、`;` 键
  （微軟 `ing=;`）；三张真实表对 404 音节逐码回环单测（仅 `lo`/`luo`
  方案固有碰撞，显式登记）。

### P2 = M2b：整句解码（`.gram` n-gram 语言模型 + beam search）
- `.gram` v1 二进制语言模型格式（规范以 `engine-core/src/lm.rs` 模块文档为权威）：
  24 字节头（magic `IOGRAM` + version + order + 4×u32 LE 计数）+ unigram/bigram/trigram
  三段（索引区 `varint(len)+utf8 key` 严格升序、值区 varint 计数，varint 与 `.ior` v2
  同为严格 LEB128）。engine-core 零 I/O 读取（`lm::from_gram_bytes(&[u8])`，损坏输入
  报 offset/key 序号）；引擎内部 packed key（unigram u32、bigram/trigram 3×21 bit u64），
  打分零字符串分配。
- dict-tools 新增 `train <in> <out.gram>`（ior TSV / rime dict.yaml 自动识别 → 词表内
  **字符级** n-gram，按词频加权、窗口不跨词边界——离线仅有词表可作语料，语料级模型待
  真实数据）与 `lminspect`（计数/总权重/top key 校验读取）；`gram.rs` 双端 golden bytes
  互锁（47 字节样本）。
- 整句 beam search（`engine-core/beam.rs` + `beam_search.rs`，纯函数、每轮新 Vec）：
  词 lattice（每起点×词长探测拼写变体，含 `(0,n)` 全句 key 覆盖用户整句学习）+
  bucket-per-position 搜索（按位置分桶、sort+dedup+truncate 修剪）；打分 = Stupid
  Backoff 三元回退（惩罚 0.4/级，OOV 伪计数 0.5 杜绝 -inf）+ `freq_weight·ln(dict_freq)`
  + `user_weight·ln(USER_BOOST+user_freq)` + 整句完成加成；`BeamParams` 全量可调
  （默认 beam_width 8），不新增 EngineConfig 字段。
- 接线：`decoder::candidates_with_lm(dict,user,Option<&GramModel>,fuzzy,raw,limit)`——
  无模型/简拼/不可切分/音节数 < `MIN_BEAM_SYLLABLES`(3)/beam_width=0 一律回退旧三步
  拼装；beam 结果尾部合并旧路径候选（去重、按 limit 截断），任何输入不劣于 M0。
  `session.rs` 未动（禁碰），宿主层后续接线。
- bench：新增场景 [4] 整句解码（dict-tools 训练 → engine-core beam 计时，兼作跨 crate
  格式互锁）与 `--help`；fixture 8 音节整句 P50 约 107 µs，top1 = 今天天气不错很好
  （LM 链式打分胜过 金田…）。`crates/bench/tests/gram_interlock.rs`：常量对齐、
  字节互认、整句端到端、坏模型拒收。

### CI 与工程
- `.github/workflows/ci.yml`：fmt/clippy/test + 行数红线（400/800 规范脚本
  `scripts/check_line_limits.sh`）+ bench 参考性回归（P0 基线门禁不阻断）。

### P4 = M3.5：配置体系第一步（新 crate `ior-config`）
- **TOML 子集解析器**（零第三方依赖、无 unsafe、纯函数）：`[section]`/`[a.b]`
  表头、`key = value`（含点分键）、基本字符串（`\" \\ \n \t \u{XXXX}`）、
  十进制整数、布尔、单行数组（嵌套一层）、`#` 注释；解析失败一律返回带
  1-based 行/列的 `ParseError`（`line N, column M: ...`），绝不 panic。
  多行字符串/浮点/日期/内联表/`[[数组表]]`/重复表头/数字键等越界语法给出
  明确报错。
- **配置 schema**（`model.rs`/`schema.rs`）：键名与默认值 1:1 对齐
  `engine-core/config.rs`（`engine.shift_action`=`commit|temp|none`、
  `engine.double_pinyin`=`off|custom`（真实双拼方案枚举待 P1 数据导入）、
  `chinese_punct`/`user_learning`/`page_size`/`max_candidates`、
  `[engine.fuzzy]` 8 开关、`switches.<name>` 动态布尔表、`punct.<name>`
  字符映射占位）；错误带点分路径（如 `engine.shift_action: expected one
  of ...`），未知键收集为 warnings 不阻断加载。
- **用户 patch 叠加**（`patch.rs`）：`apply(default, user) -> Result<Config,
  String>` 深合并（表内按键合并、数组整体替换、类型不匹配报带路径错误）；
  `serialize_config/serialize_table` 输出确定性格式（标量在前、表头在后、
  2 空格缩进、插入序键序），parse→serialize→parse 语义等价。
- 本批只做 crate 本身：未接线 FFI/addon，未改动任何现有 crate 运行时行为
  （除 workspace members 一行）。

### 验证
- `cargo test --workspace`：**193/193** 通过（+35）；`clippy -D warnings`、
  `cargo fmt --check` 全绿；fcitx5 C++ cmake 构建通过，`nm -D` 18 符号。
- **真机回归**（Ubuntu noble/X11/fcitx5 5.1.7/GTK，xdotool 合成键序）：
  全拼/翻页+页内选词/中文标点/配对引号/Shift 中英/长词提交全通过；userdb
  四路落盘与跨重启持久化实测确认。记录见 `docs/M1-VERIFY.md`（含截图）。
  过程中评审发现并修复 saveUserdb 仅覆盖 1/3 提交路径的缺陷。
## [0.1.0-m0] - 2026-10-31

M0 里程碑：引擎骨架 + fcitx5 薄壳 + 词级转换全链路打通，GTK/Qt/XIM 三前端
实测通过（Ubuntu noble / X11 / fcitx5 5.1.7）。

### 新增
- `engine-core`：403 音节表、后缀 DP 全切分、模糊音（8 组）、内置 93 词条
  词库、词级解码（整词→用户词→组合）、用户词典自学习、会话状态机
  （字母/空格/数字选词/退格/ESC/非法拼音 ASCII 透传）。
- `fcitx5-ior`：C ABI 桥（9 个 `ior_engine_*` 符号，X11 keysym 映射、
  空指针安全、`ior_engine_abi_version`）+ fcitx5 5.1.7 C++ 薄壳
  （`filterAndAccept` 拦截回传、可点选 `CandidateWord`）+ CMake 安装布局。
- `dict-tools`：TSV ↔ `.ior` 二进制（build/inspect/export/merge）。
- `voice` / `llm`：二期规划面（管线清单、延迟预算与降级策略、prompt 模板）。
- `data/`：样例词库、格式与数据版权说明（搜狗 scel/ssf 不可分发）。
- `docs/M0-FRONTEND-TEST.md`：三前端验收记录与证据截图。

### 修复（实测过程中发现）
- 薄壳 `keyEvent` 仅 `filter()` 不足以阻止按键回传应用，导致「原文+上屏」
  双重投递；改用 `filterAndAccept()`。
- addon 命名/布局：模块需为 `libior.so`（`Library=libior`），元数据装
  `<datadir>/fcitx5/addon`，而非与 .so 同目录。
- 非法拼音空格上屏时剔除用户敲入的 `'` 分隔符。

### 验证
- `cargo test --workspace`：78/78 通过；clippy `-D warnings`、fmt 全绿。
- 三前端端到端：GTK `你好`、Qt `你好`、XIM `你好/先/vk`（选词与透传）。
