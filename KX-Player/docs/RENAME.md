# 改名子系统架构与交付文档（`RENAME.md`）

> 本文是 `docs-vibecoding/04-ai-subsystem.md` §7 改名与翻译决策规范的**真实实现交付文档**。
> 写的是改名子系统当前在盘上的实际架构、流程与代码落点（逐条给出 `文件:行`），以及与规范事实源的自洽状态。

---

## 1. 架构总览与核心设计原则

改名子系统贯彻「**软件做前置判定与规范化，AI 只做片段翻译**」的确定性架构，以确保本地模型小上下文、低吞吐下的极致性能与高稳定性。

```
                  ┌─────────────────────────────────────────────────────────┐
                  │ 1. 扫描与清点 (listing.rs / preview.rs)                 │
                  │    - 只扫一层目录，根目录绝对保护                       │
                  │    - 统一 Unicode NFC 归一                              │
                  └───────────────────────────┬─────────────────────────────┘
                                              ▼
                  ┌─────────────────────────────────────────────────────────┐
                  │ 2. 十四级判定树 (judge.rs)                              │
                  │    - 14级 first-match-wins                              │
                  │    - 接入拒绝集 (Skipped::Rejected) 与锁定               │
                  │    - 前置拦截纯标点/空格，零成本过滤                     │
                  └─────────────┬─────────────────────────────┬─────────────┘
                                │                             │
        [COMPLIANT / NORMALIZE] │                             │ [NEEDS_TRANSLATION / REPAIR]
                                ▼                             ▼
    ┌──────────────────────────────────────┐     ┌──────────────────────────────────────┐
    │ 3. 本地规范化 (norm.rs / rules.rs)   │     │ 4. AI 译名流水线 (ai_name / ai_ask)  │
    │    - 提取序号前缀逐字节保留          │     │    - 剥离前缀/扩展名，仅送纯片段     │
    │    - 符号映射统一（波浪号/全角等）   │     │    - 固定 SYSTEM 前缀，KV Cache 复用 │
    │    - 超长收敛（先剔除可省修饰语）    │     │    - 9道纯函数后置校验 (postcheck)    │
    └──────────────────┬───────────────────┘     │    - 译名核准入库 (ai_ledger)        │
                       │                         └──────────────────┬───────────────────┘
                       │                                            │
                       └──────────────────────┬─────────────────────┘
                                              ▼
                  ┌─────────────────────────────────────────────────────────┐
                  │ 5. 预览审查与交互 (AiRenamePanel.vue / 子组件)           │
                  │    - 强制 Dry-Run 预检                                  │
                  │    - 视觉化冲突检测 (SameTarget / TargetExists / ...)   │
                  │    - 行内单项拒绝建议写入 rejected_suggestions          │
                  └───────────────────────────┬─────────────────────────────┘
                                              ▼
                  ┌─────────────────────────────────────────────────────────┐
                  │ 6. 事务化执行与回滚 (apply.rs / rollback.rs / paths.rs) │
                  │    - DB_LOCK 独占写锁                                   │
                  │    - 目录深层优先，撤销浅层优先                         │
                  │    - 批次落库 (rename_batches / rename_items)           │
                  │    - 追加既有 rename_history.json (history.rs)          │
                  │    - 支持整批回滚 (rollback) 与重做 (redo)              │
                  └─────────────────────────────────────────────────────────┘
```

### 核心安全边界

1. **唯一改名调用点**：全子系统只有 `rename/paths.rs:61` 一处 `std::fs::rename` 动真实文件，所有路径变更必须受其三条拒绝保证（原路径不存在拒、目标已存在拒、不代造父目录）。
2. **命令入参始终为根目录路径**：`rename_apply` 与 `rename_preview` 均接收目录路径入参并在内部重算，不信任前端回传的条目列表，彻底免疫列表状态过期引发的错位改名。
3. **模型译名只读账本**：`rename_suggest_names` 只向数据库写入已通过 9 道校验的译名，绝对不碰磁盘；`rename_apply` 从库中通过内容哈希读取核准译名。

---

## 2. 模块清单与职责分配

| 模块文件 | 职责说明 | 关键接口 / 结构 |
|---|---|---|
| `rename/mod.rs` | 改名子系统入口与子模块声明 | 导出 16 个功能模块 |
| `rename/rules.rs` | 补充规则表（NFC 归一、04 §7.5 机翻短语与汉字漏译、可省修饰语） | `to_nfc`, `has_extra_mt_smell`, `strip_omissible_modifiers` |
| `rename/norm.rs` | 本地规范化（前缀切分四族、符号转换、超长词边界截断） | `split_prefix`, `family`, `normalize`, `truncate` |
| `rename/judge.rs` | 十四级优先级判定树（含假名、繁体、机翻痕迹、格式混用） | `judge`, `Verdict`, `Reason`, `Ctx` |
| `rename/preview.rs` | 纯函数条目评估（规范化目标名生成、三向冲突检测） | `preview_level_with_skipped`, `Item`, `Counts`, `Conflict` |
| `rename/listing.rs` | 一层文件目录列出与条目聚合、全量统计与截断处理 | `preview_roots_with_skipped`, `MAX_ITEMS` |
| `rename/ai_name.rs` | AI 契约构造（固定 6 条规则 SYSTEM Prompt、片段剥离、拼回与内容键计算） | `fragment`, `rebuild`, `cache_key`, `SYSTEM_PROMPT` |
| `rename/postcheck.rs` | `04 §7.7` 九道纯函数后置校验（前缀保护、扩展名、括号配对、数字多重集等） | `postcheck`, `PostResult`, `step1` ~ `step9` |
| `rename/ai_ask.rs` | 异步请求编排、并发控制信号量、连续失联早停、一次 Retry | `translate_missing`, `ask` |
| `rename/ai_ledger.rs` | 译名账本（`rename_name_cache` 读写、`rejected_suggestions` 记录与过滤） | `plan`, `record`, `suggestion`, `annotate`, `reject`, `all_refused_paths` |
| `rename/apply.rs` | 真实改名执行器（重算预览、过滤定稿条目、逐项改盘落库） | `apply` |
| `rename/rollback.rs` | 批次整批回滚与重做（反转操作，复用执行核心） | `rollback`, `redo`, `reverse` |
| `rename/paths.rs` | 路径排序（正向深层优先、撤销浅层优先）与单一 `move_one` 执行点 | `move_one`, `sort_pairs` |
| `rename/batch.rs` | 数据库批次记账（`rename_batches` / `rename_items`） | `open`, `set_item`, `finish`, `load_batches` |
| `rename/history.rs` | 兼容已有生态：向 `rename_history.json` 追加与回退记录 | `append_batch`, `revert_batch` |
| `rename/report.rs` | 改名执行汇总报告 DTO | `Report` |

---

## 3. 核心业务流程与交互机制

### 3.1 判定与前置清洗
1. **Unicode NFC**：所有输入名称首先经由 `rules::to_nfc` 规范化为 NFC 单一码位表示。
2. **前缀切分**：`norm::split_prefix` 按正则匹配 `【NN】` 括号族、`hi09_06_` 等关键字族、`01_` 等数字族；命中部分按字节原样切出保留，仅对主体部分进行处理。
3. **十四级判定**：
   - 第 1 级优先检查 `SKIP(locked | rejected)`，命中被拒路径直接跳过；
   - 第 2-9 级纯本地判定（非法字符、全角空格、波浪号混用、全半角标点混用、长度超限等）；
   - 第 10-13 级检查机翻痕迹（`mt_smell`）、假名、繁体/日文汉字漏译、纯英文 ASCII；
   - 未命中任何规则则判为 `COMPLIANT`。

### 3.2 AI 译名与后置九道校验
1. 片段送交模型：去除前缀与扩展名，仅将核心文本（如 `親切なお姉さん`）送往模型。
2. 内容寻址缓存：以 `blake3("name-3" + fragment)` 为键，已存在的译名直接复用，不重复消耗 Token。
3. 后置九道校验（`rename/postcheck.rs`）：
   - `Step 1`: 前缀强制回填；
   - `Step 2`: 扩展名保护（剥除模型可能臆造的后缀）；
   - `Step 3`: 括号种类与数量必须与原文严格一致（不符则标记重试）；
   - `Step 4`: 数字序列多重集必须完全一致（01 不许变 1）；
   - `Step 5`: 清理 Windows 非法字符；
   - `Step 6`: 长度收敛超长截断；
   - `Step 7`: 日文假名必须有效译为中文汉字；
   - `Step 8`: 首尾空格与标点清理；
   - `Step 9`: 拒绝建议复核（用户拒过的名字立刻剔除）。

### 3.3 批次落库与回滚
1. 数据库事务记录：
   - `rename_batches`: 记录批次号、模型名、时间戳、条目数、`rolled_back` 标记；
   - `rename_items`: 记录原路径、新路径、`applied` 状态。
2. 历史文件兼容：
   - 检查父级是否存在既有 `rename_history.json`，若有则追加本次改名映射（`{"nor": {"old_folder": {"_": "new_folder"}}}`），回滚时精准剥离增量；若无则不强行新建。
3. 可逆性：回滚（`rollback`）翻转批次状态并逆向移动文件；重做（`redo`）再次正向执行，全链路具备严格的等幂性与可恢复性。

---

## 4. 与规范偏离与现状说明

1. **`Locked` 状态持久化**：规范 `04 §7.1` 提及的独立 `ai_rename_state.json` 尚未落地，当前架构中 `Skipped::Locked` 内存语义完整，持久层优先采用 SQLite 表体系管理。
2. **`provider_id` 字段**：`rename_batches` 中的 `provider_id` 当前为 `NULL`，等待后续全局 Provider 实体表建设完成后对齐接入（当前已正常记录 `model` 审计字段）。
3. **进度显示策略**：改名建议生成目前采用批量并发与阈值早停（上限 200 项），未接入 `ai_job` 的长任务进度泵，采用轻量化即时回调与报告模式。
