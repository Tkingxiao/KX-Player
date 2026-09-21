# KX-Player 布局与交互（实现版）

> 本文是 `docs-vibecoding/03-ui-layout.md` 的**实现版**：写的是**代码里的真实取值**，不是规范打算取的值。
> 每一条常量都标了出处（`文件:行`）；与规范不一致的一律标 **⚠️ 差距**。
> **尺寸的唯一事实源是 `src/styles/tokens.css`**（颜色另有 `src/styles/theme.css`）。
> 凡本文数字与 `tokens.css` 冲突，**以 `tokens.css` 为准**，本文与代码同时错的可能性请按该文件复核。

---

## 1. 三栏骨架

### 1.1 骨架结构（`src/App.vue` 模板 + `src/styles/base.css`）

```
#app-shell   display:grid; grid-template-rows: var(--titlebar-h) 1fr var(--playerbar-h)
├─ <TitleBar />                                        36px
├─ #app-middle  display:grid; grid-template-columns: auto 1fr auto
│  ├─ <Sidebar />            width: collapsed ? 56px : settings.sidebarWidth   （auto 轨）
│  ├─ #content-wrap          position:relative; display:flex; flex-direction:column（1fr 轨）
│  │  ├─ <ContentArea />        内容区（卡片网格 / 紧凑列表 / 各视图）
│  │  └─ <StageView />          ★ 常驻挂载，覆盖内容区（stage-overlay）
│  └─ <InspectorPanel />     v-if="ui.inspectorOpen" + slide-right 过渡（auto 轨）
└─ <PlayerBar />                                       76px
```

出处：`styles/base.css:81-103`（`#app-shell` / `#app-middle` / `#content-wrap`）、
`App.vue:214-217`（`#app-shell` 的重复声明）、`App.vue:188-210`（模板）。

**三处容易踩的实现细节**：

1. **StageView 是常驻挂载的**（`App.vue:195`），不是 `v-if`。原因在 `stage/StageView.vue:82-95` 的注释里：
   离开舞台时要主动把 mpv 覆盖窗口藏起来，且 pip 的几何托管也在这里，卸载会丢状态。
   非舞台时靠 `stage-hidden` 类隐藏。
2. **播放栏不在 `#app-middle` 里**，它是 `#app-shell` 的第三行 —— 所以播放栏横跨全宽，
   侧栏与检查器只占中间那一行。
3. **背景层 `#bg-layer` 在骨架之外**（`App.vue:184-186`，`position:fixed; inset:0; z-index:0`，
   骨架 `z-index:1`）。平铺模式走动 `#bg-layer` 的 `background-image`，
   其余模式用 `<img object-fit>`（`App.vue:55-90`）。

### 1.2 三栏宽度是**行内 style**，不是 CSS 变量

| 区域 | 宽度来源 | 出处 |
|---|---|---|
| 侧栏 | `(collapsed ? 56 : settings.sidebarWidth) + 'px'` | Sidebar.vue:287 |
| 内容区 | `1fr`（自动吃掉剩余） | base.css:92 |
| 检查器 | `settings.inspectorWidth + 'px'` | InspectorPanel.vue:137 |

⚠️ 这意味着 `tokens.css` 里的 `--sidebar-w` / `--inspector-w` **运行期不生效**
（见 §2.2）。要改默认值，改的是 `src/stores/settings.ts` 的 `ref(...)` 初值。

---

## 2. 尺寸常量总表（`src/styles/tokens.css`）

### 2.1 全部 token（`tokens.css:2-32`，逐行照抄）

| token | 值 | 注释里的用法 |
|---|---|---|
| `--titlebar-h` | **36px** | 标题栏高 |
| `--sidebar-w` | **232px** | 侧栏默认宽 |
| `--sidebar-w-collapsed` | **56px** | 侧栏折叠（图标栏）宽 |
| `--toolbar-h` | **44px** | 「视频舞台顶栏；StageView 的 CSS 唯一来源」 |
| `--inspector-w` | **320px** | 检查器默认宽 |
| `--playerbar-h` | **76px** | 播放栏高 |
| `--grid-min` | **176px** | 卡片封面边长默认值 |
| `--card-row-h` | **232px** | `= grid-min + 56` |
| `--row-h` | **46px** | 列表行高 |
| `--gap` | **12px** | 卡片间距 |
| `--radius-sm` / `--radius` / `--radius-lg` | **6 / 10 / 16px** | 圆角 |
| `--dur-fast` / `--dur` / `--dur-slow` | **120 / 200 / 360ms** | 动效时长 |
| `--ease` | `cubic-bezier(0.4, 0, 0.2, 1)` | 标准缓动 |
| `--ease-out-back` | `cubic-bezier(0.34, 1.32, 0.64, 1)` | 「弹入/滑入用的轻微回弹」 |
| `--dur-press` | **90ms** | 通用过渡（按钮/卡片/列表行） |
| `--tb-alpha` / `--sb-alpha` / `--pb-alpha` | `1` | 标题栏/侧栏/播放栏透明度滑杆映射 |
| `--accent-rgb` | `230 58 46` | 强调色（= `#e63a2e`，与 `settings.ts:17` 的 `clr` 默认值一致） |

颜色 token 在 `src/styles/theme.css`（`--bg-card` / `--text-muted` / `--shadow-card` /
`--on-media` / `--bg-selected` 等，深色与浅色两套），本文不重复。

### 2.2 ⚠️ 有一批 token 是**声明了但没被任何 CSS 消费**的

对 `src/` 全量 grep 各 token 的实际引用点，结果：

| token | 实际被引用处 | 状态 |
|---|---|---|
| `--titlebar-h` | App.vue:216、base.css:87、TitleBar.vue:107/191、QueuePanel.vue:59 | ✅ 活 |
| `--playerbar-h` | App.vue:216、base.css:87、PlayerBar.vue:482、QueuePanel.vue:61、ToastHost.vue:23 | ✅ 活 |
| `--toolbar-h` | **仅** StageView.vue:265/323 | ✅ 活（语义已窄化成「舞台顶栏」） |
| `--sidebar-w-collapsed` | Sidebar.vue:666 | ✅ 活 |
| `--sidebar-w` | **无** | ❌ 死（运行期值来自 settings.sidebarWidth） |
| `--inspector-w` | **无** | ❌ 死（运行期值来自 settings.inspectorWidth） |
| `--grid-min` | **无** | ❌ 死（运行期值来自 settings.gridSize） |
| `--card-row-h` | **无** | ❌ 死（行高由 `cardRowHeight()` 算，见 §3） |
| `--row-h` | **无** | ❌ 死（列表行高由 `ROW_H` 常量给，见 §3.3） |
| `--gap` | **无**（`VirtualGrid.vue:10` 的 `gap` 是 JS prop 默认值 12） | ❌ 死 |

⚠️ **差距**：规范 `03 §2.1` 把 `tokens.css` 当尺寸单一来源，实际上**一半尺寸常量是死的**，
真值在 `src/stores/settings.ts` 与 `src/utils/layout.ts`。
`utils/layout.ts:1-4` 的注释自己也承认了这个双份维护问题：
「CSS 侧同名值在 styles/tokens.css … 这里改了就不同步 —— 两处一起改」。
**改尺寸时请同时检查这两处。**

---

## 3. 卡片网格：232px 行高是怎么算出来的

### 3.1 声明式公式（与规范一致 ✅）

```ts
// src/utils/layout.ts:6-12
/** 封面之下信息区占用的像素高度（内边距 + 边框 + 标题行 + 副标题行） */
export const CARD_CHROME_H = 56

/** 网格行高 = 封面边长 + 信息区；默认 gridSize 176 → 232px（03 §2.1 硬指标） */
export function cardRowHeight(gridSize: number): number {
  return gridSize + CARD_CHROME_H
}
```

**默认档的账**：`cardRowHeight(176) = 176 + 56 = 232px` ——
与 `tokens.css:10` 的 `--card-row-h: 232px /* = grid-min + 56 */` 完全对得上。✅

**注意网格行距**：虚拟网格的一「行」实际占 `cardHeight + gap` ＝ **232 + 12 = 244px**
（`VirtualGrid.vue:22` 的 `rowH`），12px 来自 `gap` prop 默认值。
所以「232px 行高」指的是**卡片高**，「244px」是**滚动步长**，两个数别混。

### 3.2 实际 CSS 累加：比 232 多 1.5px ⚠️

把卡片内部的 CSS 声明值逐项加起来（`SmartView.vue:237-284` + `base.css:5` 的
`* { box-sizing: border-box }` + `base.css:16` 的 `line-height: 1.5`）：

| 项 | 来源 | px |
|---|---|---|
| 卡片上下边框 | `.media-card { border: 1px solid }`（SmartView.vue:241） | 2 × 1 = **2** |
| 封面 | `:style="{ height: settings.gridSize + 'px' }"`（SmartView.vue:173） | **176** |
| 信息区内边距 | `.media-card-info { padding: 9px 11px 10px }`（SmartView.vue:269） | 9 + 10 = **19** |
| 标题行 | `.media-card-name { font-size: 12.5px }` × `line-height 1.5`（SmartView.vue:271） | **18.75** |
| 副标题行 | `.media-card-sub { font-size: 10.5px; margin-top: 2px }` × 1.5（SmartView.vue:277-280） | 15.75 + 2 = **17.75** |
| **合计** | | **233.5** |

⚠️ **差距**：`CARD_CHROME_H = 56` 的注释说它覆盖「内边距 + 边框 + 标题行 + 副标题行」，
但按 CSS 声明值累加，这四项其实是 **57.5px（19 + 2 + 18.75 + 17.75）**，比 56 多 **1.5px**。
因为 `VirtualGrid.vue:113` 把行高**硬设为** `cardHeight`（232px），多出的 1.5px 只会溢出到
下方 12px 的 `marginBottom` 里 —— 视觉上几乎看不出来，但**这个常量与真实 chrome 差 1.5px**。
若要严格对齐，应取 `CARD_CHROME_H = 58`（或把卡片内边距收 1px）。
**建议实测一次**：这类偏差只在真实字体渲染下才能定论，本文给的是按声明值的静态推算。

### 3.3 三个必须同步的数字

| 数字 | 值 | 位置 |
|---|---|---|
| `CARD_CHROME_H` | 56 | `src/utils/layout.ts:7` |
| `ROW_H`（列表行高） | **46** | `src/utils/layout.ts:15`（`tokens.css:11` 的 `--row-h` 同值但已死） |
| `cardRowHeight()` | `gridSize + 56` | `src/utils/layout.ts:10-12` |

消费点：`SmartView.vue:168-169`、`FolderView.vue:261-262`（卡片）；
`TrackTable.vue:130` `:row-height="ROW_H"`（列表）。

---

## 4. 网格三档尺寸与列数计算

### 4.1 三档（`settings.gridSize`）

```ts
// src/stores/settings.ts:51
const gridSize = ref<132 | 176 | 220>(176)
```

| 档 | 封面边长 | 行高 `cardRowHeight()` | 滚动步长 `+gap` |
|---|---|---|---|
| 小 | **132px** | 188px | 200px |
| **中（默认）** | **176px** | **232px** | **244px** |
| 大 | **220px** | 276px | 288px |

读盘时的归一化（`settings.ts:150-151`）：只接受 `132` / `220`，其余一律回落到 `176`：

```ts
const rawGrid = Number(s.gridSize ?? 176)
gridSize.value = (rawGrid === 132 || rawGrid === 220 ? rawGrid : 176) as 132 | 176 | 220
```

⚠️ **差距（无 UI 入口）**：规范 `03 §2.3` 要求「卡片尺寸 3 档（132 / 176 / 220）**由 Toolbar 切换**，持久化」。
现状：`gridSize` 只有**读盘 + 两处消费**，**全仓没有任何写入点**（grep `gridSize` 只命中
`settings.ts`、`SmartView.vue`、`FolderView.vue`、`tokens.css`、`layout.ts`），
`SettingsModal.vue` 里也**没有这一项**（其外观分区只有背景图适配方式）。
所以 **3 档尺寸现在改不了**，只能手改 `settings.json` 再重启。

### 4.2 列数 = 按容器宽度算，没有硬编码

```ts
// src/components/VirtualGrid.vue:51
const c = Math.max(1, Math.floor((el.clientWidth + props.gap) / (props.cardWidth + props.gap)))
```

- `cardWidth` 由调用方传 `settings.gridSize`（`SmartView.vue:168`）。
- 重算时机：`onMounted` + `ResizeObserver` + `watch([cardWidth, gap])`，
  都经 `requestAnimationFrame` 节流（`VirtualGrid.vue:47-83`）。
- 单元格轨道：`gridTemplateColumns: repeat(cols, minmax(cardWidth px, 1fr))`
  （`VirtualGrid.vue:116`）。

⚠️ **这里是 `minmax(cardWidth, 1fr)`，不是固定 `cardWidth`**：
列数取整后剩余的宽度会**被分摊到每列**，所以卡片的实际宽度 ≥ 档位值，
而封面高度是固定的 `gridSize`（`SmartView.vue:173`）→
**封面在多数窗口宽度下不是 1:1 正方形，而是略宽的长方形**。
规范 `03 §2.3` 说的是「封面 1:1 正方形」，实现是「1fr 拉伸」。
若必须正方形，应改成 `repeat(cols, cardWidth px)` 并把余量匀成间距。

---

## 5. 侧栏与检查器的拖拽调宽

### 5.1 侧栏 180–360（`Sidebar.vue:44-70`）

```ts
const startW = settings.sidebarWidth || 232
const raw = startW + ev.clientX - startX
lastW = Math.round(Math.max(180, Math.min(360, raw)))
settings.sidebarWidth = lastW
// pointerup: 再夹一次 + settings.scheduleSave()
```

| 项 | 值 |
|---|---|
| 默认宽 | **232px**（`settings.ts:61` `ref(232)`） |
| 最小 | **180px** |
| 最大 | **360px** |
| 折叠宽 | **56px**（`--sidebar-w-collapsed`，Sidebar.vue:666） |
| 落盘时机 | 拖动**结束**（`pointerup`）才 `scheduleSave()`；拖动中只改内存 |
| 实现 | `PointerEvent` + `setPointerCapture`（拖出元素也能跟手）+ `body.sidebar-resizing` |
| 取值 | `Math.round()` **有取整** |

读盘归一化（`settings.ts:154-155`）：`Math.max(180, Math.min(360, sw))`，非法值回落 232。

### 5.2 检查器 260–480（`InspectorPanel.vue:22-47`）

```ts
const startW = settings.inspectorWidth
settings.inspectorWidth = Math.max(260, Math.min(480, startW - (ev.clientX - startX)))
```

| 项 | 值 |
|---|---|
| 默认宽 | **320px**（`settings.ts:62` `ref(320)`） |
| 最小 | **260px** |
| 最大 | **480px** |
| 落盘时机 | `mouseup`（`scheduleSave()`） |
| 实现 | `MouseEvent` + `window` 上的 `mousemove/mouseup`（不是 PointerEvent，也没有 pointer capture） |
| 取值 | **没有取整**（与侧栏不一致，会得到小数宽度） |

⚠️ **差距**：侧栏用 PointerEvent + capture + `Math.round`，检查器用 MouseEvent + window 监听 + 不取整。
两个分隔条行为不完全一致（检查器在拖动时若指针移出窗口，靠 window 监听兜住，但没有 capture；
数值上会产生小数 px）。建议统一到侧栏那套。

### 5.3 其它可拖拽宽度

| 对象 | 默认 | 范围 | 出处 |
|---|---|---|---|
| 音轨表「艺术家」列 | 170px | 80–400px | `TrackTable.vue:55,63` |
| 音轨表「专辑」列 | 170px | 80–400px | 同上 |

音轨表列模板（`TrackTable.vue:73-76`）：
`(showIndex ? '44px ' : '') + 'minmax(0, 1fr) ' + artist + 'px ' + album + 'px 56px 64px 36px'`
→ 固定列：序号 44px、时长 56px、码率 64px、操作 36px。

---

## 6. 虚拟滚动

### 6.1 网格（`src/components/VirtualGrid.vue`）

| 项 | 值 | 出处 |
|---|---|---|
| `buffer` | **2 行**（行内常量，**不是** prop） | VirtualGrid.vue:25 |
| 行高 `rowH` | `cardHeight + gap` = 232 + 12 = **244px** | :22 |
| 可见区间 | `floor(scrollTop/rowH) - 2` ～ `ceil((scrollTop+viewportH)/rowH) + 2` | :27-28 |
| 位移方式 | 外层 `vgrid-spacer` 撑总高 + 内层 `transform: translateY(offsetY)` | :106-107 |
| 滚动节流 | `requestAnimationFrame`（`@scroll.passive`） | :64-72,105 |
| 列数监听 | `ResizeObserver` | :78-79 |
| 对外 API | `defineExpose({ scrollToTop, cols })` | :101 |

### 6.2 列表（`src/components/VirtualList.vue`）

| 项 | 值 | 出处 |
|---|---|---|
| `buffer` | **6 行**（prop，默认 6） | VirtualList.vue:13 |
| 行高 | 必传 `rowHeight`；曲库列表传 `ROW_H = 46` | :10, TrackTable.vue:130 |
| 可见区间 | `floor(scrollTop/rowHeight) - 6` ～ `ceil((scrollTop+viewportH)/rowHeight) + 6` | :26-28 |
| 对外 emit | `visible(start,end)` / `scroll(top)` | :15-18 |

⚠️ **差距**：规范 `03 §2.7` 要求「虚拟滚动：列表行 46px（buffer 6）；网格行高 232px、
**列数随宽度计算 + `_vlCols` 缓存避免重建**；`ResizeObserver`；
**窗口 resize 期间暂停渲染 300ms 并在结束后重建**」。

- 46 / buffer 6 / ResizeObserver ✅ 对得上。
- **`_vlCols` 这个缓存变量不存在**（现在靠 `cols` ref + `rAF` 节流，避免重建的机制换了）。
- **「resize 期间暂停渲染 300ms」不存在**：只有 `rAF` 节流，没有暂停窗口逻辑。
- 网格 `buffer = 2`（规范只写了列表的 6）。

---

## 7. 影院舞台（`src/features/stage/StageView.vue`）

| 项 | 值 | 出处 |
|---|---|---|
| 顶栏高 | `var(--toolbar-h)` = **44px** | StageView.vue:265 |
| mpv 嵌入矩形顶部让位 | `top: var(--toolbar-h)`（否则原生覆盖窗盖住返回按钮） | :323 |
| 控制条自动隐藏 | 鼠标静止 **2500ms** 后淡出，`@mousemove` 即复位 | :120-126,198 |
| 淡出前置条件 | `shouldShow` 且 `isVideo` 且正在播放（暂停时保留控制入口） | :123-125 |
| 单击/双击区分 | 单击延迟 **220ms** 判定，双击取消单击 | :160-174 |
| 矩形同步 | `getBoundingClientRect() × devicePixelRatio`，异步下发 | :27-48 |
| 隐藏条件 | 非 stage / pip 激活 / 非视频 / 仅音频 / 尺寸为 0 → 下发 `visible:false` | :22-24,32-40 |

**舞台可见性判定**（`StageView.vue:22-24`，是理解整个视频链路的钥匙）：

```ts
const shouldShow = computed(() =>
  isStage.value && !ui.pipEnabled && !!player.current?.isVideo
  && player.videoMode === 'video' && !player.audioOnly)
```

配套的 `videoMode` 在 `stores/player.ts:63-65`：

```ts
const videoMode = computed(() => ((ui.view === 'stage' || ui.pipEnabled)
  && !!current.value?.isVideo && !audioOnly.value ? 'video' : 'audio'))
```

注释解释了为什么必须由**视图状态**派生：从队列点击、续播、自动切歌这些入口不带视频意图，
若据此把 `vid` 置 `no`，之后切进舞台就只有声音没有画面。
`videoMode` 的 `watch` 会同步调 `playerSetVideoEnabled`（`player.ts:66-68`）。

⚠️ **注意**：`StageView.vue:52-67` 的 watcher 里，三个分支（pip 刚关 / 刚回 stage / 其它）
**最终都调用 `syncStage()`**，前两个分支只是先 `setTimeout(300ms)` 或立即 `nextTick`。
这不是 bug，但三个分支的差别只有「延迟多久推一次」，读起来容易误以为有语义差异。

---

## 8. 悬浮窗 / 画中画（pip）

### 8.1 尺寸与几何

| 项 | 值 | 出处 |
|---|---|---|
| 首开尺寸 | **320 × 180**（+ `PIP_GUTTER × 2`） | stores/ui.ts:104-115 |
| 首开位置 | 屏幕居中，Y 再上移 24px | ui.ts:110-111 |
| 四档预设（宽） | **240 / 320 / 480 / 640**（小/中/大/特大） | PipRoot.vue:57-58 |
| 宽下限/上限 | **160 / 960** | PipRoot.vue:59-60 |
| 高度上限 | `max(140, screen.availHeight - 96)` | PipRoot.vue:79 |
| 宽高比 | 取 mpv `video-params`；拿不到时回落 **16:9** | PipRoot.vue:48-51 |
| `PIP_GUTTER` | **0**（客户区就是画面本身，无边框） | contracts/pip.ts:9 |
| 位置/尺寸持久化 | `localStorage['kx.pipRect']` | ui.ts:118 |
| 钉住状态持久化 | `localStorage['kx.pipPinned']` | PipRoot.vue:55 |

Rust 侧窗口属性（`commands/player.rs:211-224`）：`decorations(false)`、
`transparent(true)`、`always_on_top(true)`、`skip_taskbar(true)`、`resizable(true)`、
`min_inner_size(120, 80)`、加载 `index.html`（同一前端入口，按窗口 label 分流到 `PipRoot`）。

### 8.2 层序是**反的**（理解 pip 的关键）

pip 窗口里视频**垫在窗口之下**，从窗口的透明像素里透出来；标题/按钮/进度条是普通 DOM，
天然浮在画面上、鼠标事件照常命中。所以窗口内任何不透明底色都会盖住视频 ——
根节点与画面区必须保持透明（`PipRoot.vue:5-14` 的注释）。

**握手时序**（`commands/player.rs:198-202`，刻意设计的四步，避免 `pip:shown` 在
WebView 加载完成前被丢弃）：

```
1. pip_open      → 创建/定位/显示窗口（不 attach，不 emit pip:shown）
2. PipRoot.onMounted → api.pipReady()
3. pip_ready     → attach_overlay(hwnd) → emit("pip:shown")
4. PipRoot 收到 pip:shown → syncRect()
```

窗口**复用**（首次创建后只 show + 定位，`player.rs:208-248`）：非首次时直接
`attach_overlay` + `emit("pip:shown")`，跳过握手。

### 8.3 pip 与舞台互斥

两者**共用同一个 mpv 覆盖窗口**，所以同一时刻只有一个画面出口：

- 开 pip 时：`ui.openPip()` 若当前在 stage 且上一视图不是 stage，会 **`goBack()` 退回上次视图**
  （`ui.ts:138-142`）—— 这是规范 P0-4「悬浮期间库浏览不中断」的落地方式。
- 关 pip 时：`StageView.vue:57-59` 等 **300ms** 再推矩形，因为要给 Rust 的 `@Attach(None)` 留时间，
  否则矩形坐标还按旧的 pip 客户区算。
- 未钉住的浮窗在切回舞台时会被自动关闭（`StageView.vue:89-91`）。

---

## 9. 标题栏与播放栏

### 9.1 标题栏（`src/features/shell/TitleBar.vue`）

| 项 | 值 | 出处 |
|---|---|---|
| 高 | `var(--titlebar-h)` = **36px** | TitleBar.vue:107 |
| 三列 | `1fr minmax(320px, 520px) 1fr` | :105 |
| 搜索框高 | **26px** | :145 |
| 搜索框内层 `max-width` | **480px** | :144 |
| 窗口按钮 | **32 × 36px** | :190-191 |
| 分隔线 | 1 × 14px | :181-182 |

⚠️ **差距**：规范 `03 §2.1` 的图里搜索区是「360–520px 自适应」，
代码是 `minmax(320px, 520px)`（下限 320 不是 360），且内层还有个 `max-width: 480px`
把实际输入框限制到 480px。三处数字（320 / 480 / 520）容易混，**以代码为准**。

### 9.2 播放栏（`src/features/shell/PlayerBar.vue`）

| 项 | 值 | 出处 |
|---|---|---|
| 高 | `var(--playerbar-h)` = **76px** | :482 |
| 三列（默认） | `minmax(180px,1fr) minmax(280px,2fr) minmax(180px,1fr)` | :479 |
| 左列封面 | **56 × 56px** | :525-526 |
| 播放按钮 | **38 × 38px** | :586-587 |
| 进度条 | **4px**，hover/拖动时 **6px** | :609,615 |
| 进度条拖拽点 | 10 × 10px | :636-637 |
| 书签刻度 | 3 × 10px | :648-649 |
| 音量条 | **90 × 4px** | :687-688 |
| 睡眠菜单 | 236px 宽 | :721 |
| 设备菜单 | 260px 宽 | :722 |
| 菜单最大高 | `max-height: 320px` | :713 |

---

## 10. 键盘快捷键清单

### 10.1 全局（`src/composables/useKeyboardShortcuts.ts`）

开关：`settings.keyboardEnabled`（默认 `true`，`settings.ts:36` 可关）。
**前置跳过条件**：焦点在 `INPUT`/`TEXTAREA`/`SELECT`/`contentEditable` 时跳过所有播放类按键
（`:10-15,32`）；带 `@Ctrl`/`Meta`/`Alt` 的键直接返回（`:33`，`Ctrl+F` 例外，在之前处理）。

| 键 | 行为 | 出处 |
|---|---|---|
| `@Ctrl+F`@ | 聚焦标题栏搜索框并全选 | :25-31 |
| `Space` | 播放 / 暂停 | :37-40 |
| `←` / `→` | 后退 / 前进 **5s** | :41-48 |
| `Shift+←` / `Shift+→` | 后退 / 前进 **30s** | :43,47 |
| `↑` / `↓` | 音量 **±0.02**（即 2%，满量程 0–1） | :49-56 |
| `,` / `.` | 速度 **∓0.05**（`SPEED_STEP = 0.05`，范围 0.5–2.0） | :57-62, speedPresets.ts:5 |
| `F` | 全屏（**仅当不在 stage/pip** 且是视频且 `videoMode==='video'` 时才转发，避免与本地处理器双触发） | :63-71 |
| `C` | 字幕显隐开关 | :72-79 |
| `S` | 仅音频模式切换 | :80-87 |
| `[` / `]` | 字幕偏移 **∓0.1s**（提前 / 延后） | :88-101 |
| `Esc` | 关面板 + 关右键菜单 + 关设置；在 lyrics/stage 视图额外 `goBack()`；浏览器全屏时**直接返回**交给浏览器 | :102-110 |
| `M` | 静音切换 | :111-114 |
| `B` | 在当前进度加书签 | :115-119 |

### 10.2 局部（会覆盖/补充全局的）

| 场景 | 键 | 行为 | 出处 |
|---|---|---|---|
| 舞台 StageView | `F` | 全屏 | StageView.vue:148-150 |
| | `Esc` | **仅在全屏时**退出全屏（不退播放） | :151-152 |
| | `Space` | 播放 / 暂停 | :153-156 |
| 悬浮窗 PipRoot | `Esc` | 还原回主窗口 | PipRoot.vue:347-349 |
| | `Space` | 播放 / 暂停 | :350-352 |
| | `←` / `→` | ∓ **5s** | :353-359 |
| 确认框 / 右键菜单 / 下拉菜单 / 背景编辑模态 | `Esc` 等 | 各自在**捕获阶段**监听（`@keydown`, `true`） | ConfirmDialog.vue:41、ContextMenuHost.vue:76、SelectMenu.vue:53、BgEditorModal.vue:141 |

⚠️ **差距**：规范 `03 §1.1` P0-7 列的是
`Space` · `←/→`（`Shift` ±30s）· `↑/↓` 音量 ±2 · `F` · `C` · `[` `]` ·
`S` · `@,` `.` **速度 ∓0.1** · `@Ctrl+F`@ · `Esc`。

- 规范写速度步长 **0.1**，代码是 **0.05**（`SPEED_STEP`）—— 不一致。
- 代码**多出**两个键：`M`（静音）与 `B`（加书签），规范清单里没有。
- 规范 P0-7 没有提及舞台/悬浮窗的**局部**键位覆盖（`Esc` 语义在两处完全不同：
  舞台是退全屏、悬浮窗是还原窗口）。

### 10.3 速度预设（`src/utils/speedPresets.ts`）

```ts
export const SPEED_MIN = 0.5
export const SPEED_MAX = 2.0
export const SPEED_STEP = 0.05
export const SPEED_PRESETS = [0.75, 1, 1.25, 1.5, 2]
```

Rust 侧对速度也做了夹取：`speed.clamp(0.5, 2.0)`（`commands/player.rs:31,71`）。

---

## 11. 响应式断点

### 11.1 主骨架（`src/App.vue:92-103`）

```ts
function applyResponsive(): void {
  const w = window.innerWidth
  // <1200 自动关闭右检查器，避免内容区被压没
  if (w < 1200 && ui.inspectorOpen) ui.inspectorOpen = false
  // <1024 侧边栏折叠为图标栏
  if (w < 1024 && !settings.sidebarCollapsed) {
    settings.sidebarCollapsed = true
    settings.scheduleSave()
  }
}
```

触发：`onMounted` 时调一次 + `ResizeObserver` 监听 `document.body`（`App.vue:172-174`）。

| 断点 | 行为 |
|---|---|
| `w < 1200` | 关闭检查器（仅当当前是打开状态） |
| `w < 1024` | 侧栏折叠为 56px 图标栏 |

与规范 `03 §2.1` 的对照：

| 规范档位 | 规范行为 | 实现 |
|---|---|---|
| ≥ 1440 | 三栏全开，卡片 5–7 列 | 无断点逻辑；列数由 §4.2 的公式自然算出 |
| 1200–1440 | 三栏全开，卡片 4–6 列 | 同上 |
| 1024–1200 | 检查器自动折叠 | ✅ `w < 1200` |
| 900–1024 | 侧栏折叠为图标栏 | ✅ `w < 1024` |
| < 900 | 不允许（最小窗口 900×600） | ✅ 靠 `tauri.conf.json` 的 `minWidth: 900 / minHeight: 600` 兜住 |

⚠️ **差距**：规范里 ≥1440 / 1200–1440 的「卡片 5–7 / 4–6 列」**没有对应实现**。
列数完全由「容器宽 + gridSize」算出（`VirtualGrid.vue:51`），在不同 DPI/侧栏状态下不会
落进规范给的区间，规范那两行应视为「期望效果」而非断点规则。

⚠️ **潜在问题（建议复核）**：`< 1024` 分支会**把折叠状态写进持久化设置**
（`settings.scheduleSave()`），而 `settings.sidebarCollapsed` 同时是用户在 UI 里手动折叠的
同一个字段。后果有两个：
1. **单向**：窗口缩到 1024 以下再放大，侧栏**不会自动恢复**（没有任何 `w >= 1024` 的反向分支）。
2. **覆盖偏好**：用户手动展开的偏好会被一次窗口缩放永久改写成「折叠」并落盘。
把「响应式折叠」与「用户偏好」拆成两个状态（如 `ui.sidebarAutoCollapsed` + `settings.sidebarCollapsed`）
会更稳。

### 11.2 播放栏自身的媒体查询（`PlayerBar.vue`）

| 断点 | 变化 | 出处 |
|---|---|---|
| 默认 | `minmax(180px,1fr) minmax(280px,2fr) minmax(180px,1fr)` | :479 |
| `max-width: 980px` | `minmax(140px,1fr) minmax(220px,2fr) minmax(120px,1fr)` | :491-493 |
| `max-width: 820px` | `minmax(44px,.5fr) 1fr minmax(120px,.6fr)`，播放键缩到 34×34 | :501-509 |
| `max-width: 600px` | `44px 1fr auto` | :513-514 |

⚠️ 注意：播放栏的媒体查询用的是**视口宽度**（`@media`），而播放栏是横跨全宽的第三行，
所以它和 `ApplyResponsive` 的 1200/1024 是两套独立阈值，互不联动。

### 11.3 窗口级限制

`tauri.conf.json:17-20`：默认 1200 × 800，`minWidth 900` / `minHeight 600`。
这也解释了规范为什么把 `< 900` 标为「不允许」—— 系统层面就到不了那个宽度。

---

## 12. 动效常量

来自 `tokens.css:18-25` 与 `src/styles/motion.css`：

| 名称 | 值 | 用途 |
|---|---|---|
| `--dur-fast` | 120ms | 悬浮、淡入 |
| `--dur` | 200ms | 常规过渡 |
| `--dur-slow` | 360ms | 侧栏宽度过渡等 |
| `--dur-press` | 90ms | 按钮按下 |
| `--ease` | `cubic-bezier(0.4, 0, 0.2, 1)` | 标准 |
| `--ease-out-back` | `cubic-bezier(0.34, 1.32, 0.64, 1)` | 弹入/滑入 |

**减弱动效**：`motion.css:2-9` 在 `prefers-reduced-motion: reduce` 下把
所有 `animation-duration` / `transition-duration` 压到 `0.01ms`（全局 `!` 覆盖）；
`base.css:369-374` 另外关掉了 `@.item-rise` 与 `img.fade-in` 两个具名动画。✅ 符合规范要求。

**动效只动 `opacity` / `transform`** 这条在卡片上体现为
`.media-card:hover { transform: translateY(-2px) }`（`SmartView.vue:245-248`，120ms）✅。

---

## 13. 组件清单

### 13.1 骨架与浮层（`src/components/`，9 个）

| 组件 | 职责 | 是否常驻 |
|---|---|---|
| `VirtualGrid.vue` | 虚拟化卡片网格（列数自适应 + raf 节流） | 按需 |
| `VirtualList.vue` | 虚拟化定高列表 | 按需 |
| `MediaCover.vue` | 封面（懒加载 + 进度条 + ▶/♪ 与时长角标 + 占位图标） | 按需 |
| `PipRoot.vue` | 悬浮窗根（仅在 `label="pip"` 窗口加载，707 行） | pip 窗口常驻 |
| `ToastHost.vue` | Toast 宿主（`bottom: calc(var(--playerbar-h) + 16px)`） | 常驻 |
| `ContextMenuHost.vue` | 右键菜单宿主（捕获阶段 Esc） | 常驻 |
| `SelectMenu.vue` | 自研下拉（`min-width: 132px`） | 按需 |
| `ConfirmDialog.vue` | 确认框（捕获阶段 keydown） | 常驻（ref 暴露 `open()`） |

### 13.2 功能视图（`src/features/`，19 个 .vue）

| 目录 | 组件 | 备注 |
|---|---|---|
| `shell/` | TitleBar / Sidebar / PlayerBar / QueuePanel | 前三者常驻 |
| `library/` | ContentArea / SmartView / FolderView / RecentView / TrackTable | 按视图切换 |
| `inspector/` | InspectorPanel | `v-if="ui.inspectorOpen"` |
| `stage/` | StageView | **常驻挂载**（覆盖内容区） |
| `lyrics/` | LyricsView | 前端自绘歌词浮层 |
| `search/` | SearchView | |
| `playlists/` | ListView | 收藏夹 / 播放列表共用 |
| `settings/` | SettingsModal / BgEditorModal | **异步组件**（`defineAsyncComponent`，App.vue:27-29） |
| `taxonomy/` | AutoTagReview | **异步组件** |
| `convert/` | ConvertView | |
| `ai/` | AiTranslateView | 693 行，AI 翻译 |

**异步加载策略**（`App.vue:2-4, 26-29`）：首屏只同步加载「立刻可见」的骨架
（TitleBar / Sidebar / ContentArea / InspectorPanel / PlayerBar / StageView），
弹窗与低频视图（SettingsModal / BgEditorModal / AutoTagReview）一律
`defineAsyncComponent` —— 注释写明这是为了「避免启动时为不可见代码付出解析/求值成本」。

### 13.3 store（`src/stores/`，6 个）

`library` / `player` / `playlists` / `settings` / `taxonomy` / `ui`

⚠️ **差距**：规范 `02 §2.7` 列了 10 个 store（library / filter / taxonomy / player / queue /
playlists / subtitle / ui / settings / tasks）。现状只有 6 个 ——
**`filter` / `queue` / `subtitle` / `tasks` 四个都不存在**，职责被折进现有 store：
筛选在 `ui`（`categoryId`/`tagIds`/`tagMode`/`smartKey`/`durationKey`）+
各视图的 `computed` 里，队列在 `player` 里，「任务」在 `library.scanning` 与 convert 视图里。

### 13.4 composables 与 utils

| 目录 | 文件 |
|---|---|
| `composables/` | `useKeyboardShortcuts` / `useThemeEffect` / `useTrackActions` |
| `utils/` | `layout` / `collections` / `covers` / `fuzzy` / `format` / `queue` / `playback` / `speedPresets` / `folderTree` / `color` / `lyricsLoader` |
| `utils/parsers/` | `lyrics.ts`（LRC/SRT/VTT 解析）/ `matcher.ts`（`findBestLyricsMatch`） |
| `services/` | `confirmHost.ts` / `aiTranslate.ts` |

---

## 14. 与规范 `03-ui-layout.md` 的差异速查

| # | 项 | 规范 | 实现 | 影响 |
|---|---|---|---|---|
| 1 | 三栏骨架（232 / 320 / 折叠 56） | 232px 侧栏 + 320px 检查器，可折叠 | ✅ 一致（但宽度走行内 style，`tokens` 变量是死的） | — |
| 2 | 顶栏 `Toolbar 44px`（含面包屑/筛选 chips/排序/视图切换） | 中栏顶部有一条**全局** 44px 工具栏 | **不存在全局工具栏**。`ContentArea.vue` 只有 `.filter-chips`（`padding: 8px 20px 0`，无固定高度）；每个视图各写自己的头部（如 `SmartView.vue:209-220` 的 `.sv-head`，`padding: 18px 20px 10px` + `.sv-title` 20px），排序/视图切换也在各视图头里。`--toolbar-h` 现在只给舞台顶栏用 | 视觉/信息架构差异 |
| 3 | 搜索区 360–520px | 居中自适应 | `minmax(320px, 520px)` + 内层 `max-width: 480px` | 数字不一致 |
| 4 | 卡片尺寸 3 档由 Toolbar 切换 | 132 / 176 / 220 可切 | 档位与默认值都在，但**无 UI 入口**，改不了 | **功能缺失** |
| 5 | 封面 1:1 正方形 | 正方形 | 轨道是 `minmax(cardWidth, 1fr)`，卡片会被拉伸，封面height固定 176 → **常常不是正方形** | 视觉偏差 |
| 6 | 卡片行高 232px | 硬指标 | 声明口径 176+56=232 ✅；但按 CSS 累加是 **233.5** | 1.5px 偏差 |
| 7 | 卡片信息区 | 标题 13px 最多 2 行 + 副标题 11px + 最多 3 个标签 chip | 标题 `12.5px` **1 行省略**、副标题 `10.5px`、**无标签 chip** | 内容缺失 |
| 8 | 进度环（左上） | 圆环 | **不存在**；只有封面底部 2px 进度条（`MediaCover.vue:110-116`） | 视觉差异 |
| 9 | 虚拟滚动 buffer（列表 6 / 网格 232px 行高） | 列表 buffer 6 | 列表 ✅ 6；网格 buffer **2**（规范未给网格值） | 一致 |
| 10 | `_vlCols` 缓存 / resize 暂停渲染 300ms | 要求 | **都没有**；现为 `cols` ref + rAF 节流 | 实现差异 |
| 11 | 侧栏 180–360 拖拽 | 要求 | ✅ `Math.round` + PointerEvent + capture | 一致 |
| 12 | 检查器 260–480 拖拽 | 要求 | ✅ 范围一致，但 MouseEvent + **不取整** | 与侧栏不一致 |
| 13 | 播放栏 76px | 要求 | ✅ | 一致 |
| 14 | 舞台 2.5s 淡出 / 双击全屏 / Esc 退全屏 | 要求 | ✅ 2500ms、220ms 单击防抖、Esc 仅退全屏 | 一致 |
| 15 | 悬浮窗 4 档（240×135…640×360） | 默认 320×180，4 档 | ✅ 按宽度档 240/320/480/640 + 比例算高 | 语义等价 |
| 16 | 键盘 `,` `.` 速度步长 0.1 | 0.1 | **0.05** | 不一致 |
| 17 | 键盘清单 | P0-7 十项 | 多出 `M`（静音）、`B`（书签） | 文档未记 |
| 18 | 响应式 4 档（≥1440 / 1200–1440 / 1024–1200 / 900–1024） | 含「卡片 5–7 / 4–6 列」 | 只有 1200 / 1024 两个阈值，列数无断点 | 部分未实现 |
| 19 | 侧栏折叠 | 用户可折叠 | 响应式折叠会**写进持久化偏好**且单向不可逆 | ⚠️ 见 §11.1 |
| 20 | 动效只动 opacity/transform + prefers-reduced-motion | 要求 | ✅ | 一致 |

---

## 15. 快速索引（改哪里）

| 我想改… | 去改 | 还要同步 |
|---|---|---|
| 标题栏高 / 播放栏高 | `src/styles/tokens.css`（`--titlebar-h` / `--playerbar-h`） | 无（唯一来源） |
| 侧栏/检查器**默认**宽 | `src/stores/settings.ts` 的 `ref(232)` / `ref(320)` | `tokens.css` 里那两个已死的变量（仅注释一致） |
| 侧栏/检查器**范围** | `Sidebar.vue:55,64` / `InspectorPanel.vue:31` + `settings.ts:155,157` | **四处都要改**（拖动时夹一次、读盘时夹一次） |
| 卡片三档尺寸 | `settings.ts:51`（类型与默认）+ `settings.ts:151`（读盘白名单）+ `tokens.css:9-10` | 并**新增一个 UI 入口**（现在没有） |
| 卡片信息区高度 | `src/utils/layout.ts` 的 `CARD_CHROME_H` | `tokens.css` 的 `--card-row-h`（已死，但注释里的公式要跟着改） |
| 列表行高 | `src/utils/layout.ts` 的 `ROW_H` | `tokens.css` 的 `--row-h`（已死） |
| 网格间距 | `VirtualGrid.vue:10` 的 `gap` 默认值（调用方未传） | `tokens.css` 的 `--gap`（已死） |
| 键盘键位 | `src/composables/useKeyboardShortcuts.ts`（全局）+ `StageView.vue:144` / `PipRoot.vue:346`（局部） | 更新本文 §10 与规范 P0-7 |
| 响应式行为 | `App.vue:94-103` | 注意不要污染 `settings.sidebarCollapsed` |
| 悬浮窗档位 | `PipRoot.vue:57-60` | `ui.ts:104-115` 的首开尺寸 |