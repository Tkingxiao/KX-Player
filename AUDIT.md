# KX-Player 当前审计

> 审计日期：2026-09-24
> 范围：架构、Rust/Tauri 底层、SQLite、IPC、业务逻辑、并发、可靠性、安全、性能、UI/UX、无障碍和测试。
> 本文只保留**当前仍未闭合的问题**。已经落地并有门禁/测试验证的历史整改不在本文重复记录。
> 交接执行说明见 [HANDOVER.md](HANDOVER.md)。

## 1. 当前判断

项目已经具备可用的 Tauri + Vue + Rust + libmpv 产品骨架，也有 IPC、数据库列名、色值、类型、Vitest 和 clippy 门禁。

当前主要矛盾已经从“功能是否存在”转为：

1. IPC 对文件系统、ffmpeg、mpv 和 AI 网络端点的信任边界过宽。
2. 扫描、播放、AI、转换、watcher 和退出流程缺少统一的任务生命周期。
3. 曲库仍以全量快照传输，筛选、排序和搜索仍在前端内存执行。
4. 多处失败仍会被表现成空数据、`false` 或成功，导致 UI 与真实状态分离。
5. UI 的主要可用性风险集中在错误恢复、键盘可操作性、焦点管理和窄窗口布局，而不是颜色或品牌方向。

## 2. P0：安全与数据正确性

### P0-1 文件读取没有限额、落盘没有原子性

路径能力（`ScopedPath`）于 2026-09-24 随第 22 批闭合，读取限额与原子写于同日的第 23 批闭合，证据见 §9。本节**没有剩余项**：

- `ffprobe.rs::run_probe` 曾经用 `Command::output()` 整份收 stdout（既无字节上限也无超时，ffprobe 卡住就没有返回）、`fftools.rs` 的子进程 kill 之后没有 wait —— 这两件已于同日第 32 批连同 P1-4 一起闭合：探测走统一的子进程通道（30 秒超时、stdout 4 MiB 上限且**超限整份拒绝**），kill 之后一律 wait 到真的退出，见 §9 与 P1-4。

已完成的两条（细节与覆盖清单见 §9，不要重新列为未完成）：

- 读文件限制原始字节数，图片限制像素数，Base64 限制解码后大小；超限拒绝（返回空值 + `[limit]` 日志，不静默截断）。错误信封化随 P1-6 统一做。
- 写文件统一走 `src-tauri/src/atomic.rs`：唯一临时名 + flush + sync_all + 同目录 `fs::rename` 替换（与 P1-7 同一份实现），覆盖设置、改名账本、封面与映射、AI 译文与导出的共 8 类落盘点。
- 通用 `tools_save_file` 已于 2026-09-24 删除（前端零引用且是任意路径写入器）；后续若需要保存能力，按「只落在受控目录」重建，不接受任意目标路径。
- 路径能力的 AI 侧于同日第 26 批补齐：`ai_translate_start` / `ai_export_prompts` / `ai_import_translations` / `subtitle_scan_dir` 四条接 `PathScope` 整批拒 —— 至此「前端递进来的路径只认授权」在文件系统命令与 AI 命令两侧口径一致（细节见 §9）。

### P0-2 ffmpeg 任意参数和路径

证据（`commands/system.rs::ffmpeg_exec` 接受 `Vec<String>` 一条已于 2026-09-24 第 24 批删除，命令与 DTO 一并从契约里去掉；剩下的口径如下）：转换队列曾信任 `ConvertItem` 的输入输出路径、参数由前端决定并用覆盖式输出。第 24 批改为：命令只收 `ConvertItem`（kind/format/path/outPath），参数由 `fftools::build_convert_args` 在 Rust 侧拼装，格式表 `CONVERT_FORMATS` 之外一律整批拒绝。

风险：ffmpeg 参数本身支持网络协议、设备、复杂 filter 和多种输入源。无 shell 不等于安全；可能读写非预期资源或覆盖用户文件。

整改：删除通用执行器对前端的暴露（done，第 24 批）；改为白名单化的转换/提取/探测命令（done，第 24 批，无 copy 兜底、无自定义 filter）；输入必须属于授权路径、输出只能落在源目录同级且不得覆盖已有文件（done，第 24 批，`validate_items` 整批校验）；网络/设备协议按「输入路径含 `://` 或形如选项」直接拒，未允许的 filter 无从传入 —— 命令行整条由 Rust 侧拼装，前端只给 kind/format/path/outPath；探测侧的子进程超时/输出上限/kill+reap 已于第 32 批闭合（`probe_argv` 仍是固定参数列表，路径是最后一个独立参数）。**本节没有剩余项**，细节与用例见 §9 与 P1-4。

### P0-3 AI endpoint 与 API Key 外送

证据（原文两条已于 2026-09-24 第 25 批处理，剩下的一条例外见末尾）：`normalize_base_url` 曾对没写 scheme 的地址一律补 `http://`，而且没有任何私网、端口与重定向策略；`ai_chat`/`ai_ping`/`ai_list_models` 把 API Key 放进请求头；Key 在任务 spec、payload 与日志里以明文 `String` 形式活着。

风险：恶意设置或被篡改的配置可能访问内网服务、云元数据或将凭据发送到任意服务器；默认允许明文 HTTP。

整改：默认 HTTPS（done，第 25 批：没写 scheme 时按主机补，本机/私网才补 `http`；`http://` 公网地址直接拒）；localhost/本地模型单独白名单（done，第 25 批：回环 / 私网 / 链路本地三类，不做 DNS 也就没有重绑定）；私网地址和重定向逐跳校验（done，第 25 批：跨源重定向一律拒，同源跳最多 3 跳且逐跳过同一套校验）；限制端口（done，第 25 批：公网主机只允许 443，本机与私网不限 —— 自建服务的端口是用户自己定的）；请求中禁止泄漏完整 Key（done，第 25 批：Key 只出现在 Authorization 头，日志与 `Debug` 输出一律只报「有没有、多长」）；后端保存层拒绝持久化 `apiKey` 等秘密字段（done，第 25 批：`settingsio::save` 写入前剥掉秘密名字段并记 `[settings]` 日志）；任务线程中的秘密使用 `SecretString/Zeroize` 或等价生命周期控制（done，第 25 批：`zeroize` 清零 payload/spec/`Creds`，并手写 `Debug` 防 `{:?}` 泄漏）。

**显式放行名单（2026-10-25 补记，P0-3 的唯一例外机制）**：自建反代常挂在公网 IP 的非 443 端口且未必有证书，默认策略会把它拦下。放行必须是**用户在界面上逐条确认的动作**，落在 SQLite `ai_plaintext_allow`（007 迁移，键 `host+port`，与 scheme 无关：公网明文与公网 https 自定义端口一并覆盖）而不是可被整份覆盖的设置 JSON。应用路径的唯一入口是 `ai_url::check_for_app`（自动带上进程内缓存的名单），不带名单的 `check` 留给单测与文档。名单只解锁「公网明文」与「公网非 443」这两条可放行策略——scheme 白名单、凭据不入 URL、重定向同源三件不因放行松动（地址本身不合规，放行也救不了）；按 `host:port` 精确匹配（`endpoint_key`，IPv6 带方括号），放行一个端点不放行同主机的其它端口。`ai_endpoint_check` 给界面供靶（`ok`/`reason`/`endpoint`/`allowlist`，只有「补一条放行真能过」才给 `endpoint`），`ai_plaintext_set` 登记/撤销并返回刷新名单；本机/局域网地址拒绝登记（本来就放行，名单只会误导人）；读库失败按**空名单**处理并记日志——宁可退回默认策略，不能悄悄放行。**未做（明确取舍）**：DNS 解析后的 IP 复检——域名解析到私网（DNS 重绑定的一种）不在本批范围，因为我们不对域名做解析；要彻底封住得在连接层固定 IP，与 `reqwest` 的连接池模型冲突，留作已知缺口。

### P0-4 部分扫描覆盖完整曲库

证据：`scanner/mod.rs` 的目录权限错误、30 秒发现超时、100000 条上限和 worker 超时路径都可能返回部分文件；调用链仍可能继续 `persist_snapshot`。

风险：网络盘、权限变化、超大目录或损坏媒体下，未扫描到的文件可能被误判为删除，旧快照被部分结果覆盖。

整改：扫描结果必须携带 `complete/partial/cancelled/failed`；只有完整扫描允许执行“本轮未见即删除”；部分或取消必须保留旧条目并在 UI 显示原因。watchdog 不能只 detach 卡死 worker 后继续当作成功。

上述判据于 2026-09-24（第 29 批）闭合，细节与用例见 §9，不要重新列为未完成：扫描结果自带 `ScanStatus`（`complete`/`partial`/`cancelled`/`failed` + 理由串），只有 `complete` 允许执行删除，部分/取消走 `WriteMode::NoDelete`（只补不删）并把库里合并后的真相回读给前端；互斥是 `scanner/gate.rs` 的**全局**闸门（latest-wins，且「谁是最新的」与「谁有权写库」在同一把锁里）。

### P0-5 播放加载代际竞态

证据：`src/stores/player.ts` 虽有 `loadGeneration`，但旧请求的失败、迟到成功和 `finally` 路径仍可能修改当前播放状态；Rust 侧也缺少完整的播放代际约束。

风险：快速连续点击曲目时，旧请求可能停止新曲目、覆盖错误状态或错误写入进度。

整改：前后端统一 `generation/taskId`；所有 resolve/reject/finally 和 mpv 事件都只能修改当前代；旧请求取消或结果丢弃；补连续切歌、超时、旧错误迟到测试。

上述三条于 2026-09-24（第 28 批）闭合，细节与用例见 §9，不要重新列为未完成：前端 `loadTrack` 收敛成 `ok/failed/stale` 三态（迟到的失败与收尾一律闭嘴），`player:state` / `player:loading` / `player:ended` 全部带 `generation`，落后的广播直接丢；Rust 侧用 `pending_load` 牌子把「命令已下发、`StartFile` 还没到」这段窗口里的文件级属性（`time-pos` / `duration` / `video-params/dw|dh`）挡在共享状态之外，起播失败连牌子带「正在播放」的假象一起收回；进度回写只认引擎报过的那一代。

### P0-6 曲库查询仍是全量快照

证据：`src-tauri/src/db/library.rs` 多处无界 SELECT 并构造完整快照；`src/features/library/SmartView.vue` 和 `src/features/search/SearchView.vue` 在前端对 `allTracks` 全量筛选、排序和模糊搜索。

风险：万级以上曲库启动、搜索、筛选和排序会退化，WebView 内存与主线程压力随曲库增长；这是当前规模化红线。

整改：查询下沉 SQLite，提供分页/游标、稳定排序、轻量 `TrackRow`、SQL filter/sort、revision；前端只维护稀疏页缓存。搜索后续迁移 FTS5、简繁和拼音索引，移除前端大搜索依赖。

### P0-7 字幕链路尚未统一到 libass

证据：`src/features/lyrics/LyricsView.vue` 和 `src/utils/parsers/lyrics.ts` 仍由前端解析并自绘歌词/字幕浮层；`docs/ARCHITECTURE.md` 仍记录 libass 统一渲染未完成。

风险：字幕格式、轨道选择、时间轴、双语、偏移和视频舞台存在两套事实源，长期维护和复杂格式兼容性差。

整改：保留 Rust 侧发现/抽取职责，将渲染和样式统一交给 mpv/libass；前端只控制轨道、样式、位置和偏移。补双语、偏移持久化、拖入字幕、内封字幕导出等验收。

### P0-8 mpv 嵌入失败缺少独立降级

证据：`docs/ARCHITECTURE.md` 的 P0-20 仍未闭合；当前舞台依赖原生 mpv 覆盖窗口，嵌入失败时没有明确的独立视频窗口或可靠的音频-only 降级入口。

风险：用户可能只听到声音、看到空舞台，且没有可恢复路径。

整改：检测 HWND/overlay/mpv 初始化失败，给出明确错误和“以独立窗口播放/仅音频播放/重试”选项；降级状态必须进入播放器状态机和日志。

## 3. P1：生命周期、并发和错误语义

### P1-1 后端扫描没有互斥

前端 `scanGeneration` 只能保护 UI，不保护数据库。启动同步、手动扫描和 watcher 扫描仍可并行计算，最后完成的旧任务可能覆盖新任务。需要 Rust 侧按 root 互斥、latest-wins、提交前校验 generation，并将扫描任务化。

上述判据于 2026-09-24（第 29 批）闭合，细节与用例见 §9，不要重新列为未完成：互斥做在 Rust 侧 `scanner/gate.rs` 的**全局**闸门（`ScanGate`/`Ticket`/`begin`/`finish`/`if_current`），`begin()` 把在飞的所有旧票标为被取代，`if_current` 在持锁期间执行提交闭包（「谁是最新的」与「谁有权写库」拆不开）；前端 `scanGeneration` 从此只管 UI。

### P1-2 mpv 没有专用串行命令线程

多个 Tauri command 和事件线程可能同时访问 `PlayerCore/Mpv`；`embed.rs` 的舞台几何队列是无界累积。需要 bounded command channel、单一 mpv command thread、stage rect latest-value coalescing，以及可回滚的初始化状态机。

上述四条于 2026-09-24（第 30 批）闭合，细节与用例见 §9，不要重新列为未完成：所有 mpv 访问（写属性、发命令、读属性）都投给 `player/queue.rs` 的单一命令线程，队列有硬上限（满了等空位、等到超时报拥堵，不无声丢弃），同键作业在**排队期间**合并成最后一条；舞台几何在 `player/embed.rs` 里 latest-wins，离散命令（可见性/挂靠/销毁）一条都不合并；`PlayerCore::ensure_started` 的每条失败路径整份回滚（队列关停 + `embed::destroy()`），`cmd` 成为「mpv 是否就绪」的唯一判据。

### P1-3 没有统一 shutdown supervisor

退出流程仍依赖短暂 sleep，未可靠等待 mpv、stage、scanner、watcher、AI、ffmpeg、转换队列和设置/历史写盘。应按“停止接收 → cancel → 停 watcher → 等任务 → kill+wait 子进程 → flush → join → exit”实现统一 supervisor。

上述于 2026-09-24（第 33 批）闭合，细节与用例见 §9，不要重新列为未完成：退出只剩 `shutdown::request_exit` 一个入口（托盘退出、`app_force_quit`、`force_close_window` 与窗口/会话退出触发的 `RunEvent::ExitRequested` 全部汇到它，`claim()` 保证整条流水线只跑一次），七步固定为「停止接收 → 取消在飞任务 → 停文件监听 → 等任务 → 杀子进程 → 等前端落盘 → 收线程」。每一步都有预算与记账：「等任务」3 s、「等前端落盘」2 s、「收线程」2 s，到点就把「谁还在跑」写进日志再往下走 —— 没有一处是「睡一会儿再假设已经做完」。前端落盘不再靠猜：`App.vue` 在 `save_settings` 真的返回之后回执 `shutdown_ready(ok)`，日志据此区分「已确认落盘 / 回执说没写成功 / 等不到回执」三种收场。

### P1-4 ffmpeg/ffprobe 子进程回收不完整

`fftools.rs::exec_async` 超时后只 `start_kill`，没有保证 wait/reap；`kill_all` 仍为空实现；`ffprobe.rs` 缺少统一超时和输出上限。需要句柄表、kill+wait、退出回收和单任务输出上限。

上述三条于 2026-09-24（第 32 批）闭合，细节与用例见 §9，不要重新列为未完成：子进程统一走 `fftools.rs` 的「登记 → spawn → select(正常结束 / 超时 / 应用退出)」通道，**杀完必须 wait 到真的退出**；在飞登记表（`LIVE`）配合退出闸门（`SHUTDOWN`）让 `kill_all` 逐个通知持有者自己 kill+reap、**等回收完才返回**，且闸门关上后不再出生新子进程；输出收法分 `OutCap::Tail`（ffmpeg，64 KiB 尾部窗口）与 `OutCap::Head`（ffprobe，4 MiB 上限、**超限整份拒绝**）。**已知残留（属 P1-3，已于第 33 批闭合，见 P1-3）**：退出曾是「300 ms 延时 + `kill_all` + `app.exit(0)`」，mpv、stage、scanner、watcher、AI、写盘各自拆解 —— 本批只保证「ffmpeg/ffprobe 这一类子进程在退出时被 kill 且回收」，统一 supervisor 与 `kill_all` 的接线在第 33 批完成（它现在是七步里的「杀子进程」一步）。

### P1-5 watcher 线程风暴和旧事件污染

`watcher.rs` 每个事件都创建 sleep 线程；`stop()` 不推进 debounce generation；watch 失败被吞掉，命令层仍可能返回成功。需要一个 debounce worker、dirty path 集合、start/stop generation、根目录单任务和错误返回。

上述三条于 2026-09-24（第 31 批）闭合，细节与用例见 §9，不要重新列为未完成：事件只投给**一条**常驻 worker 线程（`fs-debounce`，闲时阻塞不占 CPU），脏路径进 `BTreeSet`（去重 + 稳定顺序），攒满 1 s 窗口或攒满 `MAX_BATCH`（1024 条）就发一批 —— 整目录拷贝不再起上千条 sleep 线程，持续写入也不会把事件饿到静默为止；`start`/`stop` 各自推进 `WatcherState.gen` 代际，过代的 worker 连攒好的那批一起作废（`stop` 之后攒着的不发），旧事件不会打进新一代；`watcher::start` 返回**挂不上的根**、`start_watching` 据此返回 `IpcResult<bool>`（不再无条件 `true`）并逐条记 `[watcher]` 日志，一个根都没挂上时不留半个监听器。**已知残留（属 P1-6）**：前端仍是 fire-and-forget（`bridge/ipc.ts` 的 `.catch(() => false)`、store 里的 `void api.startWatching(...)`），挂不上监听暂时只有日志 —— 统一 UI error/retry 状态是 P1-6 的范围。

### P1-6 错误被吞，成功语义不可信

进度/书签/响度链路的后端半已于 2026-10-25（第 37 批）闭合，证据见 §7：`db::progress` 七个函数返回 `Result`（写失败上抛；读侧「库不存在 = Ok(空)」与「库在但读失败 = Err」分开）、八条命令改 `IpcResult`（校验 `invalid_argument` / join `join_failed` / 库 `IpcError::db` 三分类）、响度「量出来了但写库失败」进 `failed` 计数并记日志、`loudness:progress` 两处 emit 失败记 `[loudness]` 日志、bridge 对用户主动操作（clear/list/rename/remove）撤 catch 让 reject 上抛并由调用方 toast/warn 接住。

当前仍未闭合的剩余项：

- UI 统一 loading/success/error/retry action wrapper：各视图仍各写各的（`errText` + toast 只在部分面板有），没有统一的 action 包装层与重试入口。
- watcher 挂不上监听（离线盘）现在只有 console.warn 与 `[watcher]` 日志，没有 UI error/retry 状态（P1-5 的已知残留归到这里）。
- 读取「empty/not-found/unavailable/failed」的区分只在进度/书签做了；其余读类命令（封面、设备、目录树等）仍是失败冒充空数据。
- 残余的 `let _ =`：pip 与主窗口的 show/hide/focus 这类 UI 操作、`addBookmark` 的 catch→null（调用方尚未区分 null 来源）等，影响面小，按链路逐条裁决，不一刀切。

### P1-7 设置、历史、封面和译文落盘不具备崩溃恢复

`settingsio.rs`、`rename/history.rs`、`ai_chunk.rs`、`ai_export.rs`、`scanner/covers.rs` 存在固定 tmp、并发保存、非原子覆盖或忽略失败问题。UUID 迁移、封面重命名、settings remap、改名应用还跨 SQLite 与文件系统，无法整体回滚。

整改方向：唯一临时文件名、进程锁、flush/sync、平台原子替换、journal/intent、启动恢复和损坏文件备份。改名和迁移必须支持崩溃后检测未完成批次。

### P1-8 AI 任务一致性

当前仍有以下风险：

- 任务监听注册为异步，启动后可能丢失早到的进度/收尾事件。
- 源文件在请求期间变化时，旧译文可能与新文件 stat 绑定，污染缓存。
- 旧译文只按存在/mtime 的兜底判断，内容 hash 关系不足。
- 取消主要是旗标，在途 HTTP 请求不一定真正取消。
- 改名建议并发点击可能重复消耗 token。

整改：任务先订阅再启动，或后端保存可 replay 的任务状态；请求开始和写回前都校验内容 hash；缓存绑定 source hash + prompt version + model/provider；实现请求取消、任务去重和稳定终结态。

### P1-9 转换任务不可恢复

转换页卸载后任务仍可能继续，但重新进入页面无法查询在途任务、失败项和历史结果。事件不能作为唯一事实源，应增加 `task_list/task_get` 或等价的任务快照查询。

### P1-10 数据库连接和关系完整性

`db/mod.rs` 的全局 Mutex 串行所有读写，进度高频写回、扫描、taxonomy 和 AI cache 互相影响；每次调用还会重新打开连接和设置 PRAGMA。部分关系表缺严格外键约束，迁移结构校验也不足。

整改：读连接/单写者分离，批量进度写回，prepared statement、busy timeout、只读事务、FK/foreign_key_check、迁移 preflight 和可恢复 journal。

### P1-11 改名的文件系统与数据库不是事务

`rename/apply.rs` 先改文件再写批次明细，进程崩溃会留下“文件已改但没有账”的窗口；rollback/finish 失败时也可能出现磁盘和数据库不一致。`move_one` 的 exists 检查与 rename 之间还存在 TOCTOU。

整改：intent journal + 可恢复批次；目标创建使用不覆盖语义；应用、回滚、重做都支持启动恢复和幂等补偿。

上述三条于 2026-09-24（第 27 批）全部闭合，细节与用例见 §9，不要重新列为未完成：动手前把意图写进 `journal/rename-<batchId>.json`（写不进去就不动手），启动时按「**盘是事实、恢复只补账**」核对 `rename_items` 旗子、批次头与 `rename_history.json` —— 恢复**不替用户继续搬文件**；`move_one` 的不覆盖语义落到不带 REPLACE 的 `MoveFileW` 上；旧 ID 迁移那一半也写了 `journal/legacy-ids.json`，启动时只补「旧在、新不在」的封面（应用自管的 UUID 文件）。

### P1-12 ffmpeg 可执行文件来源未验证

`paths.rs` 允许环境变量、程序目录、固定目录和 PATH 中的同名文件决定实际 ffmpeg。已按 2026-09-24 第 24 批完成「记录 + 警告」这一层：`locate_ffmpeg_ex()` 返回 `FfmpegSource`（env/appDir/fixed/path），探测结果（`FfmpegInfo`）带上来源与 `warn`，启动时后台线程把实际路径、来源、版本和 blake3 指纹（前 16 位十六进制）写进 `[ffmpeg]` 日志行；`KX_FFMPEG` 环境变量与 PATH 命中的一律提示，程序目录/固定目录若位于用户可写目录（USERPROFILE/HOME/LOCALAPPDATA/APPDATA/TEMP）也提示，转换页显示来源提示条。未做（保留为已知取舍）：不校验签名、不阻止用户显式指定路径——提示而非拒绝，因为这是用户自有机器上的环境配置。

### P1-13 仍未闭合的功能矩阵

以下是旧规划中当前仍有源码/文档证据的功能缺口，不能与已完成的 IPC、AI 任务基础、改名主链和 ffmpeg 分发裁决混为一谈：

- 字幕：双语字幕、偏移持久化、拖入字幕、内封字幕导出，以及统一 libass 渲染。
- 视频/媒体：视频首帧封面、重封装/压缩、转换后入库或删除源、章节。
- 转换与队列：并发与队列重排等完整任务管理。
- 系统能力：媒体键、批量编辑、分类/标签管理页、统计等。
- AI：多 Provider 配置、字幕侧结构保留、暂停/继续、费用护栏和可重跑缓存命中率 benchmark。

已裁决“不随包分发”的 ffmpeg 不属于缺口，不要重新加入任务列表。

## 4. P1：规模化性能与数据架构

### P1-13 曲库仍是全量快照

证据：`db/library.rs::load_snapshot` 多处无界 SELECT；`library.ts` 持有 `allTracks`；`SmartView.vue`、`SearchView.vue` 在 JS 中全库 filter/sort/fuzzy。

影响：万级到十万级曲库的启动、搜索、筛选、排序和 WebView 内存会持续退化；当前 `vendor-search` 仍是前端大块依赖。

目标接口：

```ts
interface LibraryPage {
  rows: TrackRow[]
  total?: number
  nextCursor?: string
  hasMore: boolean
  revision: number
}
```

路线：SQL 筛选/排序/分页，默认 50、上限 200，排序追加 id；分类/标签使用 EXISTS/CTE；列表 DTO 不传歌词正文、base64 和目录树内嵌 tracks；前端用稀疏页缓存；播放队列只传 ID。

验收目标：首屏 50 行 ≤1s，普通查询 p95 ≤100ms，单页 IPC p95 ≤64KB，100k 曲库不持有全量 Track，滚动 ≥55 FPS。

### P1-14 搜索需要下沉到 SQLite/Rust

迁移 FTS5、简繁规范化、拼音全文/首字母索引和 LIKE fallback；后台分批 backfill，完成后移除 `pinyin-pro`/`chinese-conv`。搜索结果需要 exact/prefix/bm25 稳定排序和 revision，旧请求不能覆盖新查询。

### P1-15 扫描需要 dirty-set 和可取消任务

统一 `library_scan_start/cancel`、`library:scan_progress/changed`，事件带 taskId/generation/revision；watcher 只投递 dirty paths；同 root 单任务；写入按小批事务提交，取消和失败不清理未确认条目。

## 5. P1/P2：UI/UX 与无障碍

### P1-UI-1 播放错误和 loading 不可见

`player.loading/error` 没有稳定接入 PlayerBar/Stage 的恢复 UI。需要 `idle/loading/playing/paused/error` 状态组件，错误附近提供“重试当前”和“下一首”。

### P1-UI-2 ConfirmDialog 焦点和原生视频遮挡

`ConfirmDialog.vue` 只在首次 mount 时聚焦，没有 focus trap、完整 ARIA 或打开后的焦点管理；`ui.confirmOpen` 没有同步，mpv 原生覆盖窗口可能盖住对话框。Stage 暂停视频时也可能错误自动隐藏顶栏。

### P1-UI-3 进度条、音量条和列表缺乏键盘语义

进度/音量是普通 div；表格表头、网格卡片、歌词行、Inspector 行存在 div click/dblclick。需要 slider、button、tablist/tab/tabpanel、Enter/Space 和可见 focus。

### P1-UI-4 事件生命周期与异步视图

`App.vue`、`player.ts` 等监听未形成统一可销毁 registry；异步视图缺少 loading/error/retry fallback。需要 composable event scope 和可重入初始化。

### P1-UI-5 搜索启动加载和大库输入体验

App 启动后仍可能无条件加载搜索依赖；搜索每个输入都遍历全库。短期应首次进入时加载、150–250ms debounce、generation/取消旧请求；长期由 P1-14 取代。

### P1-UI-6 QueuePanel 死代码和重复入口

`features/shell/QueuePanel.vue` 没有实际挂载入口，`ui.queueOpen` 也缺可见打开链路。应删除并统一 Inspector QueueTab，或正式挂载但不能保留两套队列实现。其封面路径也应统一使用 `MediaCover/assetUrl`。

### P1-UI-7 空态、失败态和窄窗口

扫描、索引、转换、AI、播放缺少一致的空/加载/错误/重试状态；900–1200px 下 PlayerBar 低频控制堆叠。应保留主播放链，低频操作进入 overflow，并让每个视图只有一个主动作。

### P2-UI-8 设计系统治理

继续使用现有 technical/utilitarian + aurora dark 设计语言，不新增第二套颜色、字体、圆角或动效。完善 reduced-motion 对 spinner/modal/view/wave 的全局覆盖，统一 CSS 所有权，避免 `base.css` 与 `App.vue` 重复布局定义。

## 6. 架构与工程治理

- 分阶段引入 `domain / infra / services / commands`，优先 player、library、rename，不做一次性重写。
- 用 `tracing` 替代并发不安全的手写 `append_log`。
- 所有长任务统一 taskId、generation、revision、状态快照和结构化日志。
- 建立 10k/50k/100k fixture、EXPLAIN QUERY PLAN、IPC bytes、WebView heap、FPS 和扫描基准。
- 扩展 IPC 门禁：当前 `check:ipc` 对嵌套 DTO/serde 字段仍偏浅，需要 JSON fixture/runtime contract tests。
- 架构文档和配置应有自动事实校验，避免 `docs/ARCHITECTURE.md` 与 `tauri.conf.json` 再次漂移。
- Rust 测试和 clippy 必须在固定 toolchain CI 中执行，避免本地 PATH 造成半验证。

## 7. 验证基线

2026-10-25 复核（第 37 批进度/书签/响度错误语义入库前，全量实跑；其上一次全量基线见本节末「合批入库」段）：

- `npm run check:types`：通过。
- 前端 Vitest：18 个测试文件、153 条用例通过（`tests/` 按用户 2026-09-22 裁决不入库、只在本机跑；本批新增 `tests/bridge-progress-errors.test.ts` 5 条：四类用户主动操作失败 reject 上抛、`setProgress` 仍按「尽力而为」resolve false）。
- `npm run check:ipc`：通过（113 条命令 / 16 个事件 / 115 处 invoke / 16 处监听）—— 命令数与上一基线相同（`IpcResult<T>` 在契约里本来就写 `T`，八条进度/书签命令改信封不动契约），`docs/IPC.md` 已随 `gen:ipc` 校验一致。
- `npm run check:columns`：通过（18 张表 / 101 个列名 / 68 个 db/*.rs / 342 处列引用 / 7 个 DTO 接口 / 263 个字段 / 8 条例外）。
- `npm run check:colors`：通过（DESIGN.md role 26 个 hex / theme.css 57 处字面值 / 组件侧 45 处字面值覆盖 81 个文件 / 16 处例外）。
- `npm run build`：通过（900 ms）。
- `cargo test`：354 passed / 0 failed（上一基线 352 + `db/progress.rs` 新增 2 条：进度/书签读写往返一条龙、「写失败必须上抛、库不存在是 Ok(空) 不是 Err」）。
- `cargo clippy --all-targets -- -D warnings`：无告警。
- `npm run check` 是前六段串成的一条命令（types → ipc → columns → colors → vitest → clippy），2026-10-25 一次跑通；`npm run test` 另含 `cargo test` —— 注意 npm 派生的 shell 不带 `~/.cargo/bin`，跑之前先把 cargo 加进 PATH，否则 vitest 过了 cargo 却「不是内部或外部命令」。

2026-10-25 合批入库基线（第 34–36 批：响度 Rust 侧积分、数据目录进安装目录与 NSIS 安装器、AI 三协议/端点放行/译文管理；当时一次实跑：vitest 17 文件 / 148 条、check:ipc 113 命令、check:columns 68 个 db/*.rs / 342 引用、check:colors 81 文件、cargo test 352 / 0 failed、build 985 ms；与 ffmpeg 自带 `ebur128` 滤镜的对账是 2026-09-24 一次性真跑，结论留档：48 kHz WAV `-32.2146` vs `-32.2`、44.1 kHz MP3 `-27.7754` vs `-27.8`、5.1 FLAC `-29.7157` vs `-29.7`、带封面 MP4 `-32.2166` vs `-32.2`，差都在滤镜法只报一位小数的舍入里）。`.ulpi/design/DESIGN.md` 曾被误删致 check:colors 红，已从 HEAD 恢复，门禁与 `.gitignore` 未动。

## 8. 优先级结论

### 立即做

1. Path capability、文件大小限制、ffmpeg 白名单、AI URL 策略。
2. 扫描 partial 保护和后端任务互斥。
3. 播放 generation 防竞态。
4. 统一错误语义和启动恢复 UI。
5. 子进程 kill+wait 与统一 shutdown。

### 随后做

1. mpv 单线程命令队列。
2. 原子落盘、迁移/改名 journal。
3. watcher 单 debounce worker。
4. 曲库 SQL 分页和轻量 DTO。
5. FTS5、搜索下沉和扫描 dirty-set。

### 再做

1. UI 错误恢复、焦点、slider、tabs、live region。
2. QueuePanel 收敛和窄窗口 overflow。
3. B4 字幕、视频封面、AI 余项和其他功能缺口。
4. 分层重构、tracing、性能基线与文档门禁。

## 9. 当前审计口径

已确认完成或已有门禁覆盖的历史整改不再列为当前问题，包括 IPC 基础契约、SQLite 差异写入、UUIDv5/blake3 主链、AI 长任务基础通道、ffmpeg 不随包分发裁决、颜色门禁、透明窗口基础设计、AI-16 导出导入、改名主链 NFC/rejected/后置校验、旧 `rename_dir` 删除与改名流程并轨等。

其中「rejected / 后置校验」两处于 2026-09-24 按源码与规范复核后修正，并补了用例（不是回归，是把口径钉死）：

- **「已拒绝」按名字判**：`rejected_suggestions` 的主键是 `(target_path, suggestion)`，所以判定树第 1 条命中的必须是「这一条**当下那个候选名**被拒过」。预览侧与落地侧共用一条判据（`ai_ledger` 的同一个查询），预览说跳过的那条，落地确实一个字不动；换个未被拒的新译名就不再跳过 —— 上一版按 `target_path` 一刀切，会把「拒过一次」误算成「这条永久作废」（`04 §7.2` 第 1 条）。
- **「可应用」的数**：前端 `isApplicable` 先看档位，`skip`（锁定 / 已拒绝）不进这个数 —— 它可能仍挂着本地字形变化，不排会把确认框里「把 N 个名字改掉」一起数进去。
- **长度收敛剥修饰语后不留脏空白**：`高音质 完全版 名称` 之类的名字剥完会留下连续/首尾空格，那正是判定树第 4 条要修的形态；收法与本地规范化、后置第 8 条同一份实现（`04 §7.3.2` 长度那一行）。
- **括号清单补齐 `[]`**：后置第 3 条的比对清单与「省略后不留空括号」的对表都按 `04 §7.3.2` 那一行取全 `【】『』「」（）()[]`。

**文件系统命令的路径能力（P0-1 的前半）**于 2026-09-24 闭合，不要重新加回来：

- 判据集中在 `src-tauri/src/scope.rs` 的 `PathScope`（`allow_root` / `allow_file` / `revoke_root` / `resolve` / `check_all`）。授权来路只有三条：原生对话框选中的路径（`open_*` 命令返回前当场登记）、应用数据目录与封面目录（`lib.rs` setup 登记）、库里已登记的 `folderPaths` 元数据（应用自己写入的，不是前端递进来的）。
- 判据落在 `canonicalize` 后的真路径上；目标还不存在时按「最长已存在祖先 + 接回尾巴」判定（新建文件、转换输出这些场景要过）；`..` 一律直接拒，不交给 OS 做词法折叠（Windows 会折叠、Linux 不会，赌它就等于两套语义）。
- 过闸门的命令：读类 `read_as_data_url` / `read_text_file` / `file_exists` / `list_dir` / `show_item_in_folder`，以及 `rename_preview` / `rename_apply` / `rename_suggest_names` / `player_play` / `scan_folders` / `scan_folders_incremental` / `startup_sync` / `remove_folder` / `get_folder_covers` / `analyze_loudness` / `start_watching`（AI 侧四条见下方「AI 命令的路径闸门」）。能回 `IpcResult` 的直接回 `PermissionDenied`（`IpcError::outside_scope`），回不了旧形状的记 `[scope]` 日志并返回空值。
- 拒绝粒度分两种：扫描与改名是**整批拒**（「要么全做要么不做」），封面/响度/watcher 是**逐条筛**（一条离线盘上的路径不该拖垮整批）。
- 同批删除 `tools_save_file`（前端零引用 + 任意路径写入器）；`lib.rs` 的 asset 协议范围从整个数据目录收紧为封面目录（递归）+ 背景图单文件，SQLite、设置、日志、`folder-cover-map.json` 不再可被 `asset://` 取到。
- `scope.rs` 自带 9 条用例：根目录连带子目录、空范围全拒、根外平路径与 `..` 分别对应 `Outside`/`Unresolvable`、授单个文件不连带它的兄弟、不存在的目标按父目录判、相对路径与怪路径（`""`、`.`、`\\?\`、`C:`、带 `\0`）全拒、整批拒语义、撤销根之后重新关闭。
- 副产物：`check:ipc` 的「Tauri 注入参数」判据从**形参名**白名单（`app`/`window`/`state`…）改成按**类型**识别（`State<'_, T>`、`AppHandle`、`WebviewWindow`…）。Tauri 本来就是按类型注入，名字白名单既逼着 `State<'_, PathScope>` 必须叫 `state`，又会把恰好叫 `app` 的真实 JS 参数漏检。

**读取限额与原子写（P0-1 的后半）**于 2026-09-24（第 23 批）闭合：

- 限额集中在 `src-tauri/src/limits.rs`。数值：正文/歌词/字幕 8 MiB、图片原始字节 32 MiB、图片像素 64 Mi 像素（防解码炸弹，不是防大图）、目录条数 20000、AI 响应体 16 MiB。语义是**超限拒绝、不截断** —— 截出来的文本会静默变成「另一份歌词」，比读不到更难排查。
- 读入口覆盖：`read_as_data_url`（字节 + 像素双关）、`read_text_file`（字节）、`list_dir`（条数，到第 N+1 条记 `[limit]` 并丢掉）、`scanner/covers.rs` 的外来封面拷贝、`ai_export`/`ai_chunk` 的字幕源文件（严格 UTF-8，坏编码报错而不是静默替换 U+FFFD）、`bgimage` 的 base64 解码与外来图片选择。AI 侧 `ai.rs` 的 `read_body_capped` 先看 `Content-Length` 再按块累计，无长度声明的流式响应也会在读满时停下；对话、失败响应与模型清单三条路径都走它。
- 图片的像素关**认不出格式一律放行**：`image` crate 认得的格式比 WebView 窄，把「这里认不出」当「有问题」会挡掉本来能显示的封面，而认不出的图我们也不解码它。
- 原子写集中在 `src-tauri/src/atomic.rs`：唯一临时名（pid + 进程内计数 + 纳秒）+ flush + `sync_all` + 同目录 `fs::rename` 替换（Windows 上 std 的 rename 走 `MoveFileEx(REPLACE_EXISTING)`，可覆盖）。失败时删掉临时件再报错，不留垃圾。8 类落盘点已接入：`settingsio` 设置、`rename/history` 账本、`scanner/covers` 封面字节与 `folder-cover-map.json`、`ai_chunk` 译文、`ai_export` 写出、`commands/system.rs::ai_export_prompts`、`bgimage`（含 `migrate_oversized`）、`paths::append_log` 的截断重写。日志的**追加**仍是 `OpenOptions::append`（追加语义不需要整份替换），只有超 1 MB 的截断重写走原子写。
- `atomic.rs` 6 条用例（含 4 线程 × 8 次 × 512 KiB 的并发写：最终内容必须是其中某一整份，不能是拼接体）；`limits.rs` 8 条（含 GBK 字节必须报错、1600 万像素必须拒、非图片放行）；`ai.rs` 2 条（到点即停、按块累计）。
- 仍整份读的只剩应用自管的小文件（`settings.json`、`rename_history.json`、`folder-cover-map.json`、日志本体的 1 MB 上限内），量级由自己写入时决定，不在本轮限额范围。
- 副产物：`rename/history.rs`、`rename/batch.rs` 测试里「目录不残留临时件」的断言从固定名 `*.json.tmp` 改为扫描 `*.tmp` —— 临时名带 pid/计数/纳秒，按固定名找已经找不到东西了。

**ffmpeg 白名单与来源验证（P0-2 / P1-12）**于 2026-09-24（第 24 批）闭合：

- 通用执行器消失：命令 `ffmpeg_exec`（收 `Vec<String>`）与 DTO `ExecResult` 从 Rust、契约（`commands.ts`/`api.ts`/`dto.ts`）、桥接与 `docs/IPC.md` 四处一并删除（前端本就是零引用 —— 与 `tools_save_file` 同类的死面，但它是任意参数执行器，留着迟早被接上）。`fftools::exec_ffmpeg` 随之删除。
- 参数侧白名单：`convert_run` 只收 `ConvertItem`（kind / format / path / outPath），命令行由 `fftools::build_convert_args` 在 Rust 侧拼。格式表 `CONVERT_FORMATS = mp3/aac/m4a/flac/wav/ogg`，表外返回 `None`，队列里记一条 `failed`「不支持的输出格式」并继续下一条 —— **没有 `_ => copy` 兜底**（旧实现的兜底会让「未知格式」变成无声重封装）。不做自定义 filter、不做自定义参数，前端传不进任何额外开关。
- 路径侧整批校验：`validate_items` 在开跑前逐条过，任一条不合格**整批拒**（不留半批跑过的现场）。规则：kind 必须 ∈ {convert, extract}；format 必须在表内；path 不属于 `parse_invalid` 的形状（含 `://` 或以 `-` 开头，后者会被 ffmpeg 当选项）；`scope.allows(path)` 且是真实文件；扩展名与模式匹配（音频 11 种 / 视频 7 种）；`outPath` 非空、与源**同目录**、不等于源、扩展名等于所选格式、**目标不存在**。错误分 `Reject::Outside` → `PermissionDenied` 与 `Reject::Invalid` → `InvalidArgument` 两类，都带可读中文。
- 不覆盖：命令行用 `-n`（旧实现是 `-y`）。「目标已存在就拒」是双保险 —— 前端 `uniqueOutputPath()` 靠 `file_exists` 探同目录同名来加序号，而探针要目录被授权才看得到东西，所以 `open_media_files` 在选中文件后**连父目录一起 `allow_root`**（与 AI 翻译页 `open_folder` 同粒度），并写进了 doc comment。
- 源目录锁定：转换输出固定落源文件所在目录，不引入「输出目录」概念 —— 少一个需要授权的面，也避免跨盘移动。
- 来源验证（P1-12）：`paths.rs` 新增 `FfmpegSource`（env / appDir / fixed / path）与 `locate_ffmpeg_ex()`；`FfmpegInfo` 带上 `source` 与 `warn`；启动时 `fftools::log_ffmpeg_identity()` 在后台线程把实际路径、来源、版本与 blake3 指纹（流式 64 KiB 分块，取前 16 位十六进制）写进 `[ffmpeg]` 日志行。提示策略：`KX_FFMPEG` 环境变量与 PATH 命中的**一律提示**；程序目录/固定目录只有位于用户可写目录（USERPROFILE / HOME / LOCALAPPDATA / APPDATA / TEMP）才提示。转换页显示来源提示条（`.cv-note`），不可用时仍是原来的 `.cv-warning` 降级块。
- 副产物（拖入文件根本进不来）：转换页原来的 `<input type=file multiple>` + HTML5 `@drop` 在 Tauri 下是死的 —— `dragDropEnabled` 默认为 true 会拦掉 HTML5 拖放事件，WebView2 里 `File.path` 也拿不到。改为两条正路：点选走 `open_media_files(kind)`（对话框选中即登记授权，不靠前端自报），拖入走 `getCurrentWindow().onDragDropEvent`（OS 级手势）；Rust 侧在主窗口 `WindowEvent::DragDrop(Drop)` 里对每个落下的路径 `scope.allow_file`，前端只负责把路径显示进列表——**授权由事件本身产生，不是前端递进来的字符串**。`open_audio_files` 因此变成 `open_media_files(kind)`（kind = audio / video）。
- `fftools.rs` 新增 10 条用例（表外格式、未知 kind、越界输入、输出必须同目录、目标已存在、扩展名与模式、输出扩展名/不覆盖源、缺失或协议形状输入、空批、合法对通过并断言参数里含 `-n` 不含 `-y` 且不含 `copy`、extract 带 `-vn`、`-i` 后紧跟输入、末位是输出、无 `-ac`/`-ar`），用临时目录树构造真实文件，不碰用户曲库。

**AI 端点地址策略与 Key 生命周期（P0-3）**于 2026-09-24（第 25 批）闭合：

- 端点规则集中在新模块 `src-tauri/src/ai_url.rs`（`check` → `Target { base, host, port, https, local }`）。空串 = 历史默认 `http://127.0.0.1:11434`（本机 Ollama）；没写 scheme 时本机/私网补 `http`、其余补 `https`（**旧实现一律补 `http`**，用户打 `api.deepseek.com` 就是一个明文端点）；显式写了非 http/https 的 scheme 直接拒（`ftp://a` 这类如果去「补」，会变成 `https://ftp://a` 被解析成「主机叫 ftp、路径是 //a」，一个非 http 端点混进来）；地址里带用户名/密码直接拒（凭据只从 API Key 那一栏来）；公网必须 https 且只允许 443（要别的端口请加反向代理）；`base` 去尾斜杠，没有版本段时补 `/v1`。
- 「本机/私网」判据按**主机名**：`localhost` 系（含 `foo.localhost`）、IPv4 回环/私网/链路本地/未指定、IPv6 回环/未指定/ULA（fc00::/7）/链路本地（fe80::/10）。**不做 DNS 解析** —— 域名一律按公网对待，不做 DNS 就没有重绑定面（已知缺口的取舍见本段末尾）。
- 重定向：`reqwest` 客户端挂 `ai_url::redirect_policy()`（`Policy::custom`），逐跳走 `redirect_verdict` —— 最多 3 跳、必须**同源**（scheme + host + port 三者全同，换端口也算跨源）、目标本身再过一遍 `check`；拒了就把原因带回请求错误。跨源会把 `Authorization` 交给另一个主机，软件不替用户做这个决定。注：`reqwest` 的 `redirect` 模块在 `default-features = false` 下仍可用，不受 feature 门限制。
- Key 生命周期：「只在内存、永不落盘」这条规则现在有三层（见 §2 P0-3 段落的证据列）。代码侧：`AiChatPayload.api_key` 与两个任务 spec 各实现 `Drop`（`zeroize` 清零），`ai_chunk::Creds.api_key` 直接用 `zeroizing::Zeroizing<String>`，Authorization 头临时串也用 `Zeroizing` 包；三处 `Debug` 手写、只输出 `ai::redact` 的「未设置 / 已设置（N 字符）」。落盘侧：`settingsio::save` 写入前 `strip_secrets` 递归剥掉名字像秘密的字段（12 个键名，先去掉 `_`/`-` 再比对，大小写无关），命中记 `[settings] 拒绝持久化秘密字段：…`，其余字段照常写入。
- 预检：`Hub::check_ready` 让两个 AI 任务在**开跑前**判地址，不合格直接以 `queue-finished` + `error` 收尾（不留「每个文件失败一遍然后又停」的现场）；`rename_suggest_names` 与 `ai_list_models` 同样先过 `check`。`AiError::Config` 计入 `is_dead_endpoint` 分类 —— 地址配置错与网络错在「别再敲下一个文件」上是一回事。
- 日志：`note_endpoint` 每个 origin 只记一次，内容是「`scheme://host:port` · API Key 已设置（N 字符）」——**只记 origin 不记路径**（反代和自建网关常把令牌挂在路径里，而日志是会被贴出来求助的东西）。
- 前端：`AiApiPane` 接口地址栏下加一行灰色提示（公网必须 https + 443，本机/局域网可 http 并自带端口），口径与后端一致。
- 用例：`ai_url.rs` 10 条（scheme 逃逸、公网 http、非 443、本机/私网/IPv6 放行、空串默认、`/v1` 版本段、路径不进日志、重定向同源与超跳、同源但目标不合规仍拒）、`settingsio.rs` 3 条（顶层/嵌套/数组剥离且保留其它字段、现有设置键全不误伤、大小写与分隔符归并）、`ai.rs` 1 条（配置错归 dead-endpoint，成功才清零失联计数）。
- **未做（明确取舍，留作已知缺口）**：不做「DNS 解析之后再复检 IP」——本策略从头到尾只判主机名，不做 DNS 就没有「解析结果与校验结果不一致」的窗口；在 `reqwest` 的连接池模型下按次复检既拿不到能改写的时机，收益也只在「用户主动把域名指向内网」这一种自伤场景里。

**AI 命令的路径闸门（P0-1 的 AI 侧）**于 2026-09-24（第 26 批）闭合：

- 四条命令接 `PathScope`，与 rename/convert 同口径**整批拒**（任一条不在授权范围内就整批不做）：`ai_translate_start`（`spec.paths`）、`ai_export_prompts`（要读源字幕）、`ai_import_translations`（要写 `.zh.*`，输出与源同目录，所以随输入一起放行）、`subtitle_scan_dir`（`dir`）。判据与日志在 `commands/system.rs::scope_denial`，拒绝写 `[scope] <命令> 拒绝整批：<路径>（<原因>）`。
- `ai_translate_start` 的返回类型是 taskId、回不了错误信封：拒绝走 `ai_job::reject` —— **不登记任务表**（没有东西在跑，`cancel` 也不认它），只发一条与正常收尾**同形**的 `queue-finished` + error（`file_index = 0`、`file_count` = 整批条数）。前端 `startJob` 的「先订阅再启动 + 暂存早到事件」正好覆盖这条路径，UI 不会漏掉原因。
- `subtitle_scan_dir` 不能把 `State<'_, PathScope>` 当入参：Tauri 不接受「带引用的 async 命令返回非 Result」，所以 scope 从 `AppHandle` 取（`open_folder` 的先例），并且在 `spawn_blocking().await` 之前就放掉借用；拒绝 = 空列表 + `[scope]` 日志。
- 不需要闸门的：`ai_texts_start`（纯文本、无路径）与 `ai_ping`/`ai_chat`/`ai_list_models`（只有网络）；`rename_reject_suggestion` 只往拒绝表写一行、不碰盘，也不在闸门范围内 —— 别按「带路径参数」一刀切把它加进去，加了只会让「先移除文件夹、再拒译名」这类流程平白失败。
- 正当来路（2026-10-25 第 38 批起为四条）：`open_folder` 选中的目录（`allow_root`）→ `subtitle_scan_dir` 扫出的路径；`open_text_files` 授权的译文文件；**翻译账里记录过的目录**（`ai_translated::ledger_dirs`：账内 `source_path`/`out_path` 的去重真实父目录，启动时随 `folderPaths` 一起登记——账是应用自己写入的、目录是用户当年对话框授权过的延续，没有它重启后译文管理页的「打开译文 / 重译」全被闸门拒掉，详见 HANDOVER 第 38 批）。
- 用例 2 条：AI 闸门整批拒且**指出越界的那一条**（授权前后各断言一次）；拒绝事件与收尾事件同形（`taskId`/`state`/`fileIndex`/`fileCount`/`error` 齐全，且不带译文）。

**改名与旧 ID 迁移的 intent journal 与启动恢复（P1-11）**于 2026-09-24（第 27 批）闭合：

- 意图目录 `data_dir()/journal/`，落盘与清单集中在新的中立模块 `src-tauri/src/journal.rs`（`put` / `remove` / `list` / `read`）：写走 `crate::atomic::write_str`（唯一临时名 + sync + 同目录 rename），`list` 按前缀筛，目录不存在当空。改名用 `rename-<batchId>.json`（前缀 `rename-`），旧 ID 迁移用 `legacy-ids.json`；**跑完才删，遗留一份文件本身就是「上次没干净收场」的证据**。
- 改名侧 `src-tauri/src/rename/journal.rs`：`apply` / `rollback` / `redo` 三条会写盘的入口都在**动手之前**写下 `Intent { batchId, dir(apply|rollback|redo), startedAt, pairs(执行顺序), roots }`，写不进去就整批不做；`pairs` 与运行时同源同序，恢复靠它重放 `resolve(cur, &moved)` 才能把「父目录先改了名」的换算还原出来。
- 恢复的立场只有一条：**盘是事实，恢复只补账**。启动时绝不替用户继续搬文件 —— 库里记的是「预览那一刻的绝对路径」，崩溃到下次启动之间用户完全可能自己动过盘，照 intent 硬搬才是真丢数据。逐条按现状判：目标在源不在 → 生效（缺行就 `add_item`、旗子反了就 `set_item`）；源在目标不在 → 没生效（不动，计数；若账上却记着已生效则报「账上记着这一条已改名/已回滚，盘上却还叫这个名字」）；**两个都在 / 两个都不在 → 说不清，报出来，盘和账都不动**。
- 批次头两种收尾：`applied_at` 为空（应用根本没跑完）→ `finish` 并标注「应用/回滚/重做（崩溃后恢复）」；非空 → 只翻 `rolled_back` 与说明 —— `applied_at` 记的是**首次应用**，恢复不该把它改写成「现在」。
- 幂等：`history::merge` 认「值没变就不算改动」、`revert` 只撤「值还是本批写的那个」，所以恢复跑两遍与跑一遍等价（「恢复到一半又崩」敢再来一次的前提）。意图文件只在核对成功后才删；批次在库里找不到、文件读不出来、曲库打不开（重装/清库）都**留着下次再试**，并如实上报。
- `move_one` 不再指着两次 `exists` 之间没人插手：新原语 `rename_noreplace` 在 Windows 走 `MoveFileW`（不带 `MOVEFILE_REPLACE_EXISTING`，旧实现的 `std::fs::rename` 会静默覆盖），竞态输给别人的目标一律报到「目标名已被占用，未覆盖」；`Cargo.toml` 给既有的 windows-sys 依赖加一个 `Win32_Storage_FileSystem` feature，不新引 crate。
- 旧 ID 迁移那一半（`db/mod.rs`）：改库之前先把 `{ map, covers }` 写成 `journal/legacy-ids.json`（写不成就不迁），事务里只改库、提交后才搬封面、最后收意图；启动时 `recover_legacy_ids` 只补「旧封面在、新封面不在」这一种（封面是应用自管的 UUID 文件名，不是用户文件），**绝不覆盖已存在的任何东西**，冲突逐条报出。测试用注入的意图目录（`migrate_legacy_ids_in` / `recover_legacy_ids_in`），不碰真实应用数据目录。
- 可见性：恢复结论进 `[重命名]` / `[迁移]` 日志行（带条数与明细），同时经 `state::startup_warn` 在首屏提示（给用户的句子只给条数，明细在日志里）。
- 用例 10 条：`src/journal.rs` 1（写入/读回/前缀筛选/删除，含临时件不残留）；`rename/journal.rs` 6（崩在 `add_item` 之前的补账、半批按现状收尾、歧义条目不进账、回滚半途只翻旗子且不动 `applied_at`、跑两遍零新增、批次不在时留文件下次再试）；`db/mod.rs` 2（崩溃后的迁移被下一次启动补完且收掉意图、恢复不覆盖更新的封面）；`rename/paths.rs` 1（不覆盖改名原语，文件与目录两例，并断言目标内容一个字都没变）。
- 已知取舍（不要当漏洞重新提）：恢复只在启动时跑一次，不提供运行中的手动「重新核对」入口；恢复永远不搬用户文件，所以半批的现场是「账按现状收口 + 首屏提示」，剩下的名字要不要接着改由用户在改名页自己决定。

**播放加载代际竞态（P0-5）**于 2026-09-24（第 28 批）闭合：

- 代际只有一个来源：前端 store 里的 `loadGeneration`，每次 `playTrack` 自增，随 `player_play` 的 `generation` 参数下发（Rust 侧**必填**，不给默认值 —— 少一个调用方就会让整条链悄悄失效）。Rust 只做单调记账（`shared.generation.max(gen)`）与回传，不生成代际。
- 三个带载荷的播放事件统一带上它：`player:state`（`PlayerState.generation`）、`player:loading`（从 `null` 改成 `{ generation }`）、`player:ended`（`{ reason, generation }`）。前端判断只有一条：`payload.generation < loadGeneration` 就是上一代的迟到消息，整条丢弃 —— 迟到状态不进镜像、迟到 `StartFile` 不点亮加载态、迟到 EOF **不写进度也不推进队列**（原症状：一次点歌跳过两首）。
- `loadTrack` 从 `boolean` 收敛成 `'ok' | 'failed' | 'stale'`：超车后的失败不再写 `error`/`loading`（原来旧请求的 reject 会把新曲目按成错误态），`stale` 的收尾一概不做（不查字幕轨、不应用响度、不启动进度写入、不重新声明 vid）。
- 进度回写加第二道闸：`writeProgress` 只在 `reportedGeneration === loadGeneration` 时落盘。「引擎确实为这一代报过状态」是唯一凭据 —— 前端在起播时自己算的 `resumeMs/1000` 不算，否则超车那一刻手里的 position 还停在上一首，写下去就是给新曲目记旧账。
- Rust 侧的真问题不是标签而是**共享状态只有一个**：`play_file` 与 mpv 的属性事件改的是同一个 `PlayerState`，光靠标签分不出「这条 `time-pos` 是谁的」。因此加 `PlayerCore.pending_load` 牌子（`Arc<AtomicU64>`，0 = 无）：`play_file` 先立牌再改状态，事件线程在牌子期间把**文件级属性**（`time-pos` / `duration` / `video-params/dw|dh`）整批丢弃；`StartFile` 与 `EndFile` 收牌（mpv 事件队列是 FIFO，旧文件的属性必然在它的 `EndFile` 之前、新文件的 `StartFile` 之前生成，因此 `StartFile` 一到，后面收到的文件级属性必然属于新文件 —— 不需要在事件线程里回读引擎做补偿，也就不必冒 `get_property` 阻塞的风险）。
- 音量/静音/倍速/暂停**不挡**：它们是跨曲目的传输设置，挡下来会让用户在新曲目开始前调的音量被无声吞掉（单测盯住这一条）。
- 起播失败（`loadfile` 被 mpv 拒绝）时，`play_file` 用 CAS 把牌子收回并清掉 `playing/track_path`：原来 IPC 已经 reject，共享状态却还写着「正在播放本条」，250ms 的状态定时器会把 UI 重新点成播放中。CAS 而非直接清零 —— 更新的代际若已接手，牌子是它的。
- 用例 6 条（`tests/player-generation.test.ts`，把 `@/bridge/ipc` 换成受控桩，`playerPlay` 的承诺由测试自己决定何时兑现）：连续切歌时旧失败不改错误态且不按灭加载态、旧加载迟到成功不做响度收尾、旧 `player:state` 一个字段都不进、旧 `player:ended` 不推进队列也不写进度、旧 `player:loading` 不点亮加载态、引擎未报过状态时 `seek` 不落盘。Rust 侧 3 条纯函数用例盯住「挡什么、不挡什么」。
- 已知残留（属 P1-2，第 30 批已闭合，见下）：两条 `player_play` 谁先到达 mpv 由 Tokio 阻塞池的调度决定，本批只保证「状态不串代」，命令本身的串行化（专用命令线程 + 有界队列）当时留到 P1-2；`loading` 在 `play_file` 复位状态到达时仍会瞬时熄灭一次（既有的观感问题，与本批的竞态无关）。

**扫描 incomplete 状态与后端互斥（P0-4 / P1-1）**于 2026-09-24（第 29 批）闭合：

- 症状是「一次没扫完的扫描会把曲库删空」：`save_snapshot` 写的是整库差异 —— 本轮没见到的曲目、目录与归属一律删。而「没见到」有好几种来路（发现阶段的 30 秒上限、`MAX_DIR_COUNT` 2000 目录上限、10 万文件上限、离线或权限变化读不出来的根、被更新的扫描取代），每一种都能把真实存在的条目当成本轮删除。
- 收口状态随结果走：`ScanResult.scan: ScanStatus`（`outcome` = `complete`/`partial`/`cancelled`/`failed` + `reasons`），字段带 `#[serde(default)]`，老形状的载荷仍按 `complete` 解。**只有 `complete` 允许执行删除**。
- `db::library` 的写入分两种模式：`WriteMode::Full`（原行为）与 `WriteMode::NoDelete`（`save_snapshot_no_delete`）。后者照常 upsert 目录树/曲目/艺术家/专辑，但跳过归属删除、曲目删除、目录删除与两处孤儿回收；`write_diff` 的删除段全部收在 `if mode == WriteMode::Full` 里。`fileCount`/`scannedAt` 也只在 Full 下改写 —— 它们记的是「最后一次完整扫描」的事实，只补不删的一轮没有资格改；`folderPaths` 照写（那是用户配置，不是扫描结论）。
- 部分/取消的结果返回的是**库里合并后的真相**，不是本轮的半成品列表：`settle_scan` 落库后 `load_snapshot` 回读（回读不出来才退回本轮结果）；取消时也回读现有快照、只把状态改成 `cancelled`。前端因此永远不会因为一次部分扫描而看到少掉一半的曲库。
- 互斥做在 **Rust 侧的全局闸门**（新模块 `scanner/gate.rs`：`ScanGate` / `Ticket` / `begin` / `finish` / `if_current` + 全局 `GATE`），**不是在根目录维度上**：`save_snapshot` 的差异是整库口径，两个根同时扫、后落库的那个会把另一个根的曲目全部当成「本轮没见到」。新扫描 `begin()` 时把在飞的所有票标记为被取代（latest-wins），旧的提交一律落空。前端 `scanGeneration` 仍然只管 UI 显示，不再是库的守卫。
- 「谁是最新的」与「谁有权写库」不许拆开：`if_current` 在闸门互斥锁**持有期间**执行提交闭包 —— 新扫描要么在提交前拿到锁（把旧票作废）、要么在提交后拿到（自己成为最新），不存在「校验通过了、写的时候已经被取代」的窗口（有一条用例专门断言提交进行中新扫描拿不到锁）。
- 取消同时是省 CPU 的手段：`Ticket::cancel_flag()`（`Arc<AtomicBool>`）穿进 `discover_files` 与 `parse_pool` —— 发现阶段在遍历循环里逐轮查（`'walk` 标号跳出），解析池的 worker 不再领新文件（正在解析的那个只能等它自己回来）、watchdog 循环随之中止。被取代的那一轮在发现后与解析后各查一次闸门，命中就直接回 `cancelled`（不落库）。
- `startup_sync` 的「无变更就早退」也带状态：早退前把发现阶段攒下的 `ScanStatus` 写进返回的快照，否则一次部分遍历会以「已是最新」的口吻收场。`remove_folder` 同样先 `begin()` 取票（它写的是用户明确要求的完整快照），避免在飞的旧一轮在摘除之后又把库里写回去。
- 前端（`stores/library.ts` + `features/library/ContentArea.vue`）：`cancelled` 的结果一个字段都不进（它是被超车那一轮的遗留，收下就是用旧快照回退曲库），也不弹提示 —— 被超车是正常收尾；`partial`/`failed` 照常应用（库里已经合并好了）并弹出可关闭的提示条，失败说「曲库保持原样，可稍后重试」，未完成说「已扫到的照常更新，没扫到的条目继续保留」。提示条配色只用 `--warning` 的 `color-mix` 与文本变量（不写颜色字面值），`check:colors` 的字面值计数不变。
- 用例 18 条：`scanner/gate.rs` 7 条（新票是当前的、新扫描取代旧的、最新者活到最后、过期票收工不干扰当前、只有当前票能跑提交步、提交进行中新扫描拿不到锁、取消旗子跟着票）；`scanner/mod.rs` 8 条（健康的树算完整、读不出来的根算不完整、取消后不接着走、被取消的解析池不报完整、整轮算完整、部分扫描保住没见到的条目并回合并库、完整扫描照旧删、写失败报 failed 且曲库不动）；`db/library.rs` 3 条（只补不删保住每一条没见到的行、该插该更新的照做、不碰完整扫描的 fileCount/scannedAt）。前端 6 条在 `tests/scan-status.test.ts` 盯住四类状态各自的取舍与手动收起。
- 已知取舍（不要当漏洞重新提）：部分扫描**照样写库**（只补不删），不是「整轮作废」。3000 个目录或离线盘上的大库会长期停在 partial，若拒绝写入就永远更新不了；代价是一次部分扫描里被改名的文件可能留下一条幽灵行，等下一次完整扫描收掉 —— 「新增」与「删除」的取舍不对称，是有意的。
- 已知残留（属 stage 3 曲库规模化，别当本批漏洞重新提）：`ScanStatus` 只报类别与理由串，不报「哪个根没扫完」；多根场景下用户需看 `[scan]` 日志行里的明细。

**mpv 单一命令线程与有界命令队列（P1-2）**于 2026-09-24（第 30 批）闭合：

- 症状是「同一时刻好几个线程各自对 mpv 说话」：`player_play` 与各 `player_set_*` 由 Tauri 的调度线程执行、属性事件由 `event_loop` 自己的线程收，`set_property`/`command` 虽然线程安全但**顺序无人保证**，而 mpv 只按收到的顺序执行。连点两次播放、拖音量连发几十条 `volume`、换曲与暂停交错，都可能以意外顺序落到引擎上。`embed.rs` 的舞台几何队列是同一性质，且无界累积、逐条 `SetWindowPos`。
- 新模块 `src-tauri/src/player/queue.rs`：泛型 `CommandQueue<C>`（生产环境 `C = Arc<Mpv>`；测试用假上下文驱动**同一条**队列，顺序/合并/上限这些性质不必启动真 mpv 就能钉住）。**一条**命令线程按 FIFO 执行；队列有硬上限（`DEFAULT_CAP` 32），满了等空位、等 5 秒仍满就报 `Busy`「播放器命令队列拥堵」，不无声丢弃；投递有 10 秒回执上限，`submit` 拿到的仍是 mpv 的真实结果（语义与改造前一致，前端 `bridge/ipc.ts` 的 `catch` 一处没动）。
- 合并键只作用于**排队期间**的作业：同键只留最后一条（`volume`/`mute`/`speed`/`pause`/`sid`/`sub-visibility`/`sub-delay`/`sub-style`/`volume-gain`/`vid`/`audio-device` 都带键；换曲、`seek`、`stop` 是 `None`，决不合并）。正在执行的那条不受影响（它已脱离队列），所以合并吃掉的只是「还没轮到、且已被后来的值取代」的中间态。`set_sub_style` 与 `set_subtitle_track` 各自**整份**下发：分开发会被滑块的中间态插进来，落到 mpv 上是一份混合样式。
- 两道护栏：命令线程内再投递（作业里等自己执行完）由 `IN_WORKER` 标记当场拒绝，报错而不是挂住；单条作业 panic 由 `catch_unwind` 接住、记日志、线程继续服务，投递方因回复通道断开立刻拿到「命令线程已退出」，不必干等超时。
- `PlayerCore` 只剩一个命令入口：`cmd: Mutex<Option<Arc<CommandQueue<Arc<Mpv>>>>>` **同时**是「mpv 是否就绪」的唯一判据（原来「有 mpv 句柄」与「已初始化」是两份状态，「一半就绪」最坏 —— 命令发不出去、读属性却以为自己能读；`host`/`main_hwnd` 两个只写不读的 `OnceCell` 一并删掉）。读属性（`subtitle_tracks` / `refresh_devices`）也排队：一边 `loadfile` 一边读 `track-list`，拿到的是半截状态。
- 初始化可回滚（`ensure_started`，`start_lock` 串行）：宿主窗口 → `Mpv::with_initializer` → 命令队列 → 事件线程，四条失败路径逐条拆干净（`embed::destroy()`、`queue.shutdown()`），只有全成功才把 `cmd` 置位并记 `[mpv] initialized with embedded window`。顺序上**先起事件线程、后公开 `cmd`**：观察属性/开事件由事件线程独占完成，而「公开 cmd」= 允许用户命令进来，反过来会让 `observe_property` 落在第一条 `loadfile` 之后，那一代的属性整段漏收（进度与画面尺寸永远不动）。
- 舞台侧（`player/embed.rs`）：入队策略抽成纯函数 `push_cmd` —— `Rect` 是绝对几何，排队中的旧矩形直接换掉（拖一次窗口能发几十条，全留着只会让画面追着历史位置跑）；`Visible`/`Attach`/`Destroy` 有先后语义，一条都不合并；队列仍有 64 条硬上限，超了丢最旧的一条并记 `[stage]` 日志。宿主窗口句柄从全局 `OnceCell` 改成**线程局部** `MY_HOST`（换代时全局会先被新的一代覆盖，而旧线程还在抽队列）；`destroy()` 之后 `start_stage_thread` 先等上一代的退出信号（`EXIT_RX`，等不到只记日志、不无限等）再建新一代 —— 这是「回滚之后还能重来」的前提：原来 `HOST` 是 `OnceCell`，重来那一次的新窗口注册不进去，命令会 `PostMessage` 给一扇已销毁的窗口（表现为黑屏，且说不清画面归谁）。
- 顺带修掉两处由队列暴露出来的时序问题：`FileLoaded` 的 `start` 复位改为**有起播在途时不做**（连点两首时，这条复位会插到第二首的 `loadfile` 之前，把它的续播起点抹掉），复位本身走 `post`（不等结果），事件线程不被一条 mpv 调用扣住；`emit_state` 的节流时钟从「每条线程一个 `thread_local`」改成进程级 `AtomicU64`（`last_emit_ms`）—— 两条线程写同一条 `player:state`，各记各的时钟等于没节流。
- `destroy_host()`（退出路径）先 `post` 一条 `quit` 再 `shutdown()`：事件线程收到 `Shutdown` 自行退出，退场过程中再来的 `player_*` 命令当场拿到「命令线程已退出」，不排进一条即将消失的线程。
- 用例 13 条：`player/queue.rs` 10 条（200 条顺序一条不丢、同键只跑最后一条、不同键互不合并、无键命令一条不少、满队列报拥堵不静默丢、回执是真实结果、panic 隔离且线程存活、作业内自锁投递被拒、读查询走同一条线程、关停前排队的照跑完且不再收新活）；`player/embed.rs` 3 条（矩形只留最新、合并矩形不动它周围的离散命令、超限丢最旧并如实报告）。
- 已知取舍（不要当漏洞重新提）：`submit` 的 10 秒回执上限意味着超时后 UI 会看到「播放器命令超时」，而那条命令可能仍在 mpv 里执行 —— 旧实现是无上限地阻塞 IPC 线程，那更糟；合并只对**排队期间**的同键作业生效，正在执行的那条不会被撤销（撤销一条已经进了 mpv 的命令要再发一条反向命令，顺序更难说清）；本批不碰 stage 3 的曲库规模化；退出路径曾是 P1-3 的待办，**已于第 33 批闭合（见 P1-3）** —— 本批只把拆解做干净（`destroy_host()` 先 post `quit` 再 `shutdown()`），它现在是收尾七步的最后一步 `PlayerCore::shutdown`。

**watcher 单 worker 与脏路径事件（P1-5）**于 2026-09-24（第 31 批）闭合：

- 症状是「一次整目录拷贝在 watcher 里起上千条线程」：notify 回调**每来一个事件**就 `thread::spawn` 一条「睡 1 s 再看自己是不是最后一个」的线程；`stop()` 只丢 watcher、不推进 `debounce_gen`，所以 stop 之前已经开跑的那条线程照样会在 1 s 后 emit —— 换根/移除目录之后旧事件还能打进新一代；`recommended_watcher` 与 `watcher.watch()` 的错误分别被 `Err(_) => return` 与 `let _ =` 吞掉，`start_watching` 无条件 `return true`。
- 改成**一条**常驻 worker 线程（`fs-debounce`）：notify 回调只把过了 `matters` 的路径 `send` 进 channel，worker 收进 `BTreeSet<PathBuf>`（去重 + 稳定顺序），闲时阻塞在 `recv()`（不占 CPU），拿到第一条后开一个 1 s 窗口继续收，窗口到点或攒满 `MAX_BATCH`（1024 条）就发一批。**持续写入不会饿死事件**（旧实现里每条新事件都把「谁是最新的」往后推，靠每条线程独立计时；新实现按窗口发），**内存有界**（一批最多 1024 条，不再可能攒进几万条路径）。
- 代际做在 `WatcherState.gen`（`start` 与 `stop` 都自增）：worker 发出前最后比对一次，过代就整条退休、连已经攒好的那批一起作废。`stop` 之后攒着的那批**不发**是有意的 —— 所有调用方（换根、移除目录、退出）在 stop 之后要么显式重扫一遍、要么本来就不再需要刷新，旧事件不该在新一代里冒出来。断开（sender 随 watcher 一起 drop）同样是「不发、退休」，所以 stop / 重启之后不留僵尸线程。
- 错误返回：`watcher::start` 变成 `IpcResult<Vec<String>>`（返回**挂不上的根**），`Err` 只留给「监听器根本建不起来」；`start_watching` 变成 `IpcResult<bool>`（全部挂上才是 `true`）并逐条记 `[watcher] 挂不上 <路径>：<原因>`，一个根都没挂上时连 worker 都不起、不留半个监听器。挂不上的根通常是离线盘 —— 那是**数据**不是异常，所以既不整体报错，也不影响其它根继续挂着。
- `scanner:fsChanged` 的载荷从 `null` 换成这一批的**脏路径**（`string[]`，已去重、最多 1024 条）：今天前端 `App.vue` 仍只把它当「有变化」用（`library.onFsChanged()` 走既有的全库增量重扫，行为一个字没变），第 3 阶段的「扫描消费 dirty path」直接从这条载荷接。契约三处同步：`contracts/events.ts`、`contracts/api.ts` 的 `onFsChanged` 签名、`docs/IPC.md`（`npm run gen:ipc` 重新生成）。
- 用例新增 8 条（全在 `watcher.rs`）：突发合成一批且重复路径只留一条、持续流跨多个窗口按批发出且一条不丢、满批不等窗口（窗口给 60 s，能在窗口前出来只可能是满批先发）、watcher 消失时攒着的那批作废且 worker 退休、过代 worker 一个事件都不发、闲时 worker 在 watcher 走后干净退出、`start` 如实回报挂不上的根（临时目录下的不存在路径）且不装半个监听器、真 watcher + 真文件系统端到端（建临时目录、写 `.flac`、脏路径到 sink）。worker 的 sink 可替换（生产是 `emit_fs_changed`，测试是记账闭包），这些性质不必启动 Tauri 就能钉住。
- 已知取舍（不要当漏洞重新提）：`stop` 丢掉攒着的那批（理由见上）；一批上限 1024 条，超过就分批发 —— 不截断、不丢条；`matters` 的「没后缀当目录」口径不变（第 12 批的结论，代价是名字里带点的文件夹改名会漏一次自动刷新，界面「重新扫描」兜着）。**P1-6 的残留**：前端仍是 fire-and-forget（`bridge/ipc.ts` 的 `.catch(() => false)` + store 里的 `void api.startWatching(...)`），挂不上监听暂时只有日志，统一 UI error/retry 状态在 P1-6 做。

**ffmpeg/ffprobe 子进程的超时、输出上限与退出回收（P1-4）**于 2026-09-24（第 32 批）闭合：

- 症状有三处：`fftools.rs::exec_async` 超时后只 `start_kill()` 就走人（两种平台的 kill 都是**异步生效**，不 wait 就没有回收，退出时留下还在写盘的 ffmpeg）；`kill_all()` 是空实现（注释还写着「由 OS 回收」——父进程没了它照样活着）；`ffprobe.rs::run_probe` 用 `Command::output()` 整份收 stdout，既无字节上限也无超时，卡住的 ffprobe 会让扫描线程永远等下去。
- 统一入口 `fftools.rs` 的 `run`（`exec_async` 与新的 `exec_capture` 都走它）：`spawn_hidden` 起进程后，`select!` 等三件事 —— 正常结束（读流 + `wait` 一起 `join!`）、超时、被 `kill_all` 通知。**超时与被杀两条路都走 `kill_and_reap`：`start_kill()` 失败（已经退出）也照样 `wait()` 收状态，成功则 `wait()` 带宽限期等它真的退**，宽限期内没退只记日志、绝不无限等。
- 在飞登记表 `LIVE`（`id → {Notify, done}`）+ 退出闸门 `SHUTDOWN`：`Child` 只有 `&mut` 接口，跨线程去杀它必须让**持有它的那个任务自己**动手；也不按 pid 杀（pid 会被系统回收，误杀别人的进程比不杀更糟）。所以 `kill_all` 做三件事：在同一把锁下置位闸门并取快照（此后 `register` 一律失败 → 不会再有新子进程）、置位所有转换队列的取消旗标（否则杀掉当前这个之后循环会接着起下一个）、逐个 `notify_one` 并**等回执**（有 5 秒总上限）。`Notify` 会存下许可，所以「先登记后 spawn」让通知不丢 —— 就算 `kill_all` 挤在 spawn 之前，任务一进 `select!` 立刻走 kill 分支。
- 输出上限分两种收法（`OutCap`，常量在 `limits.rs`）：`Tail(window)` 是滑动窗口，读完就丢前面的字节、**管道照常抽干**（提前不读会让子进程堵在写满的管道上），ffmpeg 的报错/ebur128 汇总都在尾部，用 64 KiB；`Head(max)` 用于 ffprobe —— JSON 在**开头**，超过 4 MiB 就**整份清空并标记**，调用方按「读不到」处理（半截 JSON 解析不了，比读不到更难查）。`read_stream`/`read_err` 合并成一个泛型的 `read_capped`。
- `ffprobe.rs` 去掉自建运行时：改成 `probe_argv(probe, path)` 纯函数（路径仍是最后一个独立参数，`-v error -print_format json -show_format -show_streams -i` 全固定）+ `exec_capture(..., 30 秒, Head(4 MiB), Tail(64 KiB))`；只有「异常」几类记日志（超时/被杀/超限），非媒体文件探不到是扫描常态、不刷日志。转换/响度/版本探测的调用点只换了超时文案 —— 旧实现无论给多少秒都报「ffmpeg 超时(600s)」（响度是 300 s、转换是 3600 s，那句话是错的）。
- 顺带收尾：`commands/library.rs` 的响度队列在取消判定里加上 `fftools::shutdown_requested()`，退出时不再逐个文件起新探测。
- 用例新增 9 条：`fftools.rs` 8 条（系统自带命令当子进程，不依赖 ffmpeg）——正常退出报码与输出且摘表、`Tail` 只留最后 6 字节并如实标记、`Head` 超限**整份丢弃**而不是留半截、程序不存在报「启动失败」且摘表、超时 300 ms 对 30 秒的子进程**到点就回**且回来后表里不留档、`kill_all` 通知在飞子进程自己 kill+wait 且等回收完才返回、空表时 `kill_all` 是 no-op、闸门关上后不再出生新子进程；`ffprobe.rs` 1 条（`probe_argv` 的路径只占一个元素、紧跟 `-i`）。会起真进程又动全局闸门的用例用一把测试锁串行跑，`kill_all` 用后把闸门放回原状。
- 已知取舍（不要当漏洞重新提）：`kill_all` 有 5 秒总上限，宁可记一条「退出时仍有子进程未确认回收」也不能被一个卡死的子进程拖住退出；`SHUTDOWN` 一旦置位在本进程内不再放行（只有退出路径调它，两个调用点后面都是 `app.exit(0)`）；`Tail` 窗口只保证内存有界、不保证「输出完整」，转换失败提示仍只取最后 3 行。**P1-3 的残留（已于第 33 批闭合，见下一条）**：退出曾是「300 ms 延时 + `kill_all` + `app.exit(0)`」，mpv/stage/scanner/watcher/AI/写盘各自拆解，统一 supervisor 见 P1-3。

**统一 shutdown supervisor（P1-3）**于 2026-09-24（第 33 批）闭合：

- 症状是「退出靠时间猜」：三条退出路径各写各的 —— `force_close_window` 与托盘「退出」是「置位 `force_close` → 广播 `beforeClose` → 睡 250/300 ms → 关窗或 `app.exit(0)`」，`app_force_quit` 是「睡 300 ms → `kill_all` → `app.exit(0)`」，而**窗口标题栏的关闭按钮**走的是 Tauri 默认关闭（`RunEvent::ExitRequested` 直接放行）——那条路上收尾一步都不做：mpv 命令队列里排着的活、在飞的扫描/响度/AI/转换、挂着的 watcher、前端防抖中的设置写盘，谁先谁后全靠运气；「睡 250 ms」在慢盘和大库上既可能不够、又白等。
- 新模块 `src-tauri/src/shutdown.rs`：退出**唯一入口** `request_exit(app, reason)`，`claim()`（`EXITING.swap(true)`）保证整条流水线只跑一次、重复请求当场返回。四条来路全部汇进来：托盘「退出」、`app_force_quit`、`force_close_window`，以及 `lib.rs` 的 `RunEvent::ExitRequested`（窗口按钮与系统会话退出的那条）—— `!exiting()` 时 `api.prevent_exit()` 并改走流水线；流水线末尾的 `app.exit(0)` 会再触发一次 `ExitRequested`，那时 `exiting()` 已为真、放行。
- 七步固定顺序（`build_pipeline`）：**停止接收 → 取消在飞任务 → 停文件监听 → 等任务收尾 → 杀子进程 → 等前端落盘 → 收线程**。顺序不是风格：先关受理闸门（`accepting()=false` + `fftools::close_gate()` + `force_close=true`）让新活根本进不来，再取消在飞的，最后才杀 —— 反过来的话，杀掉当前子进程之后，转换/响度那边还在跑的循环会接着起下一个。
- 「等任务」有 3 s 预算（10 ms 一拍），判据是各模块自报的**在飞计数**：`scanner::gate::GATE.live_count()`、`ai_job::running()`、`fftools::convert_tasks()`、`commands::library::loudness_running()`。到点没等到就记一行「仍在跑：扫描×n / AI×n / 转换×n / 响度」，然后照常往下走 —— 退出绝不无限等，超时**如实记账**而不是假装没人跑。
- 「等前端落盘」把「睡一会儿再假设写完」换成**回执制**：`flush_frontend` 先 `emit("window:beforeClose")`（主窗口已不在就跳过并记一行），再等新命令 `shutdown_ready(ok)` 带来的裁决（`READY` 条件变量，2 s 预算），日志区分三种收场：`已确认落盘` / `回执说没写成功` / `等不到回执`。前端 `App.vue` 的 `saveOnce` 改成 `flushOnExit().then(ok => api.shutdownReady(ok))` —— `stores/settings.ts` 的 `flushOnExit` 现在 `await api.saveSettings(s)` 并**返回它的结果**（抛错也回 `false`），所以回执是真实结果，不是「命令发出去了」。
- 「收线程」等 mpv 命令线程**真的退场**：`CommandQueue` 加 `stopped` 旗标（`worker_loop` 收到 `Shutdown`、排空队列后置位再返回）与 `wait_stopped(budget)`；`PlayerCore::shutdown(budget)` = `destroy_host()`（先 post `quit`、再 `shutdown()`）+ 等旗标（2 s 预算）。这是第一次能看到「命令线程已退出」这个事实，而不只是「命令已经投出去了」。
- 每一步都记账：`Pipeline::run` 逐条 `catch_unwind`，**单步 panic 不中断后续步骤**（收尾途中最不该做的就是半路停下），结束后日志一行「步骤名 + 耗时 ms」（panic 会标明），末行「收尾完成，共 N ms，退出」。整条路径的时间上界可算：3 + 2 + 2 秒 + 子进程回收上限（`kill_all` 5 秒）+ 开销。
- 退出途中的新请求一律当场拒绝、不排队：`convert_run` 回 `Busy`「应用正在退出，未开始转换」，`scan_folders`/`scan_folders_incremental`/`startup_sync` 回一份 `cancelled` 的**既有快照**（`scanner::cancelled_for_exit`：库里已有的事实照给，只是不扫），`start_watching` 回 `Busy`「应用正在退出，未开始监听」，`analyze_loudness` 回 `false`，`fftools` 的闸门让新子进程根本不 spawn。
- 用例新增 13 条：`shutdown.rs` 7 条（步骤按序跑、单步 panic 不中断后续、退出流水线只跑一次、等不到信号按预算放弃、信号一到立即放行、前端回执带裁决放行等待、没有回执就没有裁决 —— 用 `Pipeline`、`wait_until`、`wait_ready` 的公开面驱动，不必启动 Tauri）；`scanner/gate.rs` 1 条（`cancel_all` 作废全部在飞票、`live_count` 跟着起落）；`ai_job.rs` 1 条（`cancel_all` 标记每个登记任务、`running` 计数）；`fftools.rs` 2 条（`close_gate` 只关闸门：在飞子进程照跑、新的生不出来；转换队列的取消位与条数）；`player/queue.rs` 2 条（`wait_stopped` 只在 worker 排空退场后报 true、闲着的 worker 在 `shutdown` 后立刻退休）。前端 `tests/shutdown-ack.test.ts` 4 条盯住 `flushOnExit` 的三种结果与 `shutdown_ready` 的参数键名。
- 已知取舍（不要当漏洞重新提）：预算都是**上限不是保底** —— 3 s 内没跑完的扫描/AI/转换会被记一行然后放下（活下次启动重来，`kill_all` 的 5 秒同理）；`shutdown_ready` 的回执在真关闭窗口的 `beforeunload` 那条路上多半送不到，日志记「等不到回执」，这是**已知的第三种收场**、不是失败；本批不引入「保存并退出」这类需要用户二次确认的交互；`window:beforeClose` 仍是单向前端广播，前端不再有否决权（要继续等它也只会等到 2 s 预算用完）。


若后续代码回归，重新加入问题时必须附当前源码、测试或门禁证据。

