<script setup lang="ts">
/**
 * 独立悬浮窗（画中画）入口 —— 仅在 label="pip" 的独立系统窗口内加载（index.html#/pip）。
 *
 * 与旧版 PipWindow（主窗口内绝对定位）不同：本窗口由 Rust 动态创建为真正的系统窗口，
 * 主窗口最小化/隐藏不影响它；mpv 覆盖窗口动态挂靠到本窗口之上（Rust 侧 attach overlay），
 * 因此画面区（.pip-surface）就是覆盖窗口的定位矩形，控制条放在矩形之外避免被原生窗口盖住。
 *
 * 能力：整窗拖动（顶部把手/底部条 data-tauri-drag-region）、4 档预设尺寸、右下角缩放、
 * 钉住（不随主窗口切换视图而关闭）、播放/暂停、关闭；几何与钉住状态持久化。
 */
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { getCurrentWindow, PhysicalSize } from '@tauri-apps/api/window'
import { api, onEvent } from '@/bridge/ipc'
import type { MpvPlayerState } from '@/contracts/api'

const win = getCurrentWindow()

// ── 播放状态 ──
const state = ref<MpvPlayerState>({
  playing: false, position: 0, duration: 0, speed: 1, volume: 85, muted: false,
  trackPath: null, isVideo: false, videoActive: false,
})

const title = computed(() => {
  const p = state.value.trackPath
  if (!p) return ''
  const seg = p.replace(/\\/g, '/').split('/')
  return seg[seg.length - 1] || p
})
const hasVideo = computed(() => state.value.isVideo || state.value.videoActive)

// ── 几何 / 交互 ──
const surfaceEl = ref<HTMLElement | null>(null)
const hovered = ref(false)
const pinned = ref(localStorage.getItem('kx.pipPinned') === '1')

const PIP_PRESETS = [
  { label: '小', w: 240, h: 135 },
  { label: '中', w: 320, h: 180 },
  { label: '大', w: 400, h: 225 },
  { label: '特大', w: 560, h: 315 },
]
const BAR_H = 30
const GRIP_H = 22

const sizeIndex = ref(1)

function dpr(): number {
  return window.devicePixelRatio || 1
}

/** 把 mpv 覆盖窗口对齐到画面区（物理像素，相对 pip 窗口客户区） */
function syncRect(): void {
  const el = surfaceEl.value
  if (!el) return
  const r = el.getBoundingClientRect()
  if (r.width <= 0 || r.height <= 0) return
  const d = dpr()
  void api.playerSetStageRect({
    x: Math.round(r.left * d),
    y: Math.round(r.top * d),
    w: Math.round(r.width * d),
    h: Math.round(r.height * d),
    visible: true,
  })
}

/** 窗口几何（物理像素）持久化，供主窗口下次打开时恢复 */
function persistRect(): void {
  const p = win.outerPosition()
  const s = win.outerSize()
  void Promise.all([p, s]).then(([pos, size]) => {
    try {
      localStorage.setItem('kx.pipRect', JSON.stringify({
        x: pos.x, y: pos.y, w: size.width, h: size.height,
      }))
    } catch { /* ignore */ }
  })
}

async function applySizeIndex(i: number): Promise<void> {
  sizeIndex.value = i
  const preset = PIP_PRESETS[i]
  const d = dpr()
  await win.setSize(new PhysicalSize(Math.round(preset.w * d), Math.round(preset.h * d)))
  void nextTick(syncRect)
}

function cycleSize(): void {
  void applySizeIndex((sizeIndex.value + 1) % PIP_PRESETS.length)
}

function startResize(e: MouseEvent): void {
  e.preventDefault()
  e.stopPropagation()
  void win.startResizeDragging('SouthEast')
}

function togglePinned(): void {
  pinned.value = !pinned.value
  localStorage.setItem('kx.pipPinned', pinned.value ? '1' : '0')
  void api.pipSetPinned(pinned.value)
}

function close(): void {
  void api.pipClose()
}

function togglePlay(): void {
  void api.playerToggle(state.value.playing)
}

let unStates: Array<() => void> = []
let ro: ResizeObserver | null = null

onMounted(() => {
  unStates.push(onEvent('player:state', (s: MpvPlayerState) => { state.value = s }))
  unStates.push(onEvent('player:ended', () => {}))
  unStates.push(onEvent('pip:shown', () => {
    void api.playerGetState().then((s) => { state.value = s })
    void nextTick(syncRect)
  }))
  // 冷启动时拉一次快照（事件流可能在此之前已开始）
  void api.playerGetState().then((s) => { state.value = s })

  // 窗口尺寸变化：跟随重放覆盖窗口矩形
  void win.onResized(() => {
    persistRect()
    void nextTick(syncRect)
  }).then((u) => unStates.push(u))
  void win.onMoved(() => {
    persistRect()
    void nextTick(syncRect)
  }).then((u) => unStates.push(u))

  ro = new ResizeObserver(() => syncRect())
  if (surfaceEl.value) ro.observe(surfaceEl.value)

  void nextTick(() => {
    syncRect()
    // 按当前物理尺寸推算预设档位（近似，用于高亮显示）
    void win.outerSize().then((s) => {
      const d = dpr()
      let best = 1
      let dist = Infinity
      PIP_PRESETS.forEach((p, i) => {
        const dd = Math.abs(s.width - p.w * d) + Math.abs(s.height - p.h * d)
        if (dd < dist) { dist = dd; best = i }
      })
      sizeIndex.value = best
    })
  })
})

onBeforeUnmount(() => {
  for (const u of unStates) u()
  unStates = []
  ro?.disconnect()
  ro = null
})
</script>

<template>
  <div class="pip-root" :class="{ hovered, pinned }" @mouseenter="hovered = true" @mouseleave="hovered = false">
    <!-- 顶部拖拽把手（位于覆盖窗口矩形之外，始终可拖） -->
    <div class="pip-grip" data-tauri-drag-region>
      <span class="pip-grip-title">{{ hasVideo ? title : '悬浮窗' }}</span>
      <svg class="pip-grip-dots" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
        <circle cx="9" cy="7" r="1.4" /><circle cx="15" cy="7" r="1.4" />
        <circle cx="9" cy="12" r="1.4" /><circle cx="15" cy="12" r="1.4" />
        <circle cx="9" cy="17" r="1.4" /><circle cx="15" cy="17" r="1.4" />
      </svg>
    </div>

    <!-- 画面区：mpv 覆盖窗口对齐到此矩形（控制条之外，避免被原生窗口盖住） -->
    <div ref="surfaceEl" class="pip-surface" />

    <!-- 底部控制条：悬停浮现；空白区为拖拽把手 -->
    <div class="pip-bar" data-tauri-drag-region>
      <button
        class="pip-btn"
        :class="{ on: state.playing }"
        :title="state.playing ? '暂停' : '播放'"
        aria-label="播放暂停"
        @mousedown.stop
        @click="togglePlay"
      >
        <svg v-if="!state.playing" viewBox="0 0 24 24" width="13" height="13" fill="currentColor"><polygon points="6,3 21,12 6,21" /></svg>
        <svg v-else viewBox="0 0 24 24" width="13" height="13" fill="currentColor"><rect x="5" y="4" width="4.5" height="16" rx="1" /><rect x="14.5" y="4" width="4.5" height="16" rx="1" /></svg>
      </button>
      <button
        class="pip-btn"
        :title="`尺寸：${PIP_PRESETS[sizeIndex]?.label ?? '中'}（点击切换）`"
        aria-label="切换尺寸"
        @mousedown.stop
        @click="cycleSize"
      >
        <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 4h7v7H4zM13 13h7v7h-7z" /></svg>
      </button>
      <button
        class="pip-btn"
        :class="{ active: pinned }"
        :title="pinned ? '取消钉住（切换页面时自动关闭）' : '钉住（切换页面时保持显示）'"
        aria-label="钉住悬浮窗"
        @mousedown.stop
        @click="togglePinned"
      >
        <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 17v5" /><path d="M9 10.8V5h6v5.8l2 2.2H7z" /></svg>
      </button>
      <button class="pip-btn" title="关闭悬浮窗" aria-label="关闭悬浮窗" @mousedown.stop @click="close">
        <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
      </button>
    </div>

    <!-- 右下角缩放手柄（控制条区域内） -->
    <div class="pip-resize" title="拖拽调整大小" @mousedown="startResize" />
  </div>
</template>

<style>
/* 独立悬浮窗全局样式：窗口透明，WebView 只负责把手/控制条 */
html,
body {
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
  display: flex;
  flex-direction: column;
  border-radius: 10px;
  overflow: hidden;
  background: rgba(0, 0, 0, 0.85);
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.55);
  color: #fff;
  font-family: system-ui, 'Segoe UI', 'Microsoft YaHei', sans-serif;
}
.pip-root.pinned {
  box-shadow: 0 12px 40px rgba(0, 0, 0, 0.55), 0 0 0 1.5px rgb(var(--accent-rgb, 232, 104, 95));
}

/* 顶部把手：常驻可拖；悬停显示标题 */
.pip-grip {
  height: 22px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  cursor: move;
  background: linear-gradient(180deg, rgba(0, 0, 0, 0.65), rgba(0, 0, 0, 0));
}
.pip-grip-title {
  font-size: 10px;
  color: rgba(255, 255, 255, 0.72);
  max-width: 70%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  opacity: 0;
  transition: opacity 120ms ease;
}
.pip-grip-dots {
  opacity: 0.5;
}
.pip-root.hovered .pip-grip-title {
  opacity: 1;
}

/* 画面区：覆盖窗口矩形；占据把手与底部控制条之间的全部空间 */
.pip-surface {
  flex: 1;
  min-height: 0;
  background: #000;
}

/* 底部控制条：默认半透明拖拽条，悬停时浮现按钮 */
.pip-bar {
  height: 30px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 0 6px;
  cursor: move;
  background: rgba(0, 0, 0, 0.55);
  border-top: 1px solid rgba(255, 255, 255, 0.08);
  opacity: 0.55;
  transition: opacity 120ms ease;
}
.pip-root.hovered .pip-bar {
  opacity: 1;
}
.pip-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  border-radius: 5px;
  color: rgba(255, 255, 255, 0.85);
  flex-shrink: 0;
  transition: background 100ms ease, color 100ms ease;
}
.pip-btn:hover {
  background: rgba(255, 255, 255, 0.16);
  color: #fff;
}
.pip-btn.active {
  color: rgb(var(--accent-rgb, 232, 104, 95));
}

/* 右下角缩放手柄 */
.pip-resize {
  position: absolute;
  right: 0;
  bottom: 0;
  width: 18px;
  height: 18px;
  cursor: nwse-resize;
  z-index: 2;
}
.pip-resize::after {
  content: '';
  position: absolute;
  right: 4px;
  bottom: 4px;
  width: 7px;
  height: 7px;
  border-right: 2px solid rgba(255, 255, 255, 0.65);
  border-bottom: 2px solid rgba(255, 255, 255, 0.65);
  border-radius: 0 0 2px 0;
}
</style>
