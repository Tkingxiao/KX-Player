---
project: KX-Player
register: product
aesthetic_direction: technical / utilitarian + aurora dark
color_strategy: restrained
design_system: bespoke (Vue 3 + scoped CSS + CSS variables)
design_variance: 5
motion_intensity: 4
visual_density: 7
---

# KX-Player Design Language

## Design Read
"A focused, low-light cockpit for long-form ASMR media — quiet surfaces, one warm accent, density where it matters, and motion that never interrupts sleep."

## Signature
The **aurora progress thumb**: a soft, warm accent glow on the playback progress bar and a subtle top-edge sheen on the active sidebar item. It says "this is playing now" without shouting. Everything else stays muted so the media stays central.

## Inspiration
- PotPlayer / mpv: control density, immediate access, no decorative chrome.
- foobar2000: information-rich lists that still feel light.
- Rejected: generic glassmorphism, purple/blue tech glow, oversized hero imagery.

## Color (locked)

**本表的 hex 列是权威值**：`src/styles/theme.css` 逐字实现它，两侧不一致时以 hex 为准并改 theme.css。OKLCH 列由 hex 按 CSS Color 4 反算，精度取到能原样往返，只是设计语言的锚点，**不是第二事实源** —— 照着 OKLCH 重新生成颜色会挪动色值。
`npm run check:colors` 每批门禁对账三件事：theme.css 的每个字面色值必须能在本节找到 role；本节的每个 hex 必须真被 `src/` 用到；`src/styles/` 之外不许出现游离色值（accent 字面量一律禁，例外要在脚本的例外清单里写明理由）。

2026-09-22 第 16 批订正两处本文件自身的失实：① 原 OKLCH 列 **14 行全部与自家 hex 对不上**（`#ffffff` 写成 `C=0.006`、`#0d0d12` 的 L 写 13% 而实测 16.2%）；② 原句「中性色在深色主题偏暖 hue ~30、浅色主题偏冷 hue ~260」不成立 —— 两侧中性色实测都在 **hue 285–286**，~29 是 accent 的色相，与中性色无关。

### 色板成员

| role | OKLCH | hex | use |
|------|-------|-----|-----|
| Dark bg | oklch(16.2% 0.01 285) | #0d0d12 | app background |
| Dark surface | oklch(20.4% 0.016 285 / 0.92) | #16161e | cards, panels |
| Dark surface-elevated | oklch(23.1% 0.019 285 / 0.98) | #1c1c26 | popovers, modals |
| Dark text | oklch(95.7% 0.007 286) | #f0f0f5 | primary text |
| Dark text-sub | oklch(78.6% 0.014 286) | #b8b8c2 | secondary text |
| Dark text-muted | oklch(54.2% 0.019 286) | #6e6e7a | hints, disabled |
| Dark border | oklch(100% 0 0 / 0.10) | rgba(255,255,255,0.10) | dividers |
| Dark border-strong | oklch(100% 0 0 / 0.18) | rgba(255,255,255,0.18) | focus, active |
| Accent | oklch(61% 0.21 29) | #E63A2E | active state, progress, play button |
| Accent glow | oklch(61% 0.21 29 / 0.35) | rgba(230,58,46,0.35) | sheen on active items |
| Success | oklch(74.1% 0.1642 153.3) | #40c878 | completed, success toast |
| Warning | oklch(81.5% 0.138 81) | #f0b84d | attention |
| Danger | oklch(66.9% 0.159 26) | #e6685f | destructive actions |
| Light bg | oklch(96.3% 0.007 286) | #f2f2f7 | light mode background |
| Light surface | oklch(100% 0 0 / 0.92) | #ffffff | cards, panels |
| Light text | oklch(20.7% 0.01 286) | #17171c | primary text |
| Light text-sub | oklch(34.9% 0.003 286) | #3a3a3c | secondary text |
| Light text-muted | oklch(54.2% 0.019 286) | #6e6e7a | hints（与 Dark text-muted 同值，但浅底 4.51:1 / 深底 3.85:1，见「对比度」） |
| Light border | oklch(0% 0 0 / 0.10) | rgba(0,0,0,0.10) | dividers（浅色侧） |
| Light border-strong | oklch(0% 0 0 / 0.18) | rgba(0,0,0,0.18) | focus, active（浅色侧） |
| Light success | oklch(52.6% 0.1368 152.5) | #008041 | 浅色主题的完成态 |
| Light warning | oklch(54.8% 0.1178 71.1) | #9b6300 | 浅色主题的提醒态 |
| Light danger | oklch(56.1% 0.1728 27.5) | #c63f37 | 浅色主题的破坏态 |
| Ink on bright | oklch(0% 0 0) | #000000 | adaptive ink on wallpaper when composite luminance > 128 |
| Ink on dark | oklch(100% 0 0) | #ffffff | adaptive ink on wallpaper when composite luminance <= 128 |

**Light 语义变体的求法**（不是「换个更深的红绿」，是可复算的规则）：保住色相与彩度、只把 L 往下压到 4.5:1。三组浅色变体本来就在 theme.css 里、只是本节没有行，而它们压在 `#f2f2f7` 上实测只有 **3.10 / 2.53 / 3.88:1**，仍不达 AA；把深色版直接放浅底更差（1.93 / 1.61 / 2.89）。也就是说「要变体」这个判断一直是对的，只是停在了半路。本批按上述规则得到的值见上表三行。

### 半透明表面（同一主题的抬起层）

三处常驻栏和弹窗有自己的近不透明底，所以它们**参与**色板、各有 role；只是它们的角色不是「文字承载面」，而是「压在背景图上仍读得出层次的表面」。α 就是它们各自的最终不透明度（运行期还能被 `--tb-alpha` / `--sb-alpha` / `--pb-alpha` 再乘一次）。

| role | OKLCH | hex | use |
|------|-------|-----|-----|
| Dark titlebar | oklch(16.7% 0.012 285 / 0.78) | #0e0e14 | 标题栏 |
| Dark chrome | oklch(17.6% 0.012 285 / 0.86) | #101016 | 侧栏（α .86）· 弹窗底（同一 hex，α .98） |
| Dark player bar | oklch(19.5% 0.016 285 / 0.95) | #14141c | 播放条 |
| Dark surface-hover | oklch(24.1% 0.023 285 / 0.96) | #1e1e2a | 卡片 hover（比 surface-elevated 再亮一档，是「可点」的信号） |
| Light titlebar | oklch(98% 0.005 286 / 0.80) | #f8f8fc | 浅色标题栏 |
| Light elevated | oklch(99.2% 0.004 286 / 0.99) | #fcfcff | 浅色弹层（α .99）· 浅色弹窗底（同一 hex，α .98） |

弹窗底与 surface-elevated 是两个角色，别合并成一个：前者是模态的底（几乎不透明、要盖住整屏内容），后者是浮层菜单的底。

### 结构层（按公式取，不单列 hex）

分隔线、hover 泛光、遮罩、滚动条这些不是独立色板成员，定义 = **主题前景色 + α**：深色主题前景取白，浅色主题取黑。表里已有的 border 两行就是这条公式的成员。

| 用途 | α（深 / 浅） | 落地变量 |
|---|---|---|
| input / active | .08 / .06 · .10 / .06 | `--bg-input` `--bg-active` |
| hover 泛光 | .06 / .04 | `--bg-hover` |
| 选中 | accent 的 α .16 / .12 | `--bg-selected` |
| 模态遮罩 | .55 / .35 | `--modal-overlay` |
| 滚动条 | .14 / .18 | `--scrollbar-thumb` |
| 阴影 | 见 Scales → Shadow | `--shadow-card` `--shadow-pop` |

### 恒暗媒体面与平台常量

不随主题变，但同样是字面值，所以在本节登记；「值即某已有 role」的不再另立 hex。

| role | hex | use |
|------|-----|-----|
| Stage black | #000000（= Ink on bright） | 视频舞台底：P0-3 的字母箱适配依赖纯黑 |
| On media / On accent | #ffffff（= Ink on dark） | 封面与 accent 填充之上的前景 |
| Media surface | 上端 #1b1b24，下端 = Dark bg | 封面占位渐变 |
| Win close | #e81123 | 关闭按钮 hover 红：Windows 的平台惯例色，**不属于本设计语言**，不跟随 accent、不跟随主题 |
| Theme swatch | 深点 = Light text 的 hex，浅点 = Light bg 的 hex | 主题切换的两枚预览点。浅点必须等于它要预览的那面底（本批从差 1 单位的抄写值归位），深点比 app 底亮一档才在深色面板上看得见 |

Adaptive ink has a single step — pure black or pure white, no gray tiers (2026-09-22: "不要灰色"). Hierarchy on the wallpaper is carried by the type scale and weight, never by lightness. It applies only to containers with **no surface of their own**; cards, modals and the three translucent bars keep the Dark text* / Light text* roles above, otherwise black ink lands on a dark card.

### 对比度（实测，WCAG 2.x 相对亮度比）

目标仍是 **正文 4.5:1、大字号与 UI 3:1**。以下是量出来的，不是声明的：

| 配对 | 实测 | 判定 |
|---|---|---|
| Dark text / text-sub on Dark bg | 17.06:1 / 9.85:1 | AA |
| Light text / text-sub on Light bg | 16.01:1 / 10.17:1 | AA |
| Light text-muted on Light bg | 4.51:1 | AA（刚过线） |
| Dark text-muted on Dark bg | 3.85:1 | ✗ 未达 4.5 —— 已知例外，账本 §8 E20 |
| Success / Warning / Danger on Dark bg | 9.00:1 / 10.77:1 / 6.01:1 | AA |
| 三组 Light 语义色 on Light bg | 4.52:1 / 4.50:1 / 4.52:1 | AA（本批从 3.10 / 2.53 / 3.88 抬上来） |
| Accent on Dark bg | 4.62:1 | AA |
| 白字压默认 Accent | 4.19:1 | ✗ 12–13px 按钮标签未达 4.5；且 accent 运行期可被取色器改，这条只能按默认值算 —— 账本 §8 E20 |
| 自适应墨色 | 黑墨在合成亮度 >128 的一侧 ≥ 约 5.3:1；白墨在 ≤128 的一侧最坏出现在阈值附近（约 4.0:1），底色越深越高 | 单档黑白的代价：暗侧压不住中暗灰底时只剩 ~4:1。实际有 `--bg-overlay` 兜底，很少走到 |

## Type (locked)

| role | family | use | notes |
|------|--------|-----|-------|
| body / UI | Inter, 'Noto Sans SC', 'Yu Gothic UI', Meiryo | all interface text | metric-matched fallback |
| data | Inter | times, durations, counts | `font-variant-numeric: tabular-nums` |

Scale:
- page title: 18px / 600
- section title: 11px uppercase / 500 / letter-spacing 0.06em
- card title: 13px / 500
- card sub: 11px
- list row: 13px
- control label: 12px
- caption: 10.5px

## Scales (locked)

Spacing: 0, 2, 4, 6, 8, 10, 12, 16, 20, 24, 32, 40, 48, 64 px

Layout constants:
- titlebar-h: 36px
- sidebar-w: 232px (range 180–360)
- sidebar-w-collapsed: 56px
- toolbar-h: 44px
- inspector-w: 320px (range 260–480)
- playerbar-h: 76px
- grid-min: 176px
- card-row-h: 232px
- row-h: 46px
- gap: 12px

Radius:
- sm: 6px
- md: 10px
- lg: 16px
- full: 999px

Shadow:
- card: 0 6px 24px rgba(0,0,0,0.4)（浅色主题同几何、α 降到 0.1）
- pop: 0 12px 40px rgba(0,0,0,0.55)（浅色主题 0.18）
- 两侧都是纯黑投影：浅色主题没有「彩色阴影」这一档。

Motion:
- fast: 120ms
- base: 200ms
- slow: 360ms
- ease: cubic-bezier(0.4, 0, 0.2, 1)
- Only opacity and transform. No width/height/box-shadow animation.
- Honor `prefers-reduced-motion`.

## AI 工作台（长任务页的设计语言）

> **本节是 2026-09-23 重建的**：原「设计包第 1~4 期」文本随 `.ulpi/` 目录被整体删除而丢失（HEAD 里从来没有过它 —— 那一份是未入库的本机改动）。下面每一条都按**已经跑在屏幕上的代码**反写，不写理想形状；引用点全部标 `文件:行`。要改这几个组件，先改本节再改代码。
> 术语提醒：本节里出现的 `B2/B3/B5/B6/B7` 是**设计包审计编号**（`queueStats.ts:2-9`、`:78`、`:95` 的注释用的是同一套），与账本 §2 的阻断级红线 `B1~B5` 无关，别看串了。

### 1. 三条主张

- **不重建页面**：切换工种、开始/结束任务都不换组件树 —— 正在跑的队列必须一直在屏幕上。
- **状态只有一个事实源**：进度与统计从 `queueStats.ts` 的纯函数走（§6），视图只展示不算账。
- **读数不是控件**：轨道、进度带、汇总播报是给人读的量，不进 Tab 序、不做装饰。

### 2. 版面：一页三工种 + 三区骨架

同一张工作台承载**字幕翻译 / 文件夹名翻译 / 规则改名**三种工种（`AiTranslateView.vue:54-58`），切工种是 `wb-head` 里的**分段控件 + `v-show`**（`:288-295`、`:329`、`:371`、`:376`），不是路由、不重建组件树 —— 三档共用同一条队列状态与同一份凭据。快捷键 `Ctrl+1/2/3` 与点击等价（`:264-275`）。`ConvertView.vue` 借同一套骨架，所以两页必须看起来是一家人。

三区：`wb-head`（身份栏：页标题 + 工种分段 + 汇总播报）→ `wb-side`（选择范围与参数，窄列）→ `wb-stage`（右区，一次只放一个工种的主体）。三区的分工是死的：**状态灯与进度只出现在 head 与 stage，参数只出现在 side**。任何"两块都画一遍"的字段就是第二事实源。

### 3. 工序可见

#### 3.1 工序轨道（`ActionRail.vue`）

五段横轨，**一次只有一段进行中**；纯展示组件，阶段状态由父组件推导（`ActionRail.vue:6`）。状态色**只取既有 role**，不许新色：

| 段状态 | 色 | 附加形状 |
|---|---|---|
| 进行中 | `rgb(var(--accent-rgb))` + `--bg-selected` 底 | `aria-current="step"` |
| 完成 | `--text-sub` 字 / `--success` 点与勾 | — |
| 失败 | `--danger` | 可点（父组件给 `onPhaseClick` 时才给指针） |
| 跳过 | `--text-muted` | 圆点改**虚线描边**（不只靠颜色，同 §7） |
| 待办 | `--text-muted` | 点里是序号数字 |

整条 `role=list`、不进 Tab 序 —— 它是读数，不是控件。

### 4. 进度带（`ProgressBand.vue`）

字段顺序锁死：**已完成/总数 · 条目每分 · 当前项(截断) · 失败数**（`ProgressBand.vue:3`）。
「条目每分」= 已完成 ÷ 已跑秒 × 60，**前端现算**，不为它加契约（`:23-26`）。
**只在进行中渲染**，静止时整个组件不存在 —— 不留骨架位、不做装饰（`:6`）。
`role="status"`；分子是「有结果的行数」（§6 的 `finished`），分母是队列**总**行数 —— 现状如此：`exempt` 计入分子，所以扫描后一开局读数就偏高，这一格还没有按「本次范围」收口（记在账本，不在文档里写成理想）。

### 5. 队列与失败明细

`JobTable.vue` 是**唯一**画队列行的地方，`FailureDetail.vue` 是**唯一**展开失败原因的地方（复制原因 / 只重试这一项）。展开态归 `JobTable` 私有，父组件只能通过进度带的「失败 N」把用户送过去。

#### 5.1 空位不许用假数据填

缓存命中率、平均耗时这类字段**在 `AiProgress` 里没有就不显示**（`ProgressBand.vue:5`）。宁可少一列，也不许用推算值占位 —— 占位值会被当成实测值引用进文档。

#### 5.2 行状态是六个，不是四个

`pending / running / done / failed / skipped / exempt`（`queueStats.ts:13`）。`exempt` = **已翻译可豁免**（AI-15）：扫描时命中「源没动过 + 译文还在」即标它（`AiTranslateView.vue:148`），默认**不进**「开始翻译」的队列（`:163`），行内给一颗**重译**按钮把它放回 `pending`（`JobTable.vue:93-97`、`AiTranslateView.vue:239`）。
配色规则：`exempt` 用 `--success` 但压到 **opacity 0.75**（`JobTable.vue:147-148`）—— 与 `done` 同色不同强度，读作「这轮不用重翻」，**不引入第七种颜色**。

### 6. 进度与统计只有一个口径

统计认「**本次运行的范围快照**」，不认全队列 —— 这条**只对收尾汇总成立**：`summarizeRun(runPaths, files)` 先按快照过滤再算（`queueStats.ts:68-75`）。取消的行是「已跳过」不是「已完成」；收尾成功率只数本次快照里的行（`:4-9`）。
`finished = done + failed + skipped`（`exempt` 并入 `skipped` 计入，`queueStats.ts:31,44-47`）—— 进度带的**分子**走它，**分母仍是总行数**（`AiTranslateView.vue:331-336` 传的是 `:total="stat.total"`，实时那条 `stat` 过的是全表 `:43`，不是快照）。两处的口径不是一条，别在文档里把它们并成一句。
**重试的铁律**：`resetFailed` / `resetOne` / `reEnableOne` 都**不改行序、不增删行**（`queueStats.ts:79-81`）—— 导出批次的序号绑整个队列，动序等于把译文灌到别的文件上。

### 7. 可达性与播报

- 汇总播报走 `aria-live="polite"`，**只报里程碑**，不逐行刷屏（`AiTranslateView.vue:121`、`:341`）。
- 状态**不得只靠颜色区分**：跳过用虚线、完成用勾、失败靠文案与可点区（§3.1 表）。
- 分段控件带 `role/aria-selected`（`:293-295`）；轨道 `aria-current="step"`。
- 亮/暗主题与 `data-bg-bright` 墨色链路照常适用：工作台面板画了 `--bg-card`，所以**不加 `.on-bg`**（账本 §7.14 的作用面判据）。

### 8. 页面级签名

长任务页的签名动效 = **进度带左缘 2px accent 光泽**（`ProgressBand.vue:69-85`）：只动 `opacity`、`prefers-reduced-motion` 下降为静态 0.85、颜色取 `rgb(var(--accent-rgb) / 0.85)` 所以跟随用户取色器。一屏只允许一处签名 —— 同一页再加第二个发光元素就要在 §3.1 与 §4 之间做取舍，不是做加法。

### 9. 改名那一格：三段动作，只有一段碰盘

同一张卡片从上到下是三颗按钮，**危险等级要肉眼可分**（`AiRenamePanel.vue:6`）：「算一遍」只读盘，「生成译名」只写库（一个字节不改），「应用改名」是**唯一**会动文件的一步，而且整批可回滚。这条分工不靠 tooltip 传达 —— 卡片标题就把「只有「应用改名」会碰盘，整批可回滚」写在用户眼前（`:123`）。

- **两颗按钮互相挡**：生成在跑时不许应用（那时查到的是一本写了一半的账），反之亦然（`:34`、`:132`）。
- **按住要给为什么**：没算过 / 没有已定稿条目 / 没配模型，各配一句 `:title`，不许只变灰（`:133`、`AiRenameNames.vue:31`）。
- **「只是账」要说出来**：拿到名字之后那行文案明写「这些名字现在只是**账**：按上面「应用改名」才会动文件」（`AiRenameNames.vue:87`）—— 屏幕上任何一步都不能让用户以为名字已经生效。
- **拒掉的入口在清单行内**：「不要这个译名」（`:11`）；撞名与全路径超长的条目在面板上直接报「应用时一律跳过，改完也不会留账」（`AiRenamePanel.vue:178`）。

#### 9.1 档位词表只有一处，且是穷举映射

判定全在 Rust，前端只负责说人话：`renameLabels.ts:4-5` 三份 `Record<..., string>` 缺一个键 `vue-tsc` 就报错，所以「要翻 / 需翻译 / 待译」这类第二套说法长不出来。配色沿用既有 role，**零字面色值**：已合规 `--success`、要修与要翻 `--warning`、跳过 `--text-muted`（`AiRenameCounts.vue:39-42`、`AiRenameItems.vue:94-97`），与 §3.1 同一张色表。

#### 9.2 两个数不是一回事

账目条上的 `counts` 来自后端**全量**统计（清单截断也仍是全量），「可应用」却是从**列出的**条目算出来的（`AiRenameCounts.vue:3-8`）—— 所以截断时它如实标成「N+」（`AiRenamePanel.vue:49`）。拿截断后的列表去数全量的账，等于同一屏养两个口径。

## Voice
- Plain, direct, tool-like. No buzzwords.
- Action vocabulary consistent: "导入", "扫描", "播放", "转换", "收藏".
- Empty states explain why and give one next action.

## Every screen must read as the same product if placed side by side.
