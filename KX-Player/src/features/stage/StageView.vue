<script setup lang="ts">
/**
 * 视频舞台：画面由 libmpv 渲染到 Rust 侧「覆盖窗口」（本组件只负责该区域的几何与控制 UI）。
 * mpv 直解 MP4/MKV/AVI/FLV/WMV/TS/WebM/MOV，无需重封装；
 * 控制条 2.5s 无操作淡出（淡出时覆盖窗口几何扩大到整个舞台，避免黑边被 UI 占用）。
 * 交互对齐主流播放器（VLC / PotPlayer / mpv）：单击画面切换控制条、双击/F 全屏、
 * 进度条可拖拽、音量/倍速/仅音频/上下曲一应俱全。
 */
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { useUiStore } from '@/stores/ui'
import { usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { api } from '@/bridge/ipc'
import { fmtTime } from '@/utils/format'

const ui = useUiStore()
const player = usePlayerStore()
const settings = useSettingsStore()

const stageEl = ref<HTMLElement | null>(null)
const screenEl = ref<HTMLElement | null>(null)
const seekEl = ref<HTMLElement | null>(null)

const shouldShow = computed(
  () => ui.view === 'stage' && !!player.current?.isVideo && player.videoMode === 'video' && !player.audioOnly,
)

// ── mpv 覆盖窗口几何同步 ──
function sendRect(visible: boolean, expand: boolean): void {
  const dpr = window.devicePixelRatio || 1
  if (!visible) {
    void api.playerSetStageRect({ x: 0, y: 0, w: 0, h: 0, visible: false })
    return
  }
  const target = expand ? stageEl.value : screenEl.value
  if (!target) return
  const r = target.getBoundingClientRect()
  void api.playerSetStageRect({
    x: r.left * dpr,
    y: r.top * dpr,
    w: r.width * dpr,
    h: r.height * dpr,
    visible: true,
  })
}

function syncStage(): void {
  sendRect(shouldShow.value, shouldShow.value && !controlsVisible.value)
}

let ro: ResizeObserver | null = null

// ★ 修复：控件显隐变化必须用 getter 观察（直接传 .value 是原始值，永远不会触发）
watch(
  [() => ui.view, () => player.currentId, () => player.videoMode, () => player.audioOnly, () => controlsVisible.value],
  () => {
    void nextTick(syncStage)
  },
)

// ── 控制条自动隐藏 ──
const controlsVisible = ref(true)
let hideTimer: ReturnType<typeof setTimeout> | null = null

function poke(): void {
  controlsVisible.value = true
  if (hideTimer) clearTimeout(hideTimer)
  hideTimer = setTimeout(() => {
    if (player.playing && !dragging.value) {
      controlsVisible.value = false
      syncStage()
    }
  }, 2500)
}

// ── 单击画面：切换控制条；双击：全屏 ──
let clickTimer: ReturnType<typeof setTimeout> | null = null
function onScreenClick(): void {
  if (clickTimer) clearTimeout(clickTimer)
  clickTimer = setTimeout(() => {
    controlsVisible.value = !controlsVisible.value
    if (controlsVisible.value) poke()
    else syncStage()
  }, 240)
}
function onScreenDblClick(): void {
  if (clickTimer) clearTimeout(clickTimer)
  toggleFullscreen()
}

// ── 全屏：OS 窗口全屏（mpv 几何随 RO 同步）；Esc 退出全屏不退出播放 ──
const fullscreen = ref(false)

function toggleFullscreen(): void {
  void api.toggleFullscreen().then((on) => {
    fullscreen.value = on
    poke()
  })
}

function onKey(e: KeyboardEvent): void {
  if (ui.view !== 'stage') return
  const tag = (e.target as HTMLElement)?.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA') return
  if (e.key === 'f' || e.key === 'F') {
    e.preventDefault()
    toggleFullscreen()
  } else if (e.key === 'Escape' && fullscreen.value) {
    void api.toggleFullscreen().then((on) => { fullscreen.value = on })
  } else if (e.key === 'm' || e.key === 'M') {
    player.toggleMute()
  }
}

function goBack(): void {
  ui.view = 'smart'
}

// ── 进度（可拖拽）──
const progressPct = computed(() => {
  const d = player.duration || 0
  return d > 0 ? Math.min(100, (player.position / d) * 100) : 0
})
const dragging = ref(false)
const dragPct = ref(0)
const shownPct = computed(() => (dragging.value ? dragPct.value : progressPct.value))

function seekToPct(pct: number): void {
  player.seek(Math.max(0, Math.min(1, pct)) * (player.duration || 0))
}

function onSeekPointerDown(e: PointerEvent): void {
  const el = seekEl.value
  if (!el) return
  e.preventDefault()
  dragging.value = true
  const apply = (ev: PointerEvent) => {
    const rect = el.getBoundingClientRect()
    dragPct.value = Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width)) * 100
  }
  apply(e)
  el.setPointerCapture(e.pointerId)
  const onMove = (ev: PointerEvent) => { apply(ev); poke() }
  const onUp = (ev: PointerEvent) => {
    el.removeEventListener('pointermove', onMove)
    el.removeEventListener('pointerup', onUp)
    el.releasePointerCapture?.(e.pointerId)
    apply(ev)
    seekToPct(dragPct.value / 100)
    dragging.value = false
    poke()
  }
  el.addEventListener('pointermove', onMove)
  el.addEventListener('pointerup', onUp)
  poke()
}

// ── 音量 / 倍速 ──
const volPct = computed(() => Math.round((settings.muted ? 0 : settings.vol) * 100))

function onVolChange(e: Event): void {
  player.setVolume(Number((e.target as HTMLInputElement).value) / 100)
  poke()
}

function cycleSpeed(): void {
  // 0.5 → 0.75 → 1.0 → 1.25 → 1.5 → 1.75 → 2.0 → 1.0
  const steps = [0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0]
  const cur = settings.speed
  const next = steps.find((s) => s > cur + 0.001) ?? steps[0]
  player.setSpeed(next)
  poke()
}

// ── 生命周期 ──
onMounted(() => {
  poke()
  window.addEventListener('keydown', onKey)
  ro = new ResizeObserver(() => syncStage())
  if (screenEl.value) ro.observe(screenEl.value)
  if (stageEl.value) ro.observe(stageEl.value)
  void nextTick(syncStage)
})

onBeforeUnmount(() => {
  if (hideTimer) clearTimeout(hideTimer)
  if (clickTimer) clearTimeout(clickTimer)
  window.removeEventListener('keydown', onKey)
  ro?.disconnect()
  // 离开舞台：隐藏 mpv 覆盖窗口
  sendRect(false, false)
})
</script>

<template>
  <div ref="stageEl" class="stage-view" :class="{ hide: !controlsVisible }" @mousemove="poke" @mousedown="poke">
    <button class="icon-btn stage-back" :class="{ faded: !controlsVisible }" title="返回（Esc）" @click="goBack">
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><polyline points="15,18 9,12 15,6" /></svg>
    </button>

    <div class="stage-title" :class="{ faded: !controlsVisible }">
      {{ player.currentTitle }} · {{ player.currentArtist }}
    </div>

    <!-- 画面区：mpv 覆盖窗口渲染在此矩形之上（占位元素仅提供几何与背景） -->
    <div
      ref="screenEl"
      class="stage-screen"
      @click="onScreenClick"
      @dblclick="onScreenDblClick"
    >
      <div v-if="player.loading" class="stage-center-tip">
        <span class="stage-spinner" />
        <p>正在加载…</p>
      </div>

      <div v-else-if="player.error" class="stage-center-tip">
        <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="12" cy="12" r="9" /><path d="M12 7v6" /><circle cx="12" cy="16.5" r="0.5" /></svg>
        <p>{{ player.error }}</p>
      </div>

      <div v-if="!shouldShow" class="stage-audio-fallback">
        <svg viewBox="0 0 24 24" width="64" height="64" fill="none" stroke="currentColor" stroke-width="1"><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>
        <p class="stage-fallback-title">{{ player.currentTitle }}</p>
        <p v-if="player.audioOnly" class="stage-fallback-notice">仅音频模式：画面已关闭，播放位置不丢失</p>
      </div>
    </div>

    <!-- 底部控制条 -->
    <div class="stage-controls" :class="{ faded: !controlsVisible }">
      <button class="icon-btn" title="上一曲" @click="player.prev()">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><polygon points="6,4 6,20 8,20 8,4" /><polygon points="20,4 9,12 20,20" /></svg>
      </button>
      <button class="icon-btn" :title="player.playing ? '暂停' : '播放'" @click="player.togglePlay()">
        <svg v-if="!player.playing" viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><polygon points="6,3 21,12 6,21" /></svg>
        <svg v-else viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><rect x="5" y="4" width="4.5" height="16" rx="1" /><rect x="14.5" y="4" width="4.5" height="16" rx="1" /></svg>
      </button>
      <button class="icon-btn" title="下一曲" @click="player.next()">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><polygon points="16,4 16,20 18,20 18,4" /><polygon points="4,4 15,12 4,20" /></svg>
      </button>

      <span class="stage-time tnum">{{ fmtTime(player.position) }}</span>
      <div ref="seekEl" class="stage-progress" :class="{ dragging }" @pointerdown="onSeekPointerDown">
        <div class="stage-progress-fill" :style="{ width: shownPct + '%' }" />
      </div>
      <span class="stage-time tnum">{{ fmtTime(player.duration) }}</span>

      <button
        class="icon-btn"
        :class="{ active: settings.muted || settings.vol === 0 }"
        :title="settings.muted ? '取消静音' : '静音（M）'"
        @click="player.toggleMute()"
      >
        <svg v-if="settings.muted || settings.vol === 0" viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><path d="M3 9v6h4l5 4V5L7 9H3z" /><path d="M16 8l5 8M21 8l-5 8" stroke="currentColor" stroke-width="1.5" /></svg>
        <svg v-else-if="settings.vol < 0.5" viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><path d="M3 9v6h4l5 4V5L7 9H3z" /><path d="M16 9.5a4 4 0 0 1 0 5" stroke="currentColor" stroke-width="1.5" fill="none" /></svg>
        <svg v-else viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><path d="M3 9v6h4l5 4V5L7 9H3z" /><path d="M16 8.5a5.5 5.5 0 0 1 0 7M18.5 6a9 9 0 0 1 0 12" stroke="currentColor" stroke-width="1.5" fill="none" /></svg>
      </button>
      <input
        class="stage-vol"
        type="range"
        min="0"
        max="100"
        step="1"
        :value="volPct"
        @input="onVolChange"
        title="音量（↑↓）"
      />

      <button class="icon-btn stage-speed tnum" :title="'倍速（，/。）'" @click="cycleSpeed">
        {{ settings.speed.toFixed(2).replace(/\.?0+$/, '') }}×
      </button>

      <button
        class="icon-btn"
        :class="{ active: player.audioOnly }"
        title="仅音频模式（关画面）"
        @click="player.setAudioOnly(!player.audioOnly)"
      >
        <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor"><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>
      </button>

      <button class="icon-btn" :class="{ active: fullscreen }" :title="fullscreen ? '退出全屏（Esc）' : '全屏（F）'" @click="toggleFullscreen">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" /></svg>
      </button>
    </div>
  </div>
</template>

<style scoped>
.stage-view {
  position: absolute;
  inset: 0;
  z-index: 15;
  background: var(--stage-bg);
  display: flex;
  flex-direction: column;
}
.stage-back {
  position: absolute;
  top: 14px;
  left: 16px;
  z-index: 3;
  color: rgba(255, 255, 255, 0.8);
  transition: opacity 300ms var(--ease);
  background: rgba(0, 0, 0, 0.25);
  border-radius: 8px;
  backdrop-filter: blur(6px);
}
.stage-back:hover { color: #fff; background: rgba(0, 0, 0, 0.45); }
.stage-title {
  position: absolute;
  top: 16px;
  left: 56px;
  z-index: 2;
  color: rgba(255, 255, 255, 0.85);
  font-size: 13px;
  text-shadow: 0 1px 4px rgba(0, 0, 0, 0.6);
  transition: opacity 300ms var(--ease);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: calc(100% - 100px);
}
.stage-screen {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: default;
}
.stage-audio-fallback {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  color: rgba(255, 255, 255, 0.5);
}
.stage-fallback-title {
  font-size: 15px;
  color: rgba(255, 255, 255, 0.8);
}
.stage-fallback-notice {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.45);
  max-width: 440px;
  text-align: center;
}
.stage-center-tip {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 12px;
  color: rgba(255, 255, 255, 0.6);
  font-size: 13px;
}
.stage-spinner {
  width: 26px;
  height: 26px;
  border-radius: 50%;
  border: 3px solid rgba(255, 255, 255, 0.18);
  border-top-color: rgb(var(--accent-rgb));
  animation: stage-spin 0.8s linear infinite;
}
@keyframes stage-spin {
  to { transform: rotate(360deg); }
}
.stage-controls {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 18px;
  color: rgba(255, 255, 255, 0.85);
  background: linear-gradient(transparent, rgba(0, 0, 0, 0.72));
  transition: opacity 300ms var(--ease);
  flex-shrink: 0;
}
.stage-controls .icon-btn { color: rgba(255, 255, 255, 0.8); }
.stage-controls .icon-btn:hover { color: #fff; background: rgba(255, 255, 255, 0.1); }
.faded { opacity: 0; pointer-events: none; }
.stage-time { font-size: 11px; color: rgba(255, 255, 255, 0.6); min-width: 40px; text-align: center; }
.stage-progress {
  flex: 1;
  height: 4px;
  border-radius: 2px;
  background: rgba(255, 255, 255, 0.18);
  cursor: pointer;
  position: relative;
  transition: height 120ms var(--ease);
}
.stage-progress:hover,
.stage-progress.dragging { height: 7px; }
.stage-progress-fill {
  height: 100%;
  border-radius: 2px;
  background: rgb(var(--accent-rgb));
  position: relative;
}
.stage-progress-fill::after {
  content: '';
  position: absolute;
  right: -5px;
  top: 50%;
  transform: translateY(-50%);
  width: 11px;
  height: 11px;
  border-radius: 50%;
  background: #fff;
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.5);
  opacity: 0;
  transition: opacity 120ms var(--ease);
}
.stage-progress:hover .stage-progress-fill::after,
.stage-progress.dragging .stage-progress-fill::after { opacity: 1; }
.stage-vol {
  width: 72px;
  accent-color: rgb(var(--accent-rgb));
  height: 4px;
  cursor: pointer;
}
.stage-speed {
  font-size: 12px;
  min-width: 40px;
  font-weight: 600;
}
</style>
