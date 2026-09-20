<script setup lang="ts">
/**
 * 视频舞台：画面由 libmpv 渲染到 Rust 侧「覆盖窗口」（本组件只负责该区域的几何）。
 *
 * 顶部控制条（返回 / 标题 / 外观与设置）留在 WebView 内，并把 mpv 嵌入矩形的
 * 顶部让出同样高度 —— 否则覆盖窗口是原生顶层窗口，会盖住这些按钮导致点不到。
 * 交互：单击画面切换播放/暂停；双击全屏；F 全屏；Esc 退出全屏。
 */
import { ref, computed, watch, onMounted, onBeforeUnmount, nextTick } from 'vue'
import { useUiStore } from '@/stores/ui'
import { usePlayerStore } from '@/stores/player'
import { api } from '@/bridge/ipc'

const ui = useUiStore()
const player = usePlayerStore()

const stageEl = ref<HTMLElement | null>(null)
const surfaceEl = ref<HTMLElement | null>(null)

/** 顶部控制条高度（px）：mpv 矩形从这里下方开始 */
const TOPBAR_H = 44

const isStage = computed(() => ui.view === 'stage')

const shouldShow = computed(
  () => isStage.value && !ui.pipEnabled && !!player.current?.isVideo && player.videoMode === 'video' && !player.audioOnly,
)

// ── mpv 覆盖窗口几何同步 ──
function syncStage(): void {
  const dpr = window.devicePixelRatio || 1
  // PiP 激活时由 applyPipRect 接管几何，跳过舞台同步
  if (ui.pipEnabled) return
  const el = surfaceEl.value
  if (!shouldShow.value || !el) {
    void api.playerSetStageRect({ x: 0, y: 0, w: 0, h: 0, visible: false })
    return
  }
  const r = el.getBoundingClientRect()
  if (r.width <= 0 || r.height <= 0) {
    void api.playerSetStageRect({ x: 0, y: 0, w: 0, h: 0, visible: false })
    return
  }
  void api.playerSetStageRect({
    x: r.left * dpr,
    y: r.top * dpr,
    w: r.width * dpr,
    h: r.height * dpr,
    visible: true,
  })
}

let ro: ResizeObserver | null = null

watch(
  [() => ui.view, () => player.currentId, () => player.videoMode, () => player.audioOnly, () => ui.pipEnabled],
  () => {
    void nextTick(syncStage)
  },
)

// ── 视频遮挡压制 ──────────────────────────────────────────────
// mpv 覆盖窗口是原生顶层窗口，恒在 WebView 之上；播放栏弹窗/设置模态等
// 打开时先把覆盖窗口藏起来，关闭后再按舞台矩形恢复。
// 悬浮窗挂靠期间画面归 PipRoot 管，主窗口弹窗不干预浮窗。
watch(() => ui.videoBlocked, (blocked) => {
  if (ui.pipEnabled) return
  if (blocked) {
    void api.playerSetStageRect({ x: 0, y: 0, w: 0, h: 0, visible: false })
  } else {
    void nextTick(syncStage)
  }
})

// ── 画中画（PiP）─────────────────────────────────────────────
// 悬浮窗是独立系统窗口（pip），几何/交互在 PipRoot；这里只负责：
// 开启 PiP 时隐藏舞台画面，避免与浮窗争抢同一个 mpv 覆盖窗口。
const pipEnabled = computed(() => ui.pipEnabled)

// 视图切换的联动：回舞台时关闭未钉住的浮窗（钉住 = 用户希望它一直浮着）；
// 离开舞台且未开浮窗时隐藏覆盖窗口，避免画面残留在其他视图上。
watch(() => ui.view, (view) => {
  if (view === 'stage') {
    if (ui.pipEnabled && !ui.pipPinned) void ui.closePip()
  } else if (!ui.pipEnabled) {
    void api.playerSetStageRect({ x: 0, y: 0, w: 0, h: 0, visible: false })
  }
})

// ── 返回：优先回到进入舞台前的视图 ──
function goBackFromStage(): void {
  ui.goBack()
}

// ── 全屏 ──
const fullscreen = ref(false)

function toggleFullscreen(): void {
  void api.toggleFullscreen().then((on) => {
    fullscreen.value = on
    void nextTick(syncStage)
  })
}

function openSettings(): void {
  ui.settingsOpen = true
}

function onKey(e: KeyboardEvent): void {
  if (ui.view !== 'stage' && !pipEnabled.value) return
  const tag = (e.target as HTMLElement)?.tagName
  if (tag === 'INPUT' || tag === 'TEXTAREA') return
  if (e.key === 'f' || e.key === 'F') {
    e.preventDefault()
    toggleFullscreen()
  } else if (e.key === 'Escape' && fullscreen.value) {
    void api.toggleFullscreen().then((on) => { fullscreen.value = on; void nextTick(syncStage) })
  } else if (e.key === ' ') {
    e.preventDefault()
    player.togglePlay()
  }
}

function onScreenClick(): void {
  player.togglePlay()
}

function onScreenDblClick(): void {
  void toggleFullscreen()
}

onMounted(() => {
  window.addEventListener('keydown', onKey)
  ro = new ResizeObserver(() => syncStage())
  if (stageEl.value) ro.observe(stageEl.value)
  if (surfaceEl.value) ro.observe(surfaceEl.value)
  syncStage()
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onKey)
  ro?.disconnect()
  // 卸载时若不在 PiP，务必隐藏 mpv 覆盖窗口，避免画面残留
  if (!ui.pipEnabled) {
    void api.playerSetStageRect({ x: 0, y: 0, w: 0, h: 0, visible: false })
  }
})
</script>

<template>
  <section ref="stageEl" class="stage" :class="{ pip: pipEnabled }">
    <!-- 顶部控制条：位于 WebView 内，mpv 矩形从它下方开始，故按钮可点击 -->
    <div v-if="isStage && !pipEnabled" class="stage-topbar">
      <button class="stage-btn stage-back" title="返回" aria-label="返回" @click="goBackFromStage">
        <svg viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2">
          <polyline points="15,18 9,12 15,6" />
        </svg>
      </button>
      <div class="stage-meta">
        <div class="stage-title" :title="player.currentTitle">{{ player.currentTitle }}</div>
        <div class="stage-sub">{{ player.currentArtist }}</div>
      </div>
      <button
        class="stage-btn"
        :class="{ active: ui.settingsOpen }"
        title="外观与设置"
        aria-label="外观与设置"
        @click="openSettings"
      >
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
          <circle cx="12" cy="12" r="5" />
          <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
        </svg>
      </button>
    </div>

    <!-- 画面区（mpv 几何对应元素）；单击播放/暂停、双击全屏 -->
    <div
      ref="surfaceEl"
      class="stage-surface"
      :class="{ 'with-topbar': isStage && !pipEnabled }"
      @click="onScreenClick"
      @dblclick="onScreenDblClick"
    />
  </section>
</template>

<style scoped>
.stage {
  position: absolute;
  inset: 0;
  background: var(--stage-bg);
  z-index: 5;
  cursor: default;
}
.stage.pip {
  pointer-events: none;
  background: transparent;
}
.stage-topbar {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 44px;
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 12px;
  z-index: 2;
  background: linear-gradient(180deg, rgba(0, 0, 0, 0.72), rgba(0, 0, 0, 0));
  color: #fff;
}
.stage-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: var(--radius-sm);
  color: rgba(255, 255, 255, 0.82);
  flex-shrink: 0;
  transition: background var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease);
}
.stage-btn:hover {
  background: rgba(255, 255, 255, 0.14);
  color: #fff;
}
.stage-btn.active {
  color: rgb(var(--accent-rgb));
}
.stage-meta {
  flex: 1;
  min-width: 0;
}
.stage-title {
  font-size: 13px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.stage-sub {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.62);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* 画面区：占满除顶栏外的区域，mpv 覆盖窗口与之对齐 */
.stage-surface {
  position: absolute;
  inset: 0;
  cursor: pointer;
}
.stage-surface.with-topbar {
  top: 44px;
}
</style>
