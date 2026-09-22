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

## Voice
- Plain, direct, tool-like. No buzzwords.
- Action vocabulary consistent: "导入", "扫描", "播放", "转换", "收藏".
- Empty states explain why and give one next action.

## Every screen must read as the same product if placed side by side.
