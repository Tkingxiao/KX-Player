<script setup lang="ts">
/** 播放栏：封面/信息/收藏/继续收听 | 模式/上下曲/播放 | 进度(书签点)/倍速/睡眠/音量/设备。 */
import { ref, computed, onMounted, onBeforeUnmount, watch } from 'vue'
import { usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { useLibraryStore } from '@/stores/library'
import { usePlaylistsStore } from '@/stores/playlists'
import { useUiStore } from '@/stores/ui'
import { resolveTrackCover } from '@/utils/covers'
import { fmtTime, fmtCountdown } from '@/utils/format'

const player = usePlayerStore()
const settings = useSettingsStore()
const library = useLibraryStore()
const playlists = usePlaylistsStore()
const ui = useUiStore()

const coverUrl = ref<string | null>(null)
let coverReqId = 0
const coverTick = ref(0)

function refreshCover(): void {
  const t = player.current
  if (!t) { coverUrl.value = null; return }
  const myId = ++coverReqId
  const hit = resolveTrackCover(t, (u) => {
    if (myId === coverReqId && u) { coverUrl.value = u; coverTick.value++ }
  })
  if (hit) coverUrl.value = hit
  else coverTick.value++
}

watch([() => player.currentId, () => library.revision], refreshCover, { immediate: true })

const progressPct = computed(() => {
  const d = player.duration || 0
  if (!d || !isFinite(d)) return 0
  return Math.min(100, (player.position / d) * 100)
})

const bufferedPct = computed(() => {
  const d = player.duration || 0
  if (!d || !isFinite(d)) return 0
  return Math.min(100, (player.buffered / d) * 100)
})

// ── 进度条拖拽 ──
const bar = ref<HTMLElement | null>(null)
const dragging = ref(false)
let pendingSeek = 0

function barPos(e: MouseEvent): number {
  const el = bar.value
  if (!el) return 0
  const rect = el.getBoundingClientRect()
  return Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
}

function onBarDown(e: MouseEvent): void {
  if (!player.duration) return
  dragging.value = true
  pendingSeek = barPos(e) * player.duration
  window.addEventListener('mousemove', onBarMove)
  window.addEventListener('mouseup', onBarUp)
}

function onBarMove(e: MouseEvent): void {
  pendingSeek = barPos(e) * player.duration
}

function onBarUp(): void {
  window.removeEventListener('mousemove', onBarMove)
  window.removeEventListener('mouseup', onBarUp)
  if (dragging.value) {
    dragging.value = false
    player.seek(pendingSeek)
  }
}

const displayPos = computed(() => (dragging.value ? pendingSeek : player.position))

// ── 模式图标 ──
const MODE_META = [
  { icon: 'repeat', label: '顺序循环' },
  { icon: 'shuffle', label: '随机播放' },
  { icon: 'repeat-one', label: '单曲循环' },
  { icon: 'stop', label: '播完停止' },
] as const

function cycleMode(): void {
  settings.mode = ((settings.mode + 1) % 4) as typeof settings.mode
  settings.scheduleSave()
}

function goLyrics(): void {
  if (player.currentId) ui.view = 'lyrics'
}

function openStage(): void {
  if (player.current?.isVideo) ui.view = 'stage'
}

function continueListening(): void {
  // 找最近未听完的条目续播
  let best: { id: string; playedAt: number } | null = null
  for (const t of library.allTracks) {
    const p = library.progress.get(t.id)
    if (!p || p.completed) continue
    const ratio = t.duration > 0 ? (p.positionMs / 1000) / t.duration : 0
    if (ratio > 0.01 && ratio < 0.95 && (!best || p.playedAt > best.playedAt)) {
      best = { id: t.id, playedAt: p.playedAt }
    }
  }
  if (best) {
    const t = library.trackIndex.get(best.id)
    if (t) void player.playTrack(t, [best.id], '继续收听', { fromLast: true })
  }
}

const isFavorite = computed(() => (player.currentId ? playlists.isFavorite(player.currentId) : false))

const speedLabel = computed(() => settings.speed.toFixed(2).replace(/0$/, '') + '×')

// 睡眠定时
const sleepOpen = ref(false)
const sleepCustom = ref(45)
const SLEEP_PRESETS = [15, 30, 45, 60, 90]

function setSleep(min: number, action: 'pause' | 'stop' | 'finishTrack' = 'pause'): void {
  player.setSleepTimer(min, action)
  sleepOpen.value = false
}

function cancelSleep(): void {
  player.setSleepTimer(0)
  sleepOpen.value = false
}

// 音量
const volOpen = ref(false)
let volHideTimer: ReturnType<typeof setTimeout> | null = null

function showVol(): void {
  if (volHideTimer) { clearTimeout(volHideTimer); volHideTimer = null }
  volOpen.value = true
}
function hideVol(): void {
  if (volHideTimer) clearTimeout(volHideTimer)
  volHideTimer = setTimeout(() => { volOpen.value = false }, 900)
}

const volIcon = computed(() => {
  if (settings.muted || settings.vol === 0) return 'muted'
  if (settings.vol < 0.33) return 'low'
  if (settings.vol < 0.66) return 'mid'
  return 'high'
})

function setVolFromEvent(e: MouseEvent): void {
  const el = e.currentTarget as HTMLElement
  const rect = el.getBoundingClientRect()
  player.setVolume(Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width)))
}

// 输出设备
async function pickDevice(id: string): Promise<void> {
  await player.setDevice(id)
  player.deviceOpen = false
}

function deviceLabel(d: { label: string }): string {
  return d.label || '默认输出设备'
}

let ticker: ReturnType<typeof setInterval> | null = null
onMounted(() => {
  ticker = setInterval(() => {
    if (player.sleep.active) {
      player.sleepRemaining = Math.max(0, player.sleep.endAt - Date.now())
    }
  }, 1000)
})
onBeforeUnmount(() => { if (ticker) clearInterval(ticker) })
</script>

<template>
  <footer id="player-bar">
    <!-- 左：封面 + 信息 -->
    <div class="pb-left">
      <button class="pb-cover" :title="player.current?.isVideo ? '打开视频舞台' : '打开歌词'" @click="player.current?.isVideo ? openStage() : goLyrics()">
        <img v-if="coverUrl" :src="coverUrl" alt="" :key="coverTick" />
        <svg v-else viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>
      </button>
      <div class="pb-meta">
        <div class="pb-title" :title="player.currentTitle">{{ player.currentTitle }}</div>
        <div class="pb-sub">
          <span>{{ player.currentArtist }}</span>
          <button
            class="pb-heart"
            :class="{ on: isFavorite }"
            :title="isFavorite ? '取消收藏' : '收藏'"
            @click="player.currentId && playlists.toggleFavorite(player.currentId)"
          >
            <svg viewBox="0 0 24 24" width="13" height="13" :fill="isFavorite ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="2"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z" /></svg>
          </button>
          <button
            v-if="player.current && !player.current.isVideo"
            class="pb-continue"
            title="从上次位置继续收听"
            @click="continueListening"
          >继续收听</button>
        </div>
      </div>
    </div>

    <!-- 中：控制 + 进度 -->
    <div class="pb-center">
      <div class="pb-controls">
        <button class="icon-btn" :title="MODE_META[settings.mode].label" @click="cycleMode">
          <!-- 顺序循环 -->
          <svg v-if="settings.mode === 0" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><polyline points="17,1 21,5 17,9" /><path d="M3 11V9a4 4 0 0 1 4-4h14" /><polyline points="7,23 3,19 7,15" /><path d="M21 13v2a4 4 0 0 1-4 4H3" /></svg>
          <!-- 随机 -->
          <svg v-else-if="settings.mode === 1" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><polyline points="16,3 21,3 21,8" /><line x1="4" y1="20" x2="21" y2="3" /><polyline points="21,16 21,21 16,21" /><line x1="15" y1="15" x2="21" y2="21" /><line x1="4" y1="4" x2="9" y2="9" /></svg>
          <!-- 单曲循环 -->
          <svg v-else-if="settings.mode === 2" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><polyline points="17,1 21,5 17,9" /><path d="M3 11V9a4 4 0 0 1 4-4h14" /><polyline points="7,23 3,19 7,15" /><path d="M21 13v2a4 4 0 0 1-4 4H3" /><text x="10.5" y="15.5" font-size="8" fill="currentColor" stroke="none" font-weight="bold">1</text></svg>
          <!-- 播完停止 -->
          <svg v-else viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="9" /><rect x="9" y="9" width="6" height="6" rx="1" fill="currentColor" stroke="none" /></svg>
        </button>

        <button class="icon-btn" title="上一首" @click="player.prev()">
          <svg viewBox="0 0 24 24" width="17" height="17" fill="currentColor"><polygon points="19,20 9,12 19,4" /><rect x="5" y="4" width="2" height="16" /></svg>
        </button>

        <button class="pb-play" :title="player.playing ? '暂停' : '播放'" @click="player.togglePlay()">
          <svg v-if="!player.playing" viewBox="0 0 24 24" width="20" height="20" fill="currentColor"><polygon points="6,3 21,12 6,21" /></svg>
          <svg v-else viewBox="0 0 24 24" width="20" height="20" fill="currentColor"><rect x="5" y="4" width="4.5" height="16" rx="1" /><rect x="14.5" y="4" width="4.5" height="16" rx="1" /></svg>
        </button>

        <button class="icon-btn" title="下一首" @click="player.next()">
          <svg viewBox="0 0 24 24" width="17" height="17" fill="currentColor"><polygon points="5,4 15,12 5,20" /><rect x="17" y="4" width="2" height="16" /></svg>
        </button>
      </div>

      <div class="pb-progress-row">
        <span class="pb-time tnum">{{ fmtTime(displayPos) }}</span>
        <div
          ref="bar"
          class="pb-progress"
          :class="{ dragging }"
          @mousedown="onBarDown"
        >
          <div class="pb-buffer" :style="{ width: bufferedPct + '%' }" />
          <div class="pb-played" :style="{ width: progressPct + '%' }">
            <div class="pb-thumb" />
          </div>
          <span
            v-for="bm in player.bookmarks"
            :key="bm.id"
            class="pb-bookmark"
            :style="{ left: (player.duration ? (bm.atMs / 1000 / player.duration) * 100 : 0) + '%' }"
            :title="'书签：' + (bm.label || fmtTime(bm.atMs / 1000))"
          />
        </div>
        <span class="pb-time tnum">{{ fmtTime(player.duration) }}</span>
      </div>
    </div>

    <!-- 右：倍速/睡眠/音量/设备/队列 -->
    <div class="pb-right">
      <button class="pb-speed tnum" title="播放速度（, . 调整）" @click="player.setSpeed(1)">
        {{ speedLabel }}
      </button>
      <button class="icon-btn" title="睡眠定时" @click="sleepOpen = !sleepOpen" :class="{ active: player.sleep.active }">
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="13" r="8" /><path d="M12 9v4l2.5 2.5" /><path d="M9 2h6" /></svg>
      </button>
      <span v-if="player.sleep.active" class="pb-sleep-remaining tnum">{{ fmtCountdown(player.sleepRemaining) }}</span>

      <div class="pb-vol-wrap" @mouseenter="showVol" @mouseleave="hideVol">
        <button class="icon-btn" :title="settings.muted ? '取消静音' : '静音'" @click="player.toggleMute()">
          <svg v-if="volIcon === 'muted'" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><polygon points="11,5 6,9 2,9 2,15 6,15 11,19" fill="currentColor" stroke="none" /><line x1="23" y1="9" x2="17" y2="15" /><line x1="17" y1="9" x2="23" y2="15" /></svg>
          <svg v-else viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><polygon points="11,5 6,9 2,9 2,15 6,15 11,19" fill="currentColor" stroke="none" /><path v-if="volIcon !== 'low'" d="M15.54 8.46a5 5 0 0 1 0 7.07" /><path v-if="volIcon === 'high'" d="M19.07 4.93a10 10 0 0 1 0 14.14" /></svg>
        </button>
        <div v-if="volOpen" class="pb-vol-panel">
          <div class="pb-vol-bar" @mousedown="setVolFromEvent" @click="setVolFromEvent">
            <div class="pb-vol-fill" :style="{ width: (settings.muted ? 0 : settings.vol * 100) + '%' }" />
          </div>
        </div>
      </div>

      <button
        class="icon-btn"
        :class="{ active: player.deviceOpen }"
        title="音频输出设备（右键音量图标切换）"
        @click="player.deviceOpen = !player.deviceOpen"
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><polygon points="11,5 6,9 2,9 2,15 6,15 11,19" fill="currentColor" stroke="none" /><circle cx="16" cy="12" r="2" /><circle cx="21" cy="12" r="2" /></svg>
      </button>

      <button
        class="icon-btn"
        :class="{ active: ui.inspectorOpen }"
        title="检查器（队列 / 信息 / 书签）"
        @click="ui.inspectorOpen = !ui.inspectorOpen"
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="18" height="18" rx="2" /><line x1="15" y1="3" x2="15" y2="21" /></svg>
      </button>

      <button
        class="icon-btn"
        :class="{ active: ui.queueOpen }"
        title="当前播放列表"
        @click="ui.queueOpen = !ui.queueOpen"
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" /></svg>
      </button>
    </div>

    <!-- 睡眠定时弹层 -->
    <Teleport to="body">
      <div v-if="sleepOpen" class="sleep-pop" @mousedown.self="sleepOpen = false">
        <div class="sleep-card">
          <div class="sleep-title">睡眠定时</div>
          <div class="sleep-grid">
            <button v-for="m in SLEEP_PRESETS" :key="m" class="sleep-opt" @click="setSleep(m)">
              {{ m }} 分钟
            </button>
          </div>
          <div class="sleep-custom">
            <input v-model.number="sleepCustom" type="number" min="1" max="600" />
            <button class="btn-ghost" @click="setSleep(sleepCustom)">开始</button>
            <button class="btn-ghost" title="播完当前曲目后停止" @click="setSleep(1, 'finishTrack')">播完再停</button>
          </div>
          <button v-if="player.sleep.active" class="btn-ghost sleep-cancel" @click="cancelSleep">取消定时</button>
        </div>
      </div>
    </Teleport>

    <!-- 设备弹层 -->
    <Teleport to="body">
      <div v-if="player.deviceOpen" class="sleep-pop" @mousedown.self="player.deviceOpen = false">
        <div class="sleep-card">
          <div class="sleep-title">音频输出设备</div>
          <button
            v-for="d in player.devices"
            :key="d.deviceId"
            class="ctx-like"
            :class="{ active: d.deviceId === settings.devId || (!settings.devId && d.deviceId === 'default') }"
            @click="pickDevice(d.deviceId)"
          >{{ deviceLabel(d) }}</button>
          <div v-if="!player.devices.length" class="sleep-empty">未枚举到设备</div>
        </div>
      </div>
    </Teleport>
  </footer>
</template>

<style scoped>
#player-bar {
  display: grid;
  grid-template-columns: minmax(220px, 1fr) minmax(360px, 2fr) minmax(220px, 1fr);
  align-items: center;
  gap: 16px;
  height: var(--playerbar-h);
  padding: 0 16px;
  background: color-mix(in srgb, var(--bg-player), transparent calc((1 - var(--pb-alpha, 1)) * 100%));
  border-top: 1px solid var(--border);
  position: relative;
  z-index: 20;
  backdrop-filter: blur(20px);
}

.pb-left {
  display: flex;
  align-items: center;
  gap: 12px;
  min-width: 0;
}
.pb-cover {
  width: 56px;
  height: 56px;
  border-radius: var(--radius);
  overflow: hidden;
  flex-shrink: 0;
  background: var(--bg-input);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  transition: transform var(--dur-fast) var(--ease);
}
.pb-cover:hover { transform: scale(1.04); }
.pb-cover img { width: 100%; height: 100%; object-fit: cover; }
.pb-meta { min-width: 0; }
.pb-title {
  font-size: 13px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.pb-sub {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  color: var(--text-muted);
  overflow: hidden;
  white-space: nowrap;
}
.pb-heart {
  display: inline-flex;
  color: var(--text-muted);
}
.pb-heart:hover, .pb-heart.on { color: #e6685f; }
.pb-continue {
  font-size: 10.5px;
  padding: 1px 8px;
  border-radius: 999px;
  border: 1px solid var(--border-strong);
  color: var(--text-muted);
}
.pb-continue:hover { color: rgb(var(--accent-rgb)); border-color: rgb(var(--accent-rgb)); }

.pb-center {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 4px;
  min-width: 0;
}
.pb-controls {
  display: flex;
  align-items: center;
  gap: 10px;
}
.pb-play {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 38px;
  height: 38px;
  border-radius: 50%;
  background: var(--text);
  color: var(--bg);
  transition: transform var(--dur-fast) var(--ease), filter var(--dur-fast) var(--ease);
}
.pb-play:hover { transform: scale(1.06); filter: brightness(1.05); }
.pb-progress-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
}
.pb-time {
  font-size: 10.5px;
  color: var(--text-muted);
  min-width: 40px;
  text-align: center;
}
.pb-progress {
  position: relative;
  flex: 1;
  height: 4px;
  border-radius: 2px;
  background: var(--bg-input);
  cursor: pointer;
  transition: height var(--dur-fast) var(--ease);
}
.pb-progress:hover, .pb-progress.dragging { height: 6px; }
.pb-buffer {
  position: absolute;
  left: 0;
  top: 0;
  height: 100%;
  border-radius: 2px;
  background: var(--bg-active);
}
.pb-played {
  position: absolute;
  left: 0;
  top: 0;
  height: 100%;
  border-radius: 2px;
  background: rgb(var(--accent-rgb));
}
.pb-thumb {
  position: absolute;
  right: -5px;
  top: 50%;
  width: 10px;
  height: 10px;
  border-radius: 50%;
  background: rgb(var(--accent-rgb));
  transform: translateY(-50%) scale(0);
  transition: transform var(--dur-fast) var(--ease);
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.4);
}
.pb-progress:hover .pb-thumb, .pb-progress.dragging .pb-thumb { transform: translateY(-50%) scale(1); }
.pb-bookmark {
  position: absolute;
  top: -3px;
  width: 3px;
  height: 10px;
  border-radius: 1px;
  background: #ffc857;
  transform: translateX(-1px);
}

.pb-right {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
}
.pb-speed {
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 999px;
  border: 1px solid var(--border-strong);
  color: var(--text-sub);
}
.pb-speed:hover { color: rgb(var(--accent-rgb)); border-color: rgb(var(--accent-rgb)); }
.pb-sleep-remaining {
  font-size: 11px;
  color: rgb(var(--accent-rgb));
}

.pb-vol-wrap { position: relative; display: flex; }
.pb-vol-panel {
  position: absolute;
  bottom: 34px;
  left: 50%;
  transform: translateX(-50%);
  padding: 12px 10px;
  border-radius: var(--radius);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  box-shadow: var(--shadow-pop);
}
.pb-vol-bar {
  width: 90px;
  height: 4px;
  border-radius: 2px;
  background: var(--bg-input);
  cursor: pointer;
}
.pb-vol-fill {
  height: 100%;
  border-radius: 2px;
  background: rgb(var(--accent-rgb));
  pointer-events: none;
}

.sleep-pop {
  position: fixed;
  inset: 0;
  z-index: 930;
}
.sleep-card {
  position: absolute;
  right: 120px;
  bottom: calc(var(--playerbar-h) + 12px);
  width: 260px;
  padding: 16px;
  border-radius: var(--radius-lg);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  box-shadow: var(--shadow-pop);
}
.sleep-title {
  font-size: 13px;
  font-weight: 600;
  margin-bottom: 12px;
}
.sleep-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 6px;
  margin-bottom: 10px;
}
.sleep-opt {
  padding: 7px 0;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  font-size: 12px;
  color: var(--text-sub);
}
.sleep-opt:hover { border-color: rgb(var(--accent-rgb)); color: rgb(var(--accent-rgb)); }
.sleep-custom {
  display: flex;
  gap: 6px;
  margin-bottom: 8px;
}
.sleep-custom input {
  width: 70px;
  padding: 5px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  outline: none;
  font-size: 12px;
}
.sleep-cancel { width: 100%; justify-content: center; }
.sleep-empty { font-size: 12px; color: var(--text-muted); padding: 6px; }
.ctx-like {
  display: block;
  width: 100%;
  text-align: left;
  padding: 8px 10px;
  border-radius: var(--radius-sm);
  font-size: 12px;
  color: var(--text-sub);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.ctx-like:hover { background: var(--bg-hover); }
.ctx-like.active { color: rgb(var(--accent-rgb)); background: var(--bg-selected); }
</style>
