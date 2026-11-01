# ior vs Rime：差距评审与补齐规划

> 定位：ior 对标搜狗输入法的**本地拼音输入**能力，而非复刻 Rime 全方案引擎。
> 本文以 Rime 为参照系盘点差距，给出 P0–P6 补齐规划（对齐既有里程碑
> M1–M5），并标注应保持的反超点与明确不做的事。

## 1. 现状定位（2026-11-01 复审）

M0 词级链路之后，M1/M2a 已完成，M2b/M3/M3.5/P5-lite 主体落地；`cargo test --workspace`
**292/292** 通过（复审工作树），clippy/fmt/行数门禁全绿。

**已进入用户路径：**

- 词库：luna_pinyin 70,055 条 + `essay.txt` preset 编码后经 OpenCC t2s
  **427,661 简体词条 / 293,168 keys / 9.4 MB**；`.ior` v2 加载 + 多词典
  叠加 + 93 词内置兜底；
- 拼式：425 音节、查询侧 8 组模糊音、简拼、自然碼/小鶴/微軟三套真实双拼
  （`config.toml` 的 `engine.double_pinyin` 选择）；
- 解码：装有 `core.gram` 时 ≥3 音节走 beam 整句；简拼/短输入/无模型回退
  三步拼装（整词 → 用户词 → 逐音节拼字），常规步骤无候选时再做不完整
  拼写前缀回退（`nih` → 你会/你好）；
- 用户词典：快照 + append-log + 压实 + 30 天半衰期衰减 + TSV 导入导出，
  addon 四路学习均落盘；
- 交互：core 分页 / PageUp/Down / 候选索引直选、Shift 三态、中文标点、
  FFI v2（18 符号），GTK/Qt/XIM 真机回归通过；
- 配置：`$XDG_CONFIG_HOME/fcitx5/ior/config.toml`（自研 TOML 子集，零依赖），
  addon 启动时映射到 `EngineConfig`。

**已接线、仍有质量/配置缺口：**

- 整句解码：字符 trigram `.gram` + beam search 已实现并有 bench/双端互锁；
  `Session` 可装 LM、addon 会从数据目录加载 `core.gram`（缺失/损坏自动回退），
  安装脚本已训练并安装词表级模型；真实语料模型与准确率基线仍待补；
- 配置的 `[switches]` / `[punct]` 只解析占位，未作用到 session；无热重载、
  无配置 UI（`Configurable=False`）。

**仍缺的结构性能力：**

1. **数据质量**：42.77 万条解决了规模，但排序仍以词频为主；LM 由词表内字符
   n-gram 合成，并非真实语料训练，也没有长句正确率基线；
2. **拼式深度**：声调输入、spelling algebra、custom 双拼的宿主装表，
   以及双拼方案的运行时热切换；
3. **宿主隔离**：单引擎实例跨 IC 共享（多应用串扰），无状态区；
4. **交互补齐**：Enter 行为可配、以词定字、符号面板、v 模式、日期/中文数字、
   联想；
5. **用户词典自动化**：整句回填/自动造词待实现；
6. **全半角与运行时简繁**：`ascii_punct`、full shape、运行时简繁切换；
   编译期简体（OpenCC t2s）已落地。

## 2. 差距对照表

| # | Rime 能力域 | Rime 实现 | ior 现状 | 差距等级 |
|---|-------------|-----------|----------|----------|
| 1 | 词库规模与加载 | `dict.yaml` → `table.bin`（Trie 索引，10 万~百万级） | luna + essay 编译期简体化后 42.77 万条 / 9.4 MB；排序仍以词频为主，无语料 LM | 🟡 中 |
| 2 | 整句解码 | poet beam + 八股文 n-gram（gram） | trigram + beam + `Session`/addon 加载链路已通；无语料模型、无准确率基线 | 🟡 大 |
| 3 | 拼式代数 | spelling algebra（prism），双拼/简拼/模糊音全部运行时数据化 | 模糊音/简拼/三套双拼已落地；无声调、无 spelling algebra，custom 仍需宿主装表 | 🟡 中 |
| 4 | 配置/方案体系 | YAML + schema + custom patch + 部署/重编译 | `ior-config` TOML/schema/patch 已接线启动；switches/punct 未生效、无热重载/UI | 🟡 中 |
| 5 | 用户词典 | LevelDB userdb：持久化、词频自适应、自动造词（句子回填）、云 sync | 快照/日志/衰减/压实/TSV 已落地；自动造词待实现 | 🟢 小 |
| 6 | 中英/全半角/标点/简繁 | switches + punct 翻译器 + OpenCC | 中文标点 + 引号配对、编译期简体（OpenCC t2s）已落地；全半角、ascii_punct、运行时简繁切换缺 | 🟡 中 |
| 7 | 候选交互 | 翻页键可配、以词定字、长词优先、逐键提示、Enter 上屏原码、光标移动 | 分页/翻页/索引直选已落地；Enter 可配、以词定字、光标/长词优先/逐键提示缺 | 🟡 中 |
| 8 | 宿主集成 | per-IC 状态、状态区、per-app、配置 UI、quickphrase | 单引擎实例共享所有 IC（M0 已知串扰风险）、无状态区、Configurable=False | 🟡 中 |
| 9 | 特殊输入 | 符号面板 /fh、日期时间、中文数字、u 反查、emoji | 全部缺失（M4 计划中） | 🟢 小 |
| 10 | 预测/联想 | predict 插件 + userdb 共现 | 无 | 🟢 小 |
| 11 | 码表方案（五笔/仓颉） | table_translator 通用引擎 | 不存在（也未列入里程碑） | 🟢 按需 |
| 12 | 扩展生态 | librime-lua、插件（Processor/Segmentor/Translator/Filter 四类） | 无（与本项目纯函数式/零依赖约束冲突，建议不照抄） | 🟢 按需 |
| 13 | 工程体系 | 部署工具链、跨宿主（ibus/libime 复用） | FFI v2（18 符号）、CI、bench、行数门禁已落地；fuzz/第二宿主未做 | 🟢 小 |

**ior 的三个反超点，补齐过程中必须保持：**

1. 纯 Rust 零依赖，单文件分发（`.a`/`.so` 即全部）；
2. 引擎无 I/O、纯函数，可 headless 测试；
3. C ABI 明确版本化（`ior_engine_abi_version`）。

## 3. 补齐规划 P0–P6

与既有里程碑对齐：P0=M1、P1=M2a、P2=M2b、P3=M3、P4=M3.5、P5=M4、
P6=M4+/M5。

### P0（=M1）词库规模与二进制加载

- ✅ `.ior` **v2 格式**：key 索引 + 长度前缀、freq 变长整数；零依赖、无
  `unsafe`；读取端兼容 v1，损坏输入报偏移与条目上下文。
- ✅ engine-core：`Dictionary::from_ior_bytes` + **多词典叠加**
  （base + 领域 + 用户）。
- ✅ 数据：rime `luna_pinyin`（70,055 条）+ `essay.txt` preset（442,685 短语）
  编译并做 OpenCC t2s 后为 **427,661 词条 / 293,168 keys / 9.4 MB**；来源与
  LGPL-3.0 保留在文件头，转换表为 Apache-2.0（见 `data/README.md`）。
- ✅ `compile --simplify`：最长短语优先 → 单字表回退，取每行首值，等价于
  OpenCC `t2s.json`；安装脚本默认启用。简化后同一 `(key, hanzi)` 合并，
  词条数由 429,870 降至 427,661。
- ✅ dict-tools：compile 自动识别 ior TSV / rime `dict.yaml`，
  validate/inspect/export/merge/train/lminspect 齐备；bench 实测 10 万词编译
  125 ms、加载中位 26.7 ms。
- ✅ 验收：`tianqi→天气`、`jintian→今天`、`nihao→你好`、`zhongguo→中国`
  真机回归通过；`ior-bench` 已接入 CI。

### P1（=M2a）拼式管线数据化

- 模糊音从「烘焙进 key」改为**查询侧拼式变换管线**（`config.rs` →
  `spelling.rs`，≤400 行）。✅ 已落地：词典 key 只保留全拼规范形，
  `decoder::candidates_with` 经 `key_variants` 逐变体探测、同词条取
  最高频不叠加；`candidates` 旧签名保留为无模糊薄封装。
- **简拼**：声母首字母成段，优先级低于全拼。✅ 已落地：
  `dict` 内建首字母索引（`lookup_initials`）+ `spelling::initials_prefix`
  判定，纯声母串（≥2 字母、无分隔符）在常规候选之后追加（每 key 1 条，
  freq 降序）。
- **不完整输入前缀回退**：✅ 已落地。`prefix_index` 按去 `'` 平铺拼写排序，
  常规解码无候选时二分查找前缀，每 key 取最高频词条；仅作为兜底，完整
  输入顺序不变（`n` → 你、`nih` → 你会/你好）。
- **双拼**映射表数据化（自然码/小鹤/微软）。✅ 框架已落地：
  `double_pinyin.rs`（`DpTable` + `parse_tsv` + `to_full`，产出音节逐一
  过 `syllable::is_syllable` 校验）+ `config::DoublePinyin` 开关 +
  `session::set_double_pinyin`（解码用变换结果、preedit 仍显原串、学习
  落在变换后 key）。✅ **数据已导入**：`data/double-pinyin/{natural,flypy,mspy}.tsv`
  由 rime schema 的 `algebra` 规则求值生成，`include_str!` 内置；三张表对 425
  音节逐一回环校验：小鶴的 `lvan`、微軟的 `eh` 无法用两键表达，作为缺口
  显式登记；`lue`/`lve`、`luo`/`lo` 与自然碼/微軟的 `lvan`/`luan` 为方案
  固有碰撞。
  ⚠️ `Custom` 仍依赖宿主装表，addon 尚未提供表路径。
- 依赖：P0。

### P2（=M2b）整句解码 + LM

- ✅ `.gram` v1 二进制 n-gram（trigram + Stupid Backoff，varint 计数）：训练为
  dict-tools `train`（词表内**字符级** n-gram、按词频加权——离线仅词表可作语料；
  语料级模型待真实语料数据），`lminspect` 校验读取；engine-core `lm::from_gram_bytes`
  零 I/O 解析（规范见 `engine-core/src/lm.rs` 模块文档，双端 golden bytes 互锁）。
- ✅ beam search（`beam.rs`/`beam_search.rs`）：lattice + bucket 搜索接入
  `decoder::candidates_with_lm`；无模型/简拼/短输入自动回退旧三步拼装，结果尾部
  合并旧候选（任何输入不劣于 M0）。`Session` 已加 `lm: Option<GramModel>` 与
  `set_lm`，`session::candidates` 有模型时走 `candidates_with_lm`；addon 启动
  时按 `core.gram`（用户目录优先，其次系统目录）加载，失败自动回退。
- ✅ 排序统一：LM 分（backoff 0.4/级 + OOV 伪计数）+ 词频 + 用户词频 + 整句完成加成
  （`BeamParams` 可调，未动 `EngineConfig`）。
- ⏳ 验收「长句整句正确率基线报表」待真实词库/语料（当前仅 bench fixture：
  8 音节 P50 ≈ 107 µs、今天天气不错很好 胜出）。
- ✅ 模型安装：`scripts/install-core-dict.sh` 从已编译 `core.ior` 导出后训练
  `core.gram` 并随词典安装；全量计数超 `u32` 时 writer 统一缩放，reader 严格校验。
- ⏳ **质量缺口**：目前仍是词表内字符 n-gram；真实语料训练与长句准确率
  基线未开始。

### P3（=M3）用户词典持久化 + 自动造词（可与 P2 并行）

- ✅ 自研 **append-only log + snapshot compaction**（路径由 addon 层传入，
  core 只收/回 bytes，无 I/O），四路学习真机落盘并跨重启。
- ✅ **时间衰减**：频率 × 半衰期（addon 按 mtime 差计算 30 天半衰期）。
- ⏳ **自动造词**：整句上屏回填 userdb，待实现。
- ✅ 云备份 = TSV 导出/导入接口（core API 已备，暂无 UI/CLI 入口）。

### P4（=M3.5）配置/方案体系

- ✅ 自研**极简 TOML 子集 parser**（新 crate `ior-config`，不引第三方）。
- ✅ schema/默认值/patch 合并/校验 warning/规范化序列化；addon 启动时读取
  `$XDG_CONFIG_HOME/fcitx5/ior/config.toml` 并映射到 `EngineConfig`（非法即回退）。
- ⏳ **switches 热切**（ascii_mode / full_shape / ascii_punct / 简繁 / 双拼方案）：
  当前仅解析占位，未驱动 session。
- ⏳ **热部署** = addon inotify → 重建引擎实例；`Configurable=False`，无配置 UI。
- ✅ **user patch** 覆盖机制（core 已实现）。
- 依赖：P1（双拼方案需数据化映射）。

### P5（=M4）交互与宿主补齐

- ✅ core **分页状态**（page_size 生效）+ 翻页键路由 + 统一 addon/core 页
  大小 + 候选点击按索引直提。
- 🟡 **Shift 切中英**（三态可配）✅；Enter 行为可配 ⏳。
- 🟡 **标点系统**：中文标点 + 配对引号 ✅；全/半角、`ascii_punct`、自定义
  punct 表 ⏳。
- ⏳ 以词定字、`/fh` 符号面板、v 模式、日期/中文数字。
- ⏳ **per-IC session**（FFI 多引擎句柄）；fcitx5 状态区；Configurable=True。
- ⏳ 联想（P2/P3 之后）。

### P6（=M4+/M5）差异化与按需

- **简繁转换**：✅ 编译期 OpenCC t2s（vendored 数据，Apache-2.0，表驱动，
  运行时零依赖）；⏳ 运行时简繁切换。
- 评测体系进 **CI 门禁**。
- 反查 / table 码表**明确降级为 backlog**（ior 定位对标搜狗本地拼音，
  非全方案引擎）。
- 扩展点用 **Rust fn 注册** translator/filter 管线（组合而非 trait 对象），
  不做 Lua/WASM。

### 横切工程项

- **CI**：build / test / clippy / fmt + bench 回归。
- **ABI v2**：预留分页、多 IC、配置热更符号。
- 每阶段新文件 ≤400 行、迭代 ≤800 行；模块拆分（spelling / punct /
  userdb 各自独立）。

## 4. 优先级依赖图

```
P0 词库/加载 ──► P1 拼式管线 ──► P2 整句+LM ──► (P3 用户词典可与P2并行) ──► P5 联想
        └────────► P3 用户词典持久化 ──► P5 交互/宿主（分页/标点/Shift 可独立先行）
                     P4 配置体系 ◄── 依赖 P1（双拼方案需数据化映射）
```

要点：

- P0 是一切的地基（词库格式、加载、性能基线）；
- P3 不依赖 LM，可与 P2 并行推进；
- P5 中的分页/标点/Shift 属小工作量高收益项，可独立先行，不必等 P2。

## 5. 接下来三件事（2026-11-01 复审）

1. **P2 产品化**：安装脚本与 addon 加载链路已通，词表级 `core.gram` 已随装；
   下一步用真实语料重训、接入 CI 并建立长句正确率基线。
2. **per-IC 隔离**：把单引擎实例改成按 `InputContext` 建/销 session（FFI 可
   扩句柄），顺带补状态区；这是多应用日常使用的硬伤。
3. **配置闭环**：`[switches]` 驱动 full_shape / ascii_punct / 简繁 / 双拼热切，
   `[punct]` 接入 `punct.rs`，加 inotify 热重载与 `Configurable=True`
   （先 CLI 校验/生成，再谈配置 UI）。

## 6. 落地状态

| 项 | 状态（2026-11-01） |
|----|------|
| P0：`.ior` v2 加载 / compile / validate / bench | ✅ 已落地；luna 真实词库 + essay preset 经 OpenCC t2s 后 427,661 词条 / 9.4 MB（`docs/M2-VERIFY.md`） |
| P1：查询侧模糊音 / 简拼 / 三套内置双拼 | ✅ 已落地；自然碼/小鶴/微軟真实键位表 + `flypy` 配置别名 |
| P2：`.gram` n-gram + beam search | 🟡 算法/格式/bench/互锁 + 安装/加载链路 + 词表级模型已落地；真实语料模型与准确率基线待做 |
| P3：userdb 持久化（快照/日志/压实/衰减/TSV 备份） | ✅ 已落地并真机实测（四路落盘 + 跨重启）；整句自动造词待实现 |
| P4：`ior-config` TOML 子集 + schema + patch | 🟡 crate + 启动映射已落地；switches/punct 生效、热重载、配置 UI 待做 |
| P5-lite：core 分页 + Shift + 标点 + FFI v2（18 符号） | ✅ 已落地并真机回归（`docs/M1-VERIFY.md`）；P5 其余项见规划 |
| 横切：CI（fmt/clippy/test/行数门禁/bench 回归） | ✅ 已落地（`.github/workflows/ci.yml` + `scripts/check_line_limits.sh`） |
| P5 其余 / P6 各项 | 规划（以本文档为准，优先级见 §5/§7） |

## 7. 本轮复审结论

- **变化**：五个结构性缺口里，P0 数据、P1 拼式基础、P2 整句链路、P3 持久化、
  P4 配置基础已从 🔴 降级；核心矛盾从“没有能力”变成“质量、交互与部署方式”。
- **本质差距**：Rime 的强项不只在功能表，而在“部署管线”（schema/dict/preset
  → `table.bin` 可热编译）与“运行时拼写代数”；ior 已有 compile/train/install
  脚本和拼式管线，但改词库/模型仍需重编译重装、改方案仍需改配置并重启，
  尚未形成 Rime 式的部署/热编译链路。
- **质量风险**：整句已上生产路径（probe 长句正确），但 LM 由词表字符 n-gram
  合成、无真实语料与准确率基线；P3 自动造词、P5 联想都卡在这道验收上。
- **体验风险**：per-IC 串扰与 `Configurable=False` 是下一档“好用”的硬门槛；
  switches/punct/热重载不闭环，配置 crate 的价值只兑现了一半。
- **判断**：词级 + 整句已达“可用”；语料模型/评测、配置闭环、per-IC 是
  “好用”的三道门槛。
- **暂缓**：运行时简繁切换、符号面板、码表方案——先走完三道门槛，避免摊薄；
  编译期简体已随本轮落地。
