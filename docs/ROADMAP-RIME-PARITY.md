# ior vs Rime：差距评审与补齐规划

> 定位：ior 对标搜狗输入法的**本地拼音输入**能力，而非复刻 Rime 全方案引擎。
> 本文以 Rime 为参照系盘点差距，给出 P0–P6 补齐规划（对齐既有里程碑
> M1–M5），并标注应保持的反超点与明确不做的事。

## 1. 现状定位

M0 已打通**词级转换全链路**：

- 403 音节表 + 后缀 DP 全切分；
- 8 组单向模糊音；
- 93 词条内置词库；
- 3 步词级解码（整词 → 用户词 → 逐音节拼字）；
- 内存级自学习；
- fcitx5 薄壳（C ABI + `abi_version`），GTK/Qt/XIM 三前端实测通过；
- 78 测试全绿（`cargo test --workspace`）。

放到 Rime 的坐标系里：ior 目前相当于 `script_translator` **单方案、无部署、
无 LM** 的最小子集。差距不在「有没有」，而在五个**结构性缺口**：

1. **数据规模**：93 词 vs 十万~百万级；
2. **整句解码**：3 步拼装 vs beam + n-gram；
3. **拼式系统**：编译期烘焙模糊音 vs 运行时拼写代数；
4. **配置/方案体系**：Rust 结构体硬编码 vs schema + 部署/热编译；
5. **用户词典**：内存 HashMap vs 持久化 userdb。

## 2. 差距对照表

| # | Rime 能力域 | Rime 实现 | ior 现状 | 差距等级 |
|---|-------------|-----------|----------|----------|
| 1 | 词库规模与加载 | `dict.yaml` → `table.bin`（Trie 索引，10 万~百万级） | 93 词 TSV 编译期嵌入；engine-core 不能加载 `.ior` 二进制 | 🔴 致命 |
| 2 | 整句解码 | poet beam + 八股文 n-gram（gram） | 3 步拼装（整词 → 用户词 → 逐音节拼字），无 LM、无 beam | 🔴 致命 |
| 3 | 拼式代数 | spelling algebra（prism），双拼/简拼/模糊音全部运行时数据化 | 模糊音 8 组单向、编译期烘焙进词库 key；无简拼、无双拼 | 🔴 大 |
| 4 | 配置/方案体系 | YAML + schema + custom patch + 部署/重编译 | Rust 结构体硬编码，无文件配置、无 schema、无 switches、无热部署 | 🔴 大 |
| 5 | 用户词典 | LevelDB userdb：持久化、词频自适应、自动造词（句子回填）、云 sync | 内存 HashMap、count+1、无持久化/衰减/造词 | 🔴 大 |
| 6 | 中英/全半角/标点/简繁 | switches + punct 翻译器 + OpenCC | 全部缺失（大写字母透传是唯一中英切换） | 🟡 中 |
| 7 | 候选交互 | 翻页键可配、以词定字、长词优先、逐键提示、Enter 上屏原码、光标移动 | core 无分页状态（page_size 被无视）、addon 页大小 5 vs core 10 不一致、候选点击用合成数字键（翻页后会错选）、无光标/无以词定字 | 🟡 中 |
| 8 | 宿主集成 | per-IC 状态、状态区、per-app、配置 UI、quickphrase | 单引擎实例共享所有 IC（M0 已知串扰风险）、无状态区、Configurable=False | 🟡 中 |
| 9 | 特殊输入 | 符号面板 /fh、日期时间、中文数字、u 反查、emoji | 全部缺失（M4 计划中） | 🟢 小 |
| 10 | 预测/联想 | predict 插件 + userdb 共现 | 无 | 🟢 小 |
| 11 | 码表方案（五笔/仓颉） | table_translator 通用引擎 | 不存在（也未列入里程碑） | 🟢 按需 |
| 12 | 扩展生态 | librime-lua、插件（Processor/Segmentor/Translator/Filter 四类） | 无（与本项目纯函数式/零依赖约束冲突，建议不照抄） | 🟢 按需 |
| 13 | 工程体系 | 部署工具链、跨宿主（ibus/libime 复用） | FFI 9 符号 + abi_version 机制已备；无 bench/fuzz/CI | 🟡 中 |

**ior 的三个反超点，补齐过程中必须保持：**

1. 纯 Rust 零依赖，单文件分发（`.a`/`.so` 即全部）；
2. 引擎无 I/O、纯函数，可 headless 测试；
3. C ABI 明确版本化（`ior_engine_abi_version`）。

## 3. 补齐规划 P0–P6

与既有里程碑对齐：P0=M1、P1=M2a、P2=M2b、P3=M3、P4=M3.5、P5=M4、
P6=M4+/M5。

### P0（=M1）词库规模与二进制加载

- `.ior` **v2 格式**：排序 key 分桶/长度前缀、freq 变长整数；零依赖、
  无 `unsafe`。
- engine-core：`Dictionary::from_ior_bytes` + **多词典叠加**
  （base + 领域 + 用户）。
- 数据：导入 rime `luna_pinyin`（许可允许）到 **10 万级**，文件头注明来源
  与许可证。
- dict-tools：增强 compile/validate 与大库性能目标：**10 万词编译 <1s、
  查询 <10µs**。
- 验收：nihao/xian 候选质量提升 + **bench crate**（自研计时，不引
  criterion）。

### P1（=M2a）拼式管线数据化

- 模糊音从「烘焙进 key」改为**查询侧拼式变换管线**（`config.rs` →
  `spelling.rs`，≤400 行）。✅ 已落地：词典 key 只保留全拼规范形，
  `decoder::candidates_with` 经 `key_variants` 逐变体探测、同词条取
  最高频不叠加；`candidates` 旧签名保留为无模糊薄封装。
- **简拼**：声母首字母成段，优先级低于全拼。✅ 已落地：
  `dict` 内建首字母索引（`lookup_initials`）+ `spelling::initials_prefix`
  判定，纯声母串（≥2 字母、无分隔符）在常规候选之后追加（每 key 1 条，
  freq 降序）。
- **双拼**映射表数据化（自然码/小鹤/微软）。✅ 框架已落地：
  `double_pinyin.rs`（`DpTable` + `parse_tsv` + `to_full`，产出音节逐一
  过 `syllable::is_syllable` 校验）+ `config::DoublePinyin` 开关 +
  `session::set_double_pinyin`（解码用变换结果、preedit 仍显原串、学习
  落在变换后 key）。⚠️ **数据缺口**：真实键位表（自然码/小鹤/微软）是
  事实数据，网络不可用期间禁止凭记忆编造；待网络恢复后导入 `data/`，
  经同一 `parse_tsv` 加载即可（宿主层负责装表）。
- 依赖：P0。

### P2（=M2b）整句解码 + LM

- `.gram` 二进制 n-gram（trigram + backoff，量化）；训练脚本放
  dict-tools 或 lm-tools。
- **beam search** 替换 `decoder.rs` 三步拼装。
- 排序统一为：**LM 分 + 词频 + 用户词频 + 码完整度**。
- 验收：长句整句正确率基线报表。

### P3（=M3）用户词典持久化 + 自动造词（可与 P2 并行）

- 自研 **append-only log + snapshot compaction**（路径由 addon 层传入，
  core 只收/回 bytes，无 I/O）。
- **时间衰减**：频率 × 半衰期。
- **自动造词**：整句上屏回填 userdb。
- 云备份 = TSV 导出/导入接口。

### P4（=M3.5）配置/方案体系

- 自研**极简 TOML 子集 parser**（新 crate `ior-config`，不引第三方）。
- **schema = 配置组合**；switches（ascii_mode / full_shape / ascii_punct /
  简繁 / 双拼方案）热切。
- **热部署** = addon inotify → 重建引擎实例。
- **user patch** 覆盖机制。
- 依赖：P1（双拼方案需数据化映射）。

### P5（=M4）交互与宿主补齐

- core **分页状态**（page_size 生效）+ 翻页键路由 + 统一 addon/core 页
  大小 + 候选点击按索引直提。
- **Shift 切中英**（内嵌/提交/清码三态可配）、Enter 行为可配。
- **标点系统**：中英映射、全/半角、配对引号（新 `punct.rs`，数据驱动）。
- 以词定字、`/fh` 符号面板、v 模式、日期/中文数字。
- **per-IC session**（FFI 多引擎句柄）；fcitx5 状态区；Configurable=True。
- 联想（P3 之后）。

### P6（=M4+/M5）差异化与按需

- **简繁转换**：opencc 数据子集（Apache 许可，表驱动）。
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

## 5. 若只能先做三件事

1. **P0 二进制词库加载**——决定产品可用性；
2. **core 分页 + 标点 + Shift 切换**——日常体验从 demo 到能用，工作量小
   收益大；
3. **P3 用户词典持久化**——自学习是留存核心，不依赖 LM。

## 6. 落地状态

| 项 | 状态 |
|----|------|
| P0：`.ior` v2 加载 / compile / validate / bench | 本次迭代部分落地，详见 CHANGELOG [Unreleased] |
| P5：core 分页 + Shift + 标点 + FFI v2 | 本次迭代部分落地，详见 CHANGELOG [Unreleased] |
| P3：userdb 持久化 | 本次迭代部分落地，详见 CHANGELOG [Unreleased] |
| 其余 P1 / P2 / P4 及 P5 / P6 各项 | 规划（以本文档为准） |
