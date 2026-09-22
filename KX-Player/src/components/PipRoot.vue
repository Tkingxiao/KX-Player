<script setup lang="ts">
/**
 * 独立悬浮窗（画中画）入口 —— 仅在 label="pip" 的独立系统窗口内加载。
 *
 * ## 层序（和本应用其它窗口相反）
 * 视频由 Rust 侧的宿主窗口「垫」在本窗口之下，画面从本窗口的透明像素里透出来。
 * 于是标题 / 按钮 / 进度条就是普通 DOM，天然浮在画面上，鼠标事件也照常命中，
 * 既不需要一圈「放按钮的边框」，也不需要轮询系统光标来猜 hover。
 * 代价：窗口内任何不透明底色都会盖住视频 —— 根节点与画面区必须保持透明。
 *
 * ## 无黑边
 * 窗口尺寸始终等于画面显示尺寸，比例取自 mpv 的 video-params，所以 mpv 的 keepaspect
 * 不需要留 letterbox。预设切换、拖系统边框、换新都按同一比例对齐。
 * 留白 GUTTER 取 0：客户区就是画面本身，没有投影/描边那一圈（无边框窗口）。
 *
 * ## 交互
 * 拖动一律交给系统窗口循环（顶栏按下即拖，画面区越过 4px 阈值后接管），所以哪儿拖都跟手、
 * 不抖；单击播放/暂停、双击还原；改尺寸走系统隐形边框（拖哪条边都跟手，松手后回到视频比例）；
 * Esc 还原、空格播放/暂停、←/→ 快进。控制层随鼠标进出淡入淡出，暂停时常驻。
 *
 * ## 握手
 * pip_open 创建/显示窗口 → onMounted → api.pipReady()
 * → Rust attach_overlay + emit pip:shown → syncRect
 */
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { getCurrentWindow, PhysicalSize } from '@tauri-apps/api/window'
import { api, onEvent } from '@/bridge/ipc'
import type { MpvPlayerState } from '@/contracts/api'
import { PIP_GUTTER } from '@/contracts/pip'

const win = getCurrentWindow()

// ── 播放状态 ──
const state = ref<MpvPlayerState>({
  playing: false, position: 0, duration: 0, speed: 1, volume: 85, muted: false,
  trackPath: null, videoActive: false, videoW: 0, videoH: 0,
})

const title = computed(() => {
  const p = state.value.trackPath
  if (!p) return ''
  const seg = p.replace(/\\/g, '/').split('/')
  return seg[seg.length - 1] || p
})
/** 有画面可透出：mpv 解出了视频轨，且 vid 处于开启态（「仅音频」时是关的） */
const hasVideo = computed(() => state.value.videoActive && state.value.videoW > 0)
/** 画面宽高比：拿不到 mpv 参数时退回 16:9（并会在参数到达后自动对齐） */
const aspect = computed(() => {
  const { videoW, videoH } = state.value
  return videoW > 0 && videoH > 0 ? videoW / videoH : 16 / 9
})

// ── 几何 ──
const cardEl = ref<HTMLElement | null>(null)
const pinned = ref(localStorage.getItem('kx.pipPinned') === '1')

const PRESET_W = [240, 320, 480, 640]
const PRESET_LABEL = ['小', '中', '大', '特大']
const MIN_W = 160
const MAX_W = 960
/** 供 <style v-bind> 使用：CSS 里的留白与 JS 里的几何换算共用同一个值 */
const gutterCss = `${PIP_GUTTER}px`

function dpr(): number {
  return window.devicePixelRatio || 1
}
/** 画面区当前宽度（CSS px）：窗口内宽减去左右留白 */
function cardW(): number {
  return Math.max(1, window.innerWidth - PIP_GUTTER * 2)
}
function cardH(): number {
  return Math.max(1, window.innerHeight - PIP_GUTTER * 2)
}
function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v))
}
/** 按宽高比折算画面尺寸，并把整体高度限制在可用屏幕内（竖屏视频会把宽度收回来） */
function fitW(w: number, a: number): { w: number; h: number } {
  const maxH = Math.max(140, (window.screen?.availHeight || 900) - 96)
  let ww = clamp(w, MIN_W, MAX_W)
  let hh = ww / a
  if (hh > maxH) {
    hh = maxH
    ww = hh * a
  }
  return { w: ww, h: hh }
}

/** 上一次由我们自己请求的客户区尺寸（物理像素）。onResized 用它分辨「自己改的」和
 *  「用户拖窗口边框改的」——后者要按视频比例拉回去。 */
let wantSize: { w: number; h: number } | null = null

async function setSizeToCard(cw: number, ch: number): Promise<void> {
  const d = dpr()
  // setSize 设的就是客户区尺寸，所以画面区正好等于预设值
  const w = Math.round((cw + PIP_GUTTER * 2) * d)
  const h = Math.round((ch + PIP_GUTTER * 2) * d)
  wantSize = { w, h }
  try {
    await win.setSize(new PhysicalSize(w, h))
  } catch (e) {
    console.error('[pip] 调整窗口尺寸失败:', e)
  }
}

/**
 * 把 mpv 宿主窗口对齐到画面区（物理像素，相对 pip 客户区）。
 * 宿主在 pip 之下，两者必须逐帧同步，否则拖动时画面会滞后于窗口。
 */
function syncRect(): boolean {
  const el = cardEl.value
  if (!el) return false
  const r = el.getBoundingClientRect()
  if (r.width <= 0 || r.height <= 0) return false
  const d = dpr()
  void api.playerSetStageRect({
    x: Math.round(r.left * d),
    y: Math.round(r.top * d),
    w: Math.round(r.width * d),
    h: Math.round(r.height * d),
    visible: hasVideo.value,
  })
  return true
}

/** 带重试的 syncRect：每 50ms 重试，最多 20 次（等布局与 mpv 参数就绪） */
function syncRectWithRetry(maxRetries = 20, intervalMs = 50): void {
  let tries = 0
  const attempt = () => {
    if (syncRect()) return
    tries++
    if (tries < maxRetries) setTimeout(attempt, intervalMs)
  }
  void nextTick(attempt)
}

let persistTimer: ReturnType<typeof setTimeout> | null = null
/** 持久化窗口几何（物理像素），供主窗口下次打开时恢复。拖动中合并写入，避免每帧读写 localStorage */
function persistRect(): void {
  if (persistTimer) clearTimeout(persistTimer)
  persistTimer = setTimeout(() => {
    persistTimer = null
    void Promise.all([win.outerPosition(), win.innerSize()]).then(([p, s]) => {
      try {
        localStorage.setItem('kx.pipRect', JSON.stringify({ x: p.x, y: p.y, w: s.width, h: s.height }))
      } catch { /* ignore */ }
    }).catch(() => { /* ignore */ })
  }, 350)
}

function nearestIndex(w: number): number {
  let best = 1
  let dist = Infinity
  PRESET_W.forEach((pw, i) => {
    const dd = Math.abs(w - pw)
    if (dd < dist) { dist = dd; best = i }
  })
  return best
}
const sizeIndex = ref(1)

// ── 尺寸切换：一步到位 ──
// 不做补间动画。窗口是系统窗口、画面是垫在它下面的另一个原生窗口，逐帧 setSize 时
// 画面层要经 DOM→IPC→Rust 队列三跳才跟上，肉眼就成了「框先走、视频慢半拍」。
function applyPreset(i: number): void {
  sizeIndex.value = i
  const f = fitW(PRESET_W[i], aspect.value)
  void setSizeToCard(f.w, f.h).then(() => nextTick(syncRect))
}

function cycleSize(): void {
  applyPreset((sizeIndex.value + 1) % PRESET_W.length)
}

/** 换曲后比例变了：保持宽度，把高度对齐到新比例 */
function refitToAspect(): void {
  const f = fitW(cardW(), aspect.value)
  void setSizeToCard(f.w, f.h)
  void nextTick(syncRect)
}

/**
 * 窗口尺寸变了。自己 setSize 引起的那种只需把画面层重新对齐；
 * 用户拖系统边框引起的那种要按视频比例拉回去 —— 无边框窗口没有别的调尺寸入口，
 * 而放任不管就会重新出现左右黑边。取「变化更大的那条轴」为准，拖哪条边都跟手。
 */
async function handleResized(): Promise<void> {
  let s: { width: number; height: number }
  try {
    s = await win.innerSize()
  } catch (e) {
    console.error('[pip] 读取窗口尺寸失败:', e)
    return
  }
  sizeIndex.value = nearestIndex(cardW())
  persistRect()
  void nextTick(syncRect)
  if (wantSize && Math.abs(s.width - wantSize.w) <= 2 && Math.abs(s.height - wantSize.h) <= 2) return
  const a = aspect.value
  // 已经等比（含取整误差）就不干预，否则会和系统的尺寸循环互相触发
  if (Math.abs(s.width / s.height - a) / a < 0.015) return
  const d = dpr()
  const prev = wantSize ?? { w: s.width, h: s.height }
  const vertical = Math.abs(s.height - prev.h) > Math.abs(s.width - prev.w)
  const f = fitW(vertical ? (s.height / d) * a : s.width / d, a)
  await setSizeToCard(f.w, f.h)
  void nextTick(syncRect)
}

// 画面参数是异步到达的（起播后才会有 video-params）：比例变了就重新对齐窗口，
// 没变（例如又是一部 16:9）也要重推一次矩形，因为 hasVideo 可能刚从 false 翻成 true。
let lastAspect = 0
watch([aspect, hasVideo], () => {
  if (Math.abs(aspect.value - lastAspect) > 0.002) {
    lastAspect = aspect.value
    refitToAspect()
  } else {
    void nextTick(syncRect)
  }
})

// ── 控制层显隐 ──
const inside = ref(false)
const revealed = ref(false)
let revealTimer: ReturnType<typeof setTimeout> | null = null
/** 打开瞬间先亮一下，让用户看到控件在哪；之后交给鼠标 */
function reveal(ms: number): void {
  revealed.value = true
  if (revealTimer) clearTimeout(revealTimer)
  revealTimer = setTimeout(() => { revealTimer = null; revealed.value = false }, ms)
}
/** 暂停时常驻，否则鼠标离开即淡出（窗口尺寸不变，所以不再有「跳一下」的感觉） */
const uiVisible = computed(() => inside.value || revealed.value || !state.value.playing)

// ── 画面区拖动 + 点击 ──
/** 手动阶段的位移阈值：越过它就把手交给系统，同时用它区分「点击」 */
const DRAG_SLOP = 4
let bodyDown: { x: number; y: number } | null = null
let clickTimer: ReturnType<typeof setTimeout> | null = null

function onBodyDown(e: PointerEvent): void {
  if (e.button !== 0) return
  if ((e.target as HTMLElement).closest('.pip-nodrag')) return
  bodyDown = { x: e.clientX, y: e.clientY }
  window.addEventListener('pointermove', onBodyMove)
  window.addEventListener('pointerup', onBodyUp, { once: true })
}

function endBodyDrag(): void {
  window.removeEventListener('pointermove', onBodyMove)
  window.removeEventListener('pointerup', onBodyUp)
  bodyDown = null
}

/**
 * 画面区不自己搬窗口：手动 setPosition 是「读光标位移 → 算绝对位置」的反馈环，
 * 命令有一帧延迟时，测到的位移会被「窗口已经走过的量」吃掉，指令位置就在光标前后来回
 * 摆动，肉眼就是抖动。越过阈值后直接交给系统拖动循环（和顶栏同一条路），OS 带着窗口走，
 * 跟手且不会抖。代价：进入系统循环后页面收不到 pointerup，所以这里先自行收尾。
 */
function onBodyMove(e: PointerEvent): void {
  const d = bodyDown
  if (!d) return
  if (Math.hypot(e.clientX - d.x, e.clientY - d.y) < DRAG_SLOP) return
  endBodyDrag()
  void win.startDragging().catch((err) => console.error('[pip] 启动拖动失败:', err))
}

/** 只有没越过阈值时才会走到这里（越过阈值时监听已摘掉） */
function onBodyUp(): void {
  endBodyDrag()
  scheduleClick()
}

/** 单击播放/暂停；240ms 内第二下改判为「还原到主窗口」 */
function scheduleClick(): void {
  if (clickTimer) {
    clearTimeout(clickTimer)
    clickTimer = null
    restoreToMain()
    return
  }
  clickTimer = setTimeout(() => { clickTimer = null; togglePlay() }, 240)
}

/** 顶栏/底栏空白处：按下即进入系统拖动循环，没有点击语义 */
function startDrag(e: MouseEvent): void {
  if (e.button !== 0) return
  e.preventDefault()
  void win.startDragging().catch((err) => console.error('[pip] 启动拖动失败:', err))
}

// ── 播放控制 ──
function togglePlay(): void {
  void api.playerToggle(state.value.playing)
}

function restoreToMain(): void {
  if (clickTimer) { clearTimeout(clickTimer); clickTimer = null }
  void api.pipRestore()
}

function close(): void {
  void api.pipClose()
}

function togglePinned(): void {
  pinned.value = !pinned.value
  localStorage.setItem('kx.pipPinned', pinned.value ? '1' : '0')
  void api.pipSetPinned(pinned.value)
}

/** 进度条：按下即 seek，按住拖动可擦洗 */
function onSeekDown(e: PointerEvent): void {
  const dur = state.value.duration
  if (dur <= 0) return
  e.preventDefault()
  e.stopPropagation()
  const el = e.currentTarget as HTMLElement
  el.setPointerCapture?.(e.pointerId)
  const seek = (clientX: number): void => {
    const r = el.getBoundingClientRect()
    void api.playerSeek(clamp((clientX - r.left) / r.width, 0, 1) * dur)
  }
  seek(e.clientX)
  const move = (ev: PointerEvent): void => seek(ev.clientX)
  const up = (): void => {
    el.removeEventListener('pointermove', move)
    el.removeEventListener('pointerup', up)
  }
  el.addEventListener('pointermove', move)
  el.addEventListener('pointerup', up)
}

const progressPct = computed(() => {
  const { position, duration } = state.value
  return duration > 0 ? clamp(position / duration, 0, 1) * 100 : 0
})

function fmt(sec: number): string {
  const s = Math.max(0, Math.floor(sec))
  const m = Math.floor(s / 60)
  return `${m}:${String(s % 60).padStart(2, '0')}`
}

function onKeyDown(e: KeyboardEvent): void {
  if (e.key === 'Escape') {
    e.preventDefault()
    restoreToMain()
  } else if (e.key === ' ') {
    e.preventDefault()
    togglePlay()
  } else if (e.key === 'ArrowRight') {
    e.preventDefault()
    void api.playerSeek(state.value.position + 5)
  } else if (e.key === 'ArrowLeft') {
    e.preventDefault()
    void api.playerSeek(state.value.position - 5)
  }
}

let unStates: Array<() => void> = []
let ro: ResizeObserver | null = null

onMounted(() => {
  window.addEventListener('keydown', onKeyDown)

  unStates.push(onEvent('player:state', (s: MpvPlayerState) => { state.value = s }))
  unStates.push(onEvent('player:ended', () => {}))

  // 关闭/还原：复位钉住态与临时显隐状态
  unStates.push(onEvent('pip:closed', () => {
    pinned.value = false
    try { localStorage.setItem('kx.pipPinned', '0') } catch { /* ignore */ }
    if (revealTimer) { clearTimeout(revealTimer); revealTimer = null }
  }))

  // pip:shown：Rust 已完成 attach（宿主窗口已垫到本窗口之下），尺寸由主窗口下发
  unStates.push(onEvent('pip:shown', () => {
    void api.playerGetState().then((s) => { state.value = s })
    refitToAspect()
    reveal(1800)
    syncRectWithRetry()
  }))

  void api.playerGetState().then((s) => { state.value = s })

  void win.onResized(() => { void handleResized() }).then((u) => unStates.push(u))
  void win.onMoved(() => {
    persistRect()
    void nextTick(syncRect)
  }).then((u) => unStates.push(u))

  // 布局变化（留白/字体/尺寸动画）都要让画面层重新对齐
  ro = new ResizeObserver(() => syncRect())
  if (cardEl.value) ro.observe(cardEl.value)

  sizeIndex.value = nearestIndex(cardW())
  void api.pipReady()
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKeyDown)
  window.removeEventListener('pointermove', onBodyMove)
  if (clickTimer) { clearTimeout(clickTimer); clickTimer = null }
  if (revealTimer) { clearTimeout(revealTimer); revealTimer = null }
  if (persistTimer) { clearTimeout(persistTimer); persistTimer = null }
  for (const u of unStates) u()
  unStates = []
  ro?.disconnect()
  ro = null
})
</script>

<template>
  <div
    class="pip-root"
    :class="{ 'ui-on': uiVisible, pinned }"
    @mouseenter="inside = true"
    @mouseleave="inside = false"
  >
    <!-- 画面区：mpv 宿主窗口垫在这里之下，元素本身必须保持透明 -->
    <div ref="cardEl" class="pip-card" @pointerdown="onBodyDown">
      <!-- 顶部：标题 + 关闭（浮在画面上，空白处可拖窗） -->
      <div class="pip-top pip-nodrag" @mousedown="startDrag">
        <span class="pip-title">{{ title || '悬浮窗' }}</span>
        <button class="pip-btn pip-close" title="关闭悬浮窗" aria-label="关闭悬浮窗" @mousedown.stop @click.stop="close">
          <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
        </button>
      </div>

      <!-- 底部：进度 + 操作（浮在画面上） -->
      <div class="pip-bottom pip-nodrag">
        <div class="pip-row" @mousedown="startDrag">
          <button class="pip-btn pip-play" :title="state.playing ? '暂停' : '播放'" aria-label="播放暂停" @mousedown.stop @click.stop="togglePlay">
            <svg v-if="!state.playing" viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><polygon points="6,3 21,12 6,21" /></svg>
            <svg v-else viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><rect x="5.5" y="4" width="4.5" height="16" rx="1.2" /><rect x="14" y="4" width="4.5" height="16" rx="1.2" /></svg>
          </button>
          <span v-if="state.duration > 0" class="pip-time">{{ fmt(state.position) }}<i>/</i>{{ fmt(state.duration) }}</span>
          <span class="pip-spacer" />
          <button
            class="pip-btn"
            :class="{ on: pinned }"
            :title="pinned ? '取消钉住（切换页面时自动关闭）' : '钉住（切换页面时保持显示）'"
            aria-label="钉住悬浮窗"
            @mousedown.stop
            @click.stop="togglePinned"
          >
            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M12 17v5" /><path d="M9 10.8V5h6v5.8l2 2.2H7z" /></svg>
          </button>
          <button class="pip-btn" :title="`尺寸：${PRESET_LABEL[sizeIndex]}（点击切换）`" aria-label="切换尺寸" @mousedown.stop @click.stop="cycleSize">
            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M4 4h7v7H4zM13 13h7v7h-7z" /></svg>
          </button>
          <button class="pip-btn" title="还原到主窗口（双击画面 / Esc 同样可还原）" aria-label="还原到主窗口" @mousedown.stop @click.stop="restoreToMain">
            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="M15 3h6v6" /><path d="M9 21H3v-6" /><path d="M21 3l-7 7" /><path d="M3 21l7-7" /></svg>
          </button>
        </div>
        <!-- 进度条：贴着画面底边，浮在画面上；悬停加粗并可点选/擦洗 -->
        <div class="pip-seek" @mousedown.stop @pointerdown="onSeekDown">
          <div class="pip-seek-fill" :style="{ width: progressPct + '%' }" />
        </div>
      </div>

      <!-- 无视频轨时画面区没有内容可透出，补一层底色，避免浮窗「消失」的错觉 -->
      <div v-if="!hasVideo" class="pip-placeholder">
        <span class="pip-placeholder-text">纯音频</span>
      </div>
    </div>
  </div>
</template>

<style>
/* 独立悬浮窗全局样式：整窗透明，视频从下层宿主窗口透出来 */
html,
body,
#app {
  background: transparent !important;
  margin: 0;
  overflow: hidden;
  user-select: none;
}
</style>

<style scoped>
.pip-root {
  position: fixed;
  inset: 0;
  /* 四周留白：GUTTER=0 时客户区即画面，画面之外不再有任何可视边界 */
  padding: v-bind(gutterCss);
  color: var(--on-media);
  font-family: 'Inter', 'Noto Sans SC', system-ui, 'Segoe UI', 'Microsoft YaHei', sans-serif;
  /* 浮层用偏 expo 的缓出曲线：控件是「滑进来」而不是「弹一下」 */
  --pip-ease: cubic-bezier(0.16, 1, 0.3, 1);
}

.pip-card {
  position: relative;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: transparent;
  cursor: grab;
}
/* 无边框：客户区就是画面本身，所以不画投影也不画描边 —— 只有钉住时给一条主题色
   细边，用来表示「这颗浮窗会一直跟着」，它压在画面边缘上，不影响整体观感。 */
.pip-root.pinned .pip-card {
  outline: 1.5px solid rgb(var(--accent-rgb));
  outline-offset: -1.5px;
}

/* 音频兜底底色：只在没有画面可透出时出现 */
.pip-placeholder {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(140deg, var(--media-surface-hi), var(--media-surface-lo) 70%);
}
.pip-placeholder-text {
  font-size: 11px;
  letter-spacing: 0.08em;
  color: rgba(255, 255, 255, 0.5);
}

/* ── 浮层：顶栏 / 底栏 ── */
.pip-top,
.pip-bottom {
  position: absolute;
  left: 0;
  right: 0;
  z-index: 2;
  display: flex;
  opacity: 0;
  pointer-events: none;
  transition:
    opacity 200ms var(--pip-ease),
    transform 260ms var(--pip-ease);
}
.pip-top {
  top: 0;
  height: 30px;
  align-items: center;
  gap: 4px;
  padding: 0 4px 0 9px;
  background: linear-gradient(180deg, rgba(0, 0, 0, 0.74), rgba(0, 0, 0, 0.28) 62%, rgba(0, 0, 0, 0));
  transform: translateY(-8px);
  cursor: move;
}
.pip-bottom {
  bottom: 0;
  flex-direction: column;
  align-items: stretch;
  background: linear-gradient(0deg, rgba(0, 0, 0, 0.8), rgba(0, 0, 0, 0.34) 58%, rgba(0, 0, 0, 0));
  transform: translateY(8px);
}
.pip-root.ui-on .pip-top,
.pip-root.ui-on .pip-bottom {
  opacity: 1;
  pointer-events: auto;
  transform: none;
}
/* 底栏晚 45ms 跟进：两层错开出现，动效才有次序感（收起时不延迟，跟随鼠标立刻消失） */
.pip-root.ui-on .pip-bottom {
  transition-delay: 45ms;
}

.pip-title {
  flex: 1;
  min-width: 0;
  font-size: 11px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-shadow: 0 1px 3px rgba(0, 0, 0, 0.7);
  cursor: move;
}

.pip-row {
  display: flex;
  align-items: center;
  gap: 1px;
  height: 30px;
  padding: 0 4px 1px;
  cursor: move;
}
.pip-spacer {
  flex: 1;
}
.pip-time {
  padding-left: 3px;
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
  color: rgba(255, 255, 255, 0.78);
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.7);
  white-space: nowrap;
  overflow: hidden;
}
.pip-time i {
  margin: 0 1px;
  font-style: normal;
  opacity: 0.5;
}

/* ── 按钮 ── */
.pip-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  flex-shrink: 0;
  border: none;
  border-radius: 7px;
  color: rgba(255, 255, 255, 0.9);
  cursor: pointer;
  transition:
    background var(--dur-fast, 120ms) var(--ease, ease),
    color var(--dur-fast, 120ms) var(--ease, ease),
    transform var(--dur-press, 90ms) var(--ease, ease);
}
.pip-btn:hover {
  background: rgba(255, 255, 255, 0.18);
  color: var(--on-media);
  transform: scale(1.06);
}
/* 图标描一层影：压不住亮色画面（白墙、雪景）时靠它保持可辨 */
.pip-btn svg {
  filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.62));
}
.pip-btn:active {
  transform: scale(0.94);
}
.pip-btn.on {
  color: rgb(var(--accent-rgb));
  background: rgba(0, 0, 0, 0.22);
}
.pip-play {
  width: 28px;
  height: 28px;
  margin-left: 1px;
}

/* ── 进度条 ── */
.pip-seek {
  position: relative;
  height: 10px;
  margin: 0 4px;
  cursor: pointer;
  display: flex;
  align-items: center;
}
.pip-seek::before {
  content: '';
  position: absolute;
  left: 0;
  right: 0;
  height: 2px;
  border-radius: 2px;
  background: rgba(255, 255, 255, 0.24);
  transition: height var(--dur-fast, 120ms) var(--ease, ease);
}
.pip-seek-fill {
  position: relative;
  height: 2px;
  border-radius: 2px;
  background: rgb(var(--accent-rgb));
  box-shadow: 0 0 6px rgba(0, 0, 0, 0.6);
  transition: height var(--dur-fast, 120ms) var(--ease, ease);
}
.pip-seek:hover::before,
.pip-seek:hover .pip-seek-fill {
  height: 4px;
}
/* 悬停浮出指针头：擦洗时有一个明确的抓握点 */
.pip-seek-fill::after {
  content: '';
  position: absolute;
  right: -5px;
  top: 50%;
  width: 10px;
  height: 10px;
  margin-top: -5px;
  border-radius: 50%;
  background: var(--on-media);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.65);
  opacity: 0;
  transform: scale(0.5);
  transition:
    opacity var(--dur-fast, 120ms) var(--pip-ease),
    transform 180ms var(--pip-ease);
}
.pip-seek:hover .pip-seek-fill::after {
  opacity: 1;
  transform: none;
}

@media (prefers-reduced-motion: reduce) {
  .pip-top,
  .pip-bottom,
  .pip-btn {
    transition: opacity 120ms linear;
    transform: none;
  }
}
</style>
