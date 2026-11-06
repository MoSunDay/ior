# Changelog

本项目所有显著变更记录于此。格式参考 Keep a Changelog，版本遵循 SemVer。

## [Unreleased]

### fix(engine)：空格上屏当前页首选——翻页后不再选回第一页的词

- 空格（及 `enter_action = candidate` 的回车）上屏的"首选"从扁平列表
  第 0 个改为**当前显示页的第 1 个候选**，与数字选字 / 鼠标点选的
  页内寻址一致（`session::handle_commit_or_passthrough` 此前忽略
  `s.page`）。页 0 行为不变；详见
  `features/changelog/2026-11-06/space-commits-current-page-top.md`。

### fix(voice)：语音"关不掉"两根因——toggle 热键自动重复抖动 + 非法热键静默回退默认

- **根因一（主症状：取消后又在录音）**：toggle 模式下热键无自动重复
  防护——X11 按住 `ctrl+grave` 会连发 press（其间无 release），每次
  press 都执行 toggle（录音→取消→录音→…），松手时相位取决于重复
  次数奇偶：想取消反而大概率停在录音中。hold 模式本有 `state==0`
  守卫，toggle 没有。修复：薄壳 `handleVoiceHotkey` 增加按下/释放
  配对（`voiceHotkeyDown_`，与 Shift 切换同法）——配对中的 press 视
  为重复键吞掉；该键位的任何 release（含修饰键已不匹配的，如先松
  Ctrl）都复位配对标志，`reset()`（焦点切换）兜底防漏 release 卡死。
- **根因二（配置关不掉）**：`voice.hotkey` 解析失败时静默回退
  `ctrl+grave`（旧行为"宁可回默认不可静默关"），写 `"none"`/`"off"`
  等值的用户永远关不掉热键。修复：`hotkey = "none"`（大小写/空白
  不敏感）显式关闭热键；解析失败也关闭热键并打 journal 诊断
  （提示正确写法）。`voice_state::hotkey()` 以 `keysym != 0` 为门，
  keysym 0 永不匹配真实按键。`voice.enabled = false` 整体关闭语义
  不变（已有回归覆盖）。
- **部署**：lib 已重装并 `fcitx5 --replace`，运行进程经 /proc maps
  确认映射新库，journal 无 ior 告警。按键序列（按住热键松手只切
  一次、再按即取消、Ctrl 先松后下一按仍生效、`hotkey="none"` 后
  ctrl+grave 透传）**待人工验证**——headless 无法合成 X11 重复键流
  与修饰键释放次序（评审跟进：初稿曾声明已验证，降级为事实）。
- **评审跟进**：二轮评审发现 (1) 修饰键先松的 release 在 mod 检查
  早退、执行不到标志复位，同焦点内热键会被永久吞掉——已改为该键
  位任何 release 先复位；(2) 真机行为声明失实——已降级。
- 测试：voice_settings 新增 3 例（`none` 三态、非法 spec 不再复活
  默认热键、正常 spec 仍解析）；voice_state 补 `hotkey()` 死键分支
  单测；384 tests / clippy / fmt / 行数门禁全绿。

### Added
- 语音转写简体化：新增零依赖 `t2s` crate（OpenCC t2s 表自 `dict-tools`
  提炼共享），`voice_worker` 在转写后统一转简体再上屏/润色——修复
  whisper-small `language=zh` 输出繁体问题。dict-tools 改薄 re-export，
  `compile --simplify` 行为不变。

### Fixed
- `deploy-asr-node02.sh` 改装 `vllm[audio]>=0.11`：此前缺 audio extra
  （soundfile ImportError）导致 node02 转写空响应且单序列阻塞。

### feat(engine)：组句暂存——span 选字不再逐字上屏，选完整段才整词输出并学习（造词闭环）

- **行为变化**：翻页选中只覆盖部分拼音的候选（如 `panjie` 选「盼」），
  汉字进入暂存缓冲而非立即上屏（preedit 显示 `盼jie`）；剩余段选完
  （选「姐」）后一次性输出整词「盼姐」。中文/英文标点、Enter 原码、
  Shift 切换的带出行为同步适配（标点带出暂存前缀、Enter 输出
  `盼jie`、Clear 全部丢弃）。
- **造词闭环**：整段选完时按**组合键**学习（暂存 `pan` + 末段 `jie`
  → `pan'jie → 盼姐`），单字仍按各自段键学习（`pan→盼`、`jie→姐`
  保持单字频次排序）。下次输入 `panjie`，「盼姐」直达候选首两页；
  `candidate_delete` 删词键与之严格镜像（整词删 `pan'jie`、
  span 删 span 键）。
- **退格**：剩余拼音删空后，下一次退格撤销最近的暂存选字（段键回到
  raw 缓冲可重选；双拼模式下回退为全拼字母）。
- **ABI 不变**：`ior_engine_select` 对暂存中的 span 点击返回 `""`
  （宿主空串守卫已有）；`process_key` span 数字键返回 1（编辑中）。
- 测试：新增 `sentence_staging.rs` 6 例（组合提交/组合学习、退格撤销、
  Enter/Shift/标点适配、删词镜像）；既有 span 测试改写为暂存语义。
  377 tests / clippy / fmt 全绿。
### fix(engine)：Shift+`shift_action="temp"` 不再静默丢弃暂存选字（复核发现）

- 组句暂存改动引入的回归：切英文时若剩余输入的首候选是 span，旧逻辑
  只暂存不提交，模式切换尾部的清空把暂存汉字与剩余拼音一并丢弃——
  整段输入无声消失。现该分支携带已暂存汉字上屏（span 键学习保留）。
- 测试：`sentence_staging` 新增 CommitBest+span 回归用例；378 tests /
  clippy / fmt 全绿。



### fix(engine)：Shift 切中英不再误上屏——默认丢弃未上屏拼音 + 点选残留 commit 不再被下一次切换带出

- **症状**：中文模式下打了半截拼音（如 `chinses`），按 Shift 切换
  输入方式时，半截拼音被原样上屏"带出来"；点选候选词后按 Shift 还会
  把刚才点选的词再上屏一次。
- **默认行为修正**：`engine.shift_action` 默认 `commit`（原样上屏）→
  `none`（丢弃）。单独按 Shift 只切模式、绝不上屏半截拼音；要原码
  上屏仍可用 Enter（`enter_action = "raw"`）或显式配置
  `shift_action = "commit" | "temp"`。engine-core `ShiftAction` 默认
  同步 `CommitRaw` → `Clear`（ior-config 与 engine-core 两处镜像默认）。
- **FFI 残留 commit 修复**：`ior_engine_select`（候选窗点击）原来把
  commit 写入一次性邮箱却由宿主经返回值上屏、从未 `take_commit`，
  下一次 Shift 切换的 `take_commit` 会把旧词再上屏一遍。现 select
  返回前即消费邮箱，返回值是该 commit 的唯一句柄（ABI 不变）。
- **候选窗"Chinese/English"提示残留修复（真机二次复现根因）**：
  薄壳 `showHint`（中英切换的 "Chinese"/"English" 面板提示）写入的
  aux 行在开始打字后不被清除——`updateUI` 组态分支从不清理 aux、
  1 秒定时器在组态中又跳过重置，导致整个输入过程候选窗顶部挂着
  "Chinese" 字样，看起来像输入内容被加了前缀。现 `updateUI` 每次刷新
  先清 aux（删除自造词模式除外），定时器到期也仅撤提示行、保留组态
  内容；提示保留"下一键或 1 秒先到者"即消失的经典行为。真机验证：
  切换后 0.1s 截图提示在，1s 内打字后截图仅剩 preedit+候选、无提示。
- 测试：默认丢弃断言（engine_state/ffi/paging_punct/model 四层）、
  点选后切换无残留上屏回归用例；`commit`/`temp` 行为保留原覆盖。

### feat(userdb)：候选窗删除自造词（悬停高亮 + 🗑 进入删除模式 + 点击删除）；选词持久化四路复核

- **选词持久化（既有能力，本轮复核为四路闭环）**：数字/空格/Enter/
  标点等键盘提交、候选窗鼠标点选、Shift 切换附带提交与析构兜底——每次
  提交即学习并立即快照落盘（`$XDG_DATA_HOME/fcitx5/ior/userdb.tsv`，
  保存即压实），启动加载 + mtime 差 30 天半衰衰减，跨重启生效。
- **engine-core**：`user_dict::forget`（删单条学习记录：键归一化、
  空键回收、保序）；`session::delete_flat` + `DeleteOutcome`——删除键
  与 `commit_candidate` 的学习键严格镜像（前缀 span 候选删 span 键、
  整句/全输入候选删 normalized decode 键），组合原样保留，删后候选即时
  失去自学习加权；词库原词保留，非自造词返回 `NotUserWord`。
- **ABI v2 纯增补**：`ior_engine_candidate_delete(engine,
  page_local_index)`（ffi_userdict.rs，第 32 个 `ior_` 符号；0 已删 /
  1 非自造词 / 2 越界 / -1 null，容忍 null engine）。
- **C++ 薄壳**：状态区新增 `ior-delword` 动作（fcitx5 托盘图标菜单
  Del word；classicui 的状态区动作渲染在托盘而非候选窗）。点击进入一次性删除模式：候选窗 aux 行提示，
  鼠标悬停高亮为 classicui 原生；点击要删的候选即删并立即
  `userdb_save` 落盘，Esc（仅退出模式、组合不丢）/ 任意按键 / 再点
  动作退出。
- **平台约束记录**（fcitx5 5.1.7 classicui 源码复核）：候选 hover 无
  IME 回调、仅左键点击转发为 `CandidateWord::select`，故「悬停删除」
  以「悬停高亮 + 角标进入删除模式 + 点词删除」实现，全部走 stock
  fcitx5 API。
- 顺带修复在途全角代码的 clippy/fmt 违规（width.rs 文档续行缩进、
  ffi_width.rs 冗余指针转换）。

### chore(data)：移除 rime 原始件，只保留 ior 格式

- 删除上游双拼 schema 原文 `data/double-pinyin/rime/` 与转换脚本
  `scripts/convert-rime-double-pinyin.py`：ior 键位表 `*.tsv` 已入库，
  上游出处/许可保留在表头与 `data/README.md`。
- 删除 `docs/ROADMAP-RIME-PARITY.md` 及 README/代码注释中的引用；
  README 里程碑表的 (=P0)–(=P6) 对照同步移除。
- 词库源 `data/dicts/luna_pinyin.dict.yaml` 与 `essay.txt` 同步删除：预编译
  产物 `core.ior`（9.0 MB）/ `core.gram`（5.9 MB）改为入库（`.gitignore` 加
  `!data/dicts/*` 例外），上游出处与许可（LGPL-3.0 衍生数据）保留在
  `data/README.md` 来源表。
- `scripts/install-core-dict.sh` 改为纯安装（复制入库产物，不再现场
  compile/train）；重建词库需自行获取上游 rime 词库后用 dict-tools 编译。

### fix(fcitx5)：Shift 中英切换最终修复——AltTrigger 先于引擎派发（实证）

- 日志实证（引擎入口打点 + journal）：清空前 Shift_L **从未到达**
  引擎 `keyEvent`（普通键全部到达）——fcitx5 的全局
  `Hotkey/AltTriggerKeys`（默认 `Shift_L`）在**引擎派发之前**消费
  Shift，此前"薄壳 filterAndAccept 抢先"的修复从未生效过。
- 最终修复：清空 `~/.config/fcitx5/config` 的 `[Hotkey/AltTriggerKeys]`
  （原文件备份为 `config.bak-ior`）。清空后 Shift 到达引擎，薄壳的
  释放切换 + 修饰检测逻辑即正常工作（journal 见
  `TOGGLE 0->1/1->0`；英文模式字母 Forward，修饰用法不误切）。
- 引擎内保留 Shift 按下/释放消费逻辑（其他宿主/无 AltTrigger 环境
  仍有效），`ioraddon.cpp` 注释更新为实证结论。
- 注意：AltTriggerKeys 是**全局**配置，影响所有输入法的 Shift
  切换；改用 Ctrl+Space 等主触发键或托盘切换输入法。

### fix(engine)：默认半角符号 + 全角开关与标点解耦

- 用户反馈：托盘切到半角后 `，。` 仍是全角——中文标点映射默认
  `chinese_punct = true` 抢了标点。现默认 **false（半角符号）**，
  engine-core 与 ior-config 两处默认同步；要中文标点显式配置
  `[engine] chinese_punct = true`。
- 全角开关不再碰标点（对齐搜狗：半/全角与中/英标点是两个独立开关）：
  `width::intercept` 只认字母/数字/空格，标点一律走 `chinese_punct`
  （默认半角透传）；C++ idle gate 的全角放行同步去掉标点项。
- 引擎侧已有行为不变：显式 `chinese_punct = true` 时标点映射与
  引号交替、候选优先上屏等语义保持（测试改为显式 opt-in）。

### feat(engine)：全角/半角可配 + 状态区（右上角）切换，默认半角

- 配置：`[engine] full_width`（bool，默认 `false` 半角），engine-core
  `EngineConfig`/`Session` 与 ior-config schema 同步；运行期经
  `ior_engine_full_width` / `ior_engine_set_full_width`（ABI v2 增补）
  即时切换，不动未上屏编码。
- 语义（对齐搜狗/rime 的宽度开关）：全角时空闲 ASCII 键上屏全角形——
  中文模式数字、空格（`１２３`、全角空格 `U+3000`）与大写字母，
  拼音组码与中文标点映射不受影响；英文模式全部可打印 ASCII
  （`ａｂｃ１２３！`）＋全角空格。组合中的数字选词、空格上屏优先级
  不变；半角为纯透传（零行为差异）。
- C++ 薄壳：注册状态区动作 `ior-fullwidth`（**托盘图标右键菜单 +
  托盘图标菜单**，fcitx5 `SimpleAction` 勾选式），`activate()` 时重挂
  状态区；idle gate 放行全角模式下的数字/空格/标点到引擎；切换后
  面板短暂显示 Full-width / Half-width（复用 1s 一次性清除 timer，
  提示函数泛化为 `showHint(ic, text)`）。
- 引擎侧新增 `engine-core/src/width.rs`（纯函数映射 + 接管判定 +
  5 组单测：半角透传、英文全角、中文数字/空格、组合中语义保持、
  往返设置）；ior-config 增补类型校验用例。

### fix(fcitx5)：Shift 中英切换被 AltTrigger 抢键 + 模式提示

- 症状：ior 模式下按 Shift 不切中英文，而是整个输入法被切走。
  根因：C++ 薄壳只记录 Shift 按下而不消费，fcitx5 全局默认
  `Hotkey/AltTriggerKeys=Shift_L` 抢走该按下事件直接切换输入法，
  引擎的「释放时切换」永远等不到 release。
- 修复：薄壳对 Shift_L/Shift_R **按下 filterAndAccept、释放 filter**
  完全接管；释放时（且未被用作修饰键）经 `ior_engine_set_ascii_mode`
  切换中/英，与 rime 同键位行为一致（仍不透传给 `process_key`，
  两种机制择一）。
- 切换成功后面板短暂显示 `English` / `Chinese` 提示（1s 一次性定时
  清除；经 `ic->watch()` 防悬垂，组合中不覆盖候选窗）。
- `IorEngine` 保存 `eventLoop_` 指针用于该一次性定时器。

### 候选对齐 rime：前缀跨度瀑布 + 部分上屏 + 候选窗口扩容

- 逻辑对齐 rime `script_translator`：整词候选之后，按最优切分的**前缀
  跨度**递减（`k = len-1 .. 1`，长词优先）追加词条候选——`nihao` 在
  `你好` 等整词之后继续给出 `ni` 的同音字 你/拟/尼/泥/呢…，列表不再
  只有整词+1 条拼字结果；跨度组内用户词优先（与整词组同一规则）。
- **部分上屏**：选中只覆盖前缀的候选（含 beam 部分解）只上屏该词，
  未覆盖字母留在编辑区继续输入（选 你 后 preedit 剩 `hao`）；学习按
  跨度 key（`ni`）记录。覆盖前缀由候选 `pinyin` 字面匹配推导，
  简拼/前缀兜底等非字面 key 仍整体上屏，行为不变。
- 候选窗口：`FLAT_CANDIDATE_LIMIT` 10 → 50，`max_candidates` 默认
  10 → 50（engine-core 与 ior-config 两处默认同步，README 示例更新），
  page_size 5 下可翻 10 页，对齐 rime 菜单多页候选的观感。
- 新增 `crates/engine-core/tests/candidate_spans.rs`（跨度瀑布顺序、
  多音节跨度、部分上屏、边界分隔符吞并、跨度用户词优先、50 条窗口）。
### 翻页键可配：对齐 rime key_binder（FFI v2 增补，19/29 符号）

- 配置：`[engine] prev_page_keys`/`next_page_keys`（键名数组，默认
  `["Page_Up", "minus"]` / `["Page_Down", "equal"]`，即 rime luna_pinyin
  默认绑定；空数组可禁用该方向，未知键名忽略并告警）。
- 仅候选窗激活时生效（对齐 rime `when: paging`/`has_menu`）：空闲时
  -/= 等仍走中文标点或透传给应用，PageUp/Down 归属原键义。
- addon 新增 `keysym.rs`（键名 → X keysym 小表 + 解析/去重/方向判定，
  未知键名 stderr 告警忽略）。
- FFI 新增 `ior_engine_page_key_direction`（纯查询：-1 上一页 / 1 下一页
  / 0 非翻页键，null-safe，不改状态）。
- C++ 薄壳改用该符号路由翻页键（原硬编码 Page_Up/Page_Down），翻页
  保持 core 循环分页语义。

### 语音输入 V1：ctrl+` 按键说话 + node02 远端 Qwen ASR（+可选润色）

规划与验证记录见 `docs/NODE02-DEPLOY.md`、`docs/VOICE-VERIFY.md`。

#### 语音输入（voice crate，零第三方依赖）
- 热键解析：`voice::hotkey` 把 `"ctrl+grave"`（别名 `"ctrl+\`"`）等规格
  解析为 keysym + 与 fcitx5 `KeyStates::toInteger` 位对齐的修饰键掩码
  （Shift 1<<0 / Ctrl 1<<2 / Alt 1<<3 / Super 1<<6）。
- 采集：`capture_command = "auto"` 依次探测 pw-record → parec → arecord，
  输出 16 kHz s16le mono；亦可为任意 argv（测试用 `cat` 桩）。WAV 头按需
  写入/嗅探剥离（pw-record 自带头）。
- 网络：手写 http:// 客户端（分块重组、超时、Bearer 可选）、multipart 拼装、
  JSON 字符串转义/抽取（含 UTF-16 代理对），全程 `std` only。
- ASR：OpenAI 兼容 `POST /v1/audio/transcriptions`（`response_format=text`），
  默认 `http://node02:8000` + `Qwen/Qwen3-ASR-4B`；可选润色走
  `/v1/chat/completions` 抽 `content` 字段。
- 密钥只经环境变量（`IOR_ASR_API_KEY` / `IOR_LLM_API_KEY`），配置文件仅存
  变量名；endpoint 强制 `http://` 前缀（内网明文模型，见安全模型文档）。

#### fcitx5 addon 桥接（FFI v2 增补，18→28 符号，additive-only）
- 新增 `ior_engine_voice_{hotkey_keysym,hotkey_mods,hotkey_mode,hotkey_name,
  state,toggle,stop,cancel,poll,take_text}`（`ffi_voice.rs`，null-safe）。
- `voice_state.rs`（325 行）状态机 idle/1=recording/2=transcribing；
  `voice_settings.rs`（115 行，自 voice_state 拆出以满足新文件 ≤400 行）
  承载 `[voice]`/`[llm.asr]`/`[llm]` 快照；`voice_worker.rs` 后台线程
  采集→WAV→ASR→润色，经 channel 汇报，主线程 `poll` 排水。
- 双信号设计：`stop`（停录仍转写，hold 松手用）≠ `cancelled`（丢弃，
  toggle 二按用），修复了单标志版"松手即丢稿"的缺陷。
- C++ 薄壳（`ioraddon.cpp` 385 行）：`handleVoiceHotkey` 掩码比较（忽略
  NumLock 等无关位）、50 ms `EventLoop::addTimeEvent` 周期轮询
  （`setNextInterval` 续订）、`pollVoice` 完成文本直接 `commitString`
  （不经候选、不进用户词典）、录音/转写中面板显示状态提示并隐藏候选。

#### 配置（ior-config）
- 新 section：`[voice]`（enabled/hotkey/hotkey_mode/capture_command/
  sample_rate/max_duration_ms）与 `[llm.asr]`/`[llm]`（endpoint/model/
  api_key_env/language/timeout_ms），schema 全量校验 + 默认值。

#### 部署与文档
- `scripts/deploy-asr-node02.sh`：ssh 一键部署 vLLM ≥0.11（ModelScope 下载
  权重）+ systemd `ior-asr.service`（8000）/`ior-llm.service`（8001，
  `--with-llm`），无任何密钥写入。
- `docs/NODE02-DEPLOY.md`（端口/hosts/冒烟/安全模型/排障）、
  `docs/VOICE-VERIFY.md`（headless 记录 + 真机 xdotool 十项清单）、
  README 架构图/crates 表/配置示例/里程碑同步。

#### 回归
- `cargo test --workspace` 343/343（ior-config 38+1、voice 27、
  fcitx5-ior 43 等）；clippy `-D warnings`、fmt、行数门禁全绿；
  C++ 侧 cmake+ninja 构建通过，`libior.so` 导出 10 个 voice 符号。

#### 实测修正（2026-11-01，node02 真实部署）
- **vLLM JSON 信封兼容**：vLLM 0.11+ 对 `response_format=text` 实际
  返回 `{"text":...,"usage":...}`，`voice::asr::transcribe` 增加回退：
  响应以 `{` 开头时按 JSON 抽 `text` 字段；新增回归测试
  `transcribe_extracts_text_from_a_json_envelope`（voice crate 27→29）。
- `voice_state::take_text` 文档注释修正：缓冲区保留（返回借用切片），
  one-shot 交付由 `poll` 的 `has_text` 门控，非 take 清空语义。
- 部署脚本加固（`scripts/deploy-asr-node02.sh`）：pip 必须以 `vllm`
  用户安装（root 的 `~/.local` 对降权后不可见）；sudo `env_reset` 会
  剥掉环境变量 → wrapper 显式白名单转发 `VLLM_USE_MODELSCOPE`/
  `VLLM_API_KEY`；vLLM 启动时的 HF 在线存在性检查绕过
  `VLLM_USE_MODELSCOPE` → 预先 `modelscope snapshot_download` 到
  `/home/vllm/models/<name>` 并 serve 本地目录 + `--served-model-name`；
  新增 `--gpu-util`/`--asr-args`/`--pip-index` 旗标（whisper 需
  `max_num_batched_tokens ≥ 1500`）。
- **node02 实况**：8GB 4060 Ti 被 llama-server 常驻 ~6GB，默认
  Qwen3-ASR-4B 放不下 → 以 `openai-mirror/whisper-small`（0.17 gpu-util
  + eager + 448 ctx + 1 seq + 1600 batched tokens）落地，服务 active、
  4.2 s 中文音频端到端 ~2 s；润色可复用节点上 8081 口 llama-server
  （零新增显存）。whisper-small 对 espeak 机器音有幻觉（识别成字幕署名），
  管线正确、音源受限；完整记录见 `docs/NODE02-DEPLOY.md`（新增 8GB
  共享卡配方节）、`docs/VOICE-VERIFY.md`（新增实测结果节）。
- 宿主直驱 C ABI 验证：ctypes 按 thin-shell 相同调用序驱动
  `libior_engine.so`（null-safety×11、热键元数据、toggle→stop→poll→
  take_text、cancel 路径）全过，含真实 node02 端点的端到端转写。


### 前批：Rime 对标第一批（P0–P4 + preset vocabulary + LM）

规划全貌见 `docs/ROADMAP-RIME-PARITY.md`；本批覆盖 P0 / P5-lite / P3 / P1 / P2 / P4，并补齐 Rime preset vocabulary 的部署语义。

#### Enter 上屏原始拼音（2026-11-01）
- 新默认语义：预编辑激活时 Enter 把已输入的拼音原码原样上屏（含已敲入的
  `'` 分隔符），不进用户词典学习，可在中文模式下直接落拉丁字母；空格仍
  上屏首选汉字；预编辑为空时 Enter 透传给应用（换行不受影响）。
- 配置链路：`engine.enter_action = "raw" | "candidate"`（默认 `raw`；
  `candidate` 恢复旧行为=与空格一致上屏首选），贯穿 engine-core
  `EnterAction`（`session::handle_enter`）、ior-config model/schema
  （默认表新增该键）、fcitx5-addon `to_engine_config` 映射。
- 文档与路线图同步：README 配置示例、`docs/ROADMAP-RIME-PARITY.md`
  （「Enter 行为可配 / Enter 上屏原码」标记 ✅）、
  `docs/M0-FRONTEND-TEST.md`、`crates/fcitx5-addon/cpp/README.md`。
- 回归单测：`session_flow.rs` 新增 Enter 原码上屏 + 空预编辑透传、`'`
  分隔符原样保留、`candidate` 模式同空格三例；ior-config schema 增
  `enter_action` 非法值用例；`cargo test --workspace` 299/299 通过。
- 重装 `libior.so`（md5 `e7cdf52ab3cc9d2f5fbd353e423a70f6`）后 fcitx5
  真机复核：GTK3（zenity）`nihao`+Enter→`nihao`、`nihao`+空格→`你好`、
  `ni'hao`+Enter→`ni'hao`、`vk`+Enter→`vk`；XIM（xterm）单行组合
  `nihao`+Enter、`nihao`+空格 → `nihao你好`（行尾 Enter 透传结束行）；
  `engine.enter_action = "candidate"` 写入 config.toml 重启后
  `nihao`+Enter→`你好`，删配置重启还原默认。
- 复核方法备忘：`fcitx5-remote -s ior` 仅作用于当前聚焦 IC，自动化脚本
  必须在目标窗口聚焦注册 IC 之后再切换、并断言 `fcitx5-remote -n`＝`ior`
  （本机新窗口默认 IC 是 rime，其 Enter 同样上屏原码，不断言极易误判）。

#### 音节表补全与 houxuanci 回归（2026-11-01）
- `engine-core` 的 `SYLLABLES` 由 403 补到 425：依据 bundled luna_pinyin
  增补 22 个读音（`xuan`、`zen`、`kui`、`dei`、`nang`、`eng`、`lve`、`nun`、
  `nia`、`pia`、`zhei`、`sei`、`tei`、`biang`、`cei`、`din`、`fiao`、`fong`、
  `wong`、`yai`、`eh`、`lvan`），不发明数据。安装词库的 423 个去重 key
  token 已全部可切分；`houxuanci` 不再只能切成 `hou|xu|an|ci`。
- 回归单测：`segmentations("houxuanci")` 首选 `hou|xuan|ci` 且保留
  `hou|xu|an|ci`；`candidates_with` 即使面对更高频的 4 音节 `后续按次`，
  仍让 3 音节的 `候选词`（`hou'xuan'ci`）居首，并保证 `后续按次` 可达。
- 双拼表重新生成并保持幂等：小鶴的 `lvan`、微軟的 `eh` 无法用两键表达，
  转换脚本记录缺口到表头并继续；解码优先 `lve`/`nve` 规范拼写而非
  `lue`/`nue` 别名，`lvan`/`luan` 仍按字典序取常用 `luan`。回环单测显式
  登记 `lue`/`lve`、`luo`/`lo` 及自然碼/微軟 `lvan`/`luan` 碰撞；
  `cargo test --workspace` 296/296 通过。
- 重装 `libior.so`（md5 `1f78e8062e0dc22ecc4d553b3cea251f`）后 fcitx5
  真机复核：全拼 `houxuanci` 候选窗可见 `hou'xuan'ci` 与第 4 条「候选词」，
  `xuanze` + 空格落盘「选择」。

#### 候选窗 preedit 不再内联回显（2026-11-01）
- addon `updateUI` 由 `panel.setClientPreedit(...)` 改为 `panel.setPreedit(...)`：
  拼音原码只显示在候选窗顶部（候选上方），不再写入客户端文本域；空 preedit
  仍走 `panel.reset()`。
- GTK `zenity --entry` 真机 + OCR 复核：输入 `nihao` 时输入框保持占位符
  `type here`，`ni'hao` 与「你好 / 逆号 / 拟好」全部出现在候选窗内。
- 安装的 `/usr/lib/x86_64-linux-gnu/fcitx5/libior.so`
  md5 `8939ce133b11aae1476348c193444ce9`。

#### 默认简体与不完整输入前缀回退（2026-11-01）
- `dict-tools compile` 新增 `--simplify`：构建期用 vendored OpenCC ver.1.4.2
  t2s 表（`data/opencc/TSPhrases.txt` 最长短语优先 → `TSCharacters.txt`
  单字回退）转换候选汉字，再按简化后的 `(key, hanzi)` 重新归一化去重。
  引擎与运行时零 OpenCC 依赖；表为 Apache-2.0，来源与哈希见
  `data/opencc/README.md`。
- `scripts/install-core-dict.sh` 默认 `compile --simplify`。本轮改写 268,003
  词条，简化后同音同形合并使词条数 429,870 → 427,661（293,168 keys 不变），
  `.ior` 9,424,662 字节；`core.gram` 重新训练为 38,176 / 276,582 / 297,084
  unigram / bigram / trigram。真机 `tianqi/jintian/nihao/zhongguo` →
  天气 / 今天 / 你好 / 中国，`jintiantianqibucuo` → 今天天气不错。
- `engine-core` 新增按「去 `'` 平铺拼写」排序的前缀索引；当完整拼式、
  用户词、简拼等常规步骤均无候选时，`decoder` 用去分隔符的原始输入做前缀
  回退，每 key 取最高频词条。完整输入候选顺序不变；`n` → 你、`nih` →
  你会 / 你好、`nihao` 仍首选 你好。
- 验证：`cargo test --workspace` 292/292（新增简化、前缀与端到端用例），
  clippy / fmt / 行数门禁全绿；`engine-core/examples/probe` 加入
  `n/ni/nih/niha/nihao` 回归样例。

#### 品牌与第二轮复审（2026-11-01）
- 新增自有图标 `assets/ior.svg`（含 48×48 PNG 变体）；`inputmethod/ior.conf` 的占位
  `Icon=fcitx-pinyin` 改为 `Icon=ior`，CMake 安装到
  `share/icons/hicolor/scalable/apps/`（fcitx5 主题图标约定）。
- 复审 `docs/ROADMAP-RIME-PARITY.md`：刷新现状、13 项差距表、P0–P5 状态与
  下一批优先级（P2 质量收尾 / per-IC 隔离 / 配置闭环）；README 的 crates 表、
  里程碑与配置说明同步。

#### LM 接入与全量词库训练（2026-11-01）
- `Session` 新增 `lm: Option<GramModel>` 与 `set_lm`；`session::candidates` 在装有
  模型时走 `decoder::candidates_with_lm`，无模型保持原三步路径。
- addon 启动时按 `$XDG_DATA_HOME/fcitx5/ior/core.gram` → `/usr/share/fcitx5/ior/core.gram`
  加载模型（缺失静默跳过、损坏打印后回退），`new_engine_with_lm` 可供测试注入。
- 修复全量词库训练 `.gram` 的 `u32` 头溢出：raw 总权重超限时，writer 对所有计数
  统一除以同一除数（非零值保底 1），保证头与值区严格自洽；reader 的等值校验不变。
  luna+essay 计 38,979 unigram / 279,175 bigram / 297,665 trigram，raw 权重
  74,911,345,415 → 序列化头 4,161,752,858，文件 6,270,034 字节。
- `scripts/install-core-dict.sh` 从**已编译**的 `core.ior` 导出临时 TSV 后训练
  `core.gram` 并随词典安装，preset 短语一并进入模型；`.gram` 构建产物不入库。
- 真机整句：`jintiantianqibucuo␣` → 今天天氣不錯，`woxiangqubeijing␣` →
  我想去北京（`docs/M2-VERIFY.md`）。

#### P0 = M1：词库二进制与加载
- `.ior` 格式升级 **v2**：13 字节头 + key 索引区（varint 长度前缀、key 升序去重）+ 值区
  （freq 降序），freq 采用 LEB128 varint；读取端 v1/v2 双兼容，损坏输入报偏移与条目上下文。
- `engine-core`：`Dictionary::from_ior_bytes` / `from_ior_layers`（base→领域→用户多词典叠加，
  freq 求和）；core 自带最小 reader，仍零依赖、无 unsafe；`builtin` TSV 保留为回退。
- `dict-tools`：新增 `compile`（ior TSV / rime dict.yaml 表自动识别 → .ior v2）与
  `validate`（结构校验，`--dump-syllables` 导出音节清单）；新增 `source.rs` / `varint.rs` 模块。
- 新增 `ior-bench` crate（自研 Instant 计时，不引 criterion）：10 万词编译 **125 ms**
  （目标 <1s）、加载中位 26.7 ms、lookup **P99 0.39 µs**（目标 <10 µs）。
- `data/README.md`：v2 字节布局规范与 rime 词库导入示例。

#### P5-lite = M4 先行项：分页 / Shift 中英 / 中文标点
- `engine-core`：`Session` 增加分页状态（`page_count/current_page/next_page/prev_page/
  select_flat`，`page_size` 真正生效，数字键按当前页偏移选词，改码页归零）；
  `process_shift` 中英切换（`ShiftAction::{CommitRaw,CommitBest,Clear}` 三态可配）；
  新增 `punct.rs` 数据驱动中文标点（含 “” ‘’ 配对交替、全角映射，`chinese_punct` 开关）。
- FFI 升级 **ABI v2**（9→18 符号）：翻页 `page_next/prev/page_info`、候选点击改
  `ior_engine_select`（页内索引直提，修复翻页错位）、`ascii_mode/set_ascii_mode`、
  Shift/PageUp/PageDown keysym、userdb 三符号（见 P3）。
- C++ 薄壳：候选点击不再合成数字键；Page_Up/Down 翻页；Shift 释放且未作修饰时切换中英；
  候选列表页大小统一为引擎当前页（消除 5 vs 10 不一致）。

#### P3 = M3：用户词典持久化
- `engine-core`（无 I/O，只收/回 bytes）：`user_dict_store.rs` —— 快照
  `to_snapshot/from_snapshot`（`# ior userdb v1` TSV）、append-only 日志
  `log_line/replay`、压实 `compact`（幂等）、时间衰减 `decay`（整数次减半、零频清除）、
  云备份 `export_tsv/import_tsv`、合并 `absorb`。
- addon 层：`ior_engine_userdb_save/load/decay`；C++ 侧 userdb 路径
  `$XDG_DATA_HOME/fcitx5/ior/userdb.tsv`，启动按 mtime 差做 30 天半衰期衰减；
  全部四条学习路径（键盘提交/候选点选/Shift 切换附带提交/析构兜底）均落盘快照
  （快照即压实，失败静默不阻断输入）。

#### P1 = M2a：拼式管线数据化（模糊音查询化 + 简拼 + 双拼框架）
- 模糊音从「构建期烘焙进词典 key」改为**查询侧等价类展开**（`spelling.rs`：
  `syllable_variants`/`key_variants`，声母×韵母独立组合、跨变体取最高频不叠加）；
  词典 key 保持全拼规范形，`from_tsv`/`from_ior_bytes` 系列去模糊参数。
- 简拼：`Dictionary` 新增声母首字母 BTreeMap 索引 + `lookup_initials` 前缀查询，
  简拼候选排全拼之后、每 key 一条（`jt`→今天/`jin'tian`）。
- 双拼：`DoublePinyin{Off,Natural,Xiaohe,Mspy,Custom}`；`double_pinyin::builtin`
  用 `include_str!` 内置自然碼/小鶴/微軟三张真实表，`session::new_session_with`
  构造时自动装表；`Custom` 仍可加载宿主表，失败回退全拼。
- 配置：`ior-config` 同步五值枚举并接受 rime 别名 `flypy`（= 小鹤）。
- addon `candidates` 薄封装保持原签名，宿主零改动。

#### 数据导入（P0 收尾，网络恢复后落地）
- 词库：导入 rime **luna_pinyin**（LGPL-3.0，出处与许可保留在文件头），
  `compile` 产出 **70,055 词条 / 19,920 键**，`data/dicts/core.ior`（P0 时约 730 KB，
  接入 essay 后约 9.4 MB；按仓库策略不入库，`scripts/install-core-dict.sh` 一键编译安装）。
- `dict-tools` 权重解析增强：rime 词库的百分比权重（如 `99.93%`）按万分比
  整数化；零权重（`0`/`0%`）按 rime 语义视为**剔除条目**而非权重 1。
- 双拼真实键位表：新增 `scripts/convert-rime-double-pinyin.py`，对上游
  rime-double-pinyin schema 的 `algebra` 规则逐音节**求值**，转出自然碼/
  小鶴/微軟三张键位表（`data/double-pinyin/*.tsv`，GPL-3.0 出处注明），零冲突。
- `engine-core` 双拼框架扩展：支持多字母声母行（`zh/ch/sh<TAB>key`）、
  同键多零声母音节（自然碼 `e/ei/en/eng/er` 共享 e 键）、`;` 键
  （微軟 `ing=;`）；三张真实表对 404 音节逐码回环单测（仅 `lo`/`luo`
  方案固有碰撞，显式登记）。

#### P0 数据完整性：preset vocabulary（essay）部署语义
- 背景：`luna_pinyin.dict.yaml` 只提供字词与生僻词，常用短语（天氣/今天/你好/中國）
  在 Rime 部署期由 `essay.txt`（preset vocabulary）按字音编码，不写入 dict.yaml。
  旧 `tianqi→蚕戚` 并非 `.ior` 读取 bug：`parse_ior`/`dump_ior` 与 dict-tools
  `export` 逐字节一致；根因是缺少多字短语，旧候选由真实字频与缺词造成。
- `dict-tools/src/preset.rs`：解析 `phrase<TAB>weight`；按 Rime 语义只保留单字
  读音占比 ≥5% 的音，最长匹配 DFS、每短语最多 32 码组合、短语最长 32 字；
  显式词库短语优先，零/低于 `min_phrase_weight` 的短语丢弃；权重以
  `WEIGHT_SCALE=100` 定点化。
- `source.rs`：解析 `use_preset_vocabulary` / `vocabulary` / `max_phrase_length` /
  `min_phrase_weight`；启用 preset 时，缺失权重取 essay 权重，百分比按
  `essay_weight * pct / 100`（定点后等价于 `essay_weight * pct`）。
- CLI：`compile <in> <out.ior> [--vocabulary <path>]`；未显式指定时从输入同级
  目录读取 `essay.txt`，与源词条合并后统一排序去重。
- 数据：`data/dicts/essay.txt`（rime-essay，LGPL-3.0，442,685 短语），重编
  `core.ior` 为 293,168 keys / 429,870 词条 / 9,444,158 字节，compile 约 3.5 s；
  引擎确认 `tian'qi`→天氣、`jin'tian`→今天、`ni'hao`→你好、`zhong'guo`→中國。

#### P2 = M2b：整句解码（`.gram` n-gram 语言模型 + beam search）
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
  `session.rs` 增加可选 LM 字段与 `set_lm`，addon 启动加载 `core.gram` 并接入；
  模型缺失/损坏时自动回退旧路径。
- bench：新增场景 [4] 整句解码（dict-tools 训练 → engine-core beam 计时，兼作跨 crate
  格式互锁）与 `--help`；fixture 8 音节整句 P50 约 107 µs，top1 = 今天天气不错很好
  （LM 链式打分胜过 金田…）。`crates/bench/tests/gram_interlock.rs`：常量对齐、
  字节互认、整句端到端、坏模型拒收。

#### CI 与工程
- `.github/workflows/ci.yml`：fmt/clippy/test + 行数红线（400/800 规范脚本
  `scripts/check_line_limits.sh`）+ bench 参考性回归（P0 基线门禁不阻断）。

#### P4 = M3.5：配置体系第一步（新 crate `ior-config`）
- **TOML 子集解析器**（零第三方依赖、无 unsafe、纯函数）：`[section]`/`[a.b]`
  表头、`key = value`（含点分键）、基本字符串（`\" \\ \n \t \u{XXXX}`）、
  十进制整数、布尔、单行数组（嵌套一层）、`#` 注释；解析失败一律返回带
  1-based 行/列的 `ParseError`（`line N, column M: ...`），绝不 panic。
  多行字符串/浮点/日期/内联表/`[[数组表]]`/重复表头/数字键等越界语法给出
  明确报错。
- **配置 schema**（`model.rs`/`schema.rs`）：键名与默认值 1:1 对齐
  `engine-core/config.rs`（`engine.shift_action`=`commit|temp|none`、
  `engine.double_pinyin`=`off|natural|xiaohe|mspy|custom`（`flypy` 为小鹤别名）、
  `chinese_punct`/`user_learning`/`page_size`/`max_candidates`、
  `[engine.fuzzy]` 8 开关、`switches.<name>` 动态布尔表、`punct.<name>`
  字符映射占位）；错误带点分路径（如 `engine.shift_action: expected one
  of ...`），未知键收集为 warnings 不阻断加载。
- **用户 patch 叠加**（`patch.rs`）：`apply(default, user) -> Result<Config,
  String>` 深合并（表内按键合并、数组整体替换、类型不匹配报带路径错误）；
  `serialize_config/serialize_table` 输出确定性格式（标量在前、表头在后、
  2 空格缩进、插入序键序），parse→serialize→parse 语义等价。
- 已接线 addon：`new_engine` 读取 `$XDG_CONFIG_HOME/fcitx5/ior/config.toml`
  （缺失/无效回落默认），并按用户数据目录 → 系统目录 → builtin 顺序加载
  `core.ior`；`new_engine_with` 保留测试注入 Config+Dictionary。

#### 验证
- `cargo test --workspace`：**292/292** 通过；`clippy -D warnings`、
  `cargo fmt --check`、行数门禁全绿；fcitx5 C++ cmake 构建通过，`nm -D` 18 符号。
- **真机回归**（Ubuntu noble/X11/fcitx5 5.1.7/GTK，xdotool 合成键序）：
  全拼 `tianqi/jintian/nihao/zhongguo` 首选 天气/今天/你好/中国；自然码
  `jntm/vsgo/nihk` 首选 今天/中国/你好；不完整 `nih` → 你会、`n` → 你；
  整句 `jintiantianqibucuo` → 今天天气不错。记录见 `docs/M2-VERIFY.md`。
  M1 批次的翻页/标点/Shift/userdb 四路落盘见 `docs/M1-VERIFY.md`。
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
### change(engine)：中英切换等同回车——带出未上屏内容，`shift_action` 配置退役

- **行为变化（用户决策）**：中文模式输入半截拼音（含暂存选字）后切到
  英文（Shift / 托盘 / FFI `set_ascii_mode` 任一路径），不再按
  `shift_action` 处理，而是**完全等同回车**：走 `enter_action` 同一
  逻辑——默认 `raw` 原样上屏（如 `ni'hao` → 上屏 `ni'hao`），
  `enter_action = "candidate"` 时上屏首选候选（学习照常、span 暂存
  汉字携带上屏）。切回中文或无待上屏内容时行为不变（仅切模式）。
- **配置移除**：`engine.shift_action`（`commit|temp|none`）随之退役——
  三态被 `enter_action` 单轴取代，engine-core `ShiftAction`、
  ior-config model/schema/patch 镜像删除。旧配置文件中的
  `shift_action` 键降级为 unknown-key 警告（原为加载警告通道），
  不再报错、不影响加载。
- **历史**：b6b407d 曾把默认从 `commit` 改 `none`（防误上屏）；本次
  反转为"切换即回车"，与主流输入法（搜狗等）切英文带出原码一致。
- **ABI 不变**：`ior_engine_set_ascii_mode` / Shift keysym 语义仅
  commit 内容变化，符号集与签名不动。
- 测试：`process_shift` 四层断言改写（engine-core 默认原码上屏、
  candidate 路径学习、空缓冲静默、span 暂存带出），ffi/engine_state
  的"丢弃"断言改为"上屏"断言；378 tests / clippy / fmt / 行数门禁全绿。
