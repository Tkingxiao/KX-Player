<script setup lang="ts">
/** 检查器：队列（可跳转/移除）/ 信息 / 书签 三 Tab。宽度可拖拽（260–480，持久化）。 */
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { useUiStore } from '@/stores/ui'
import { usePlayerStore } from '@/stores/player'
import { useLibraryStore } from '@/stores/library'
import { useSettingsStore } from '@/stores/settings'
import { fmtTime, trackName, trackArtist } from '@/utils/format'
import type { Track } from '@/contracts/api'

const ui = useUiStore()
const player = usePlayerStore()
const library = useLibraryStore()
const settings = useSettingsStore()

// P0-1：拖拽左缘分隔条调宽（260–480），持久化
let resizeMove: ((ev: MouseEvent) => void) | null = null
let resizeUp: (() => void) | null = null

function startResize(e: MouseEvent): void {
  e.preventDefault()
  const startX = e.clientX
  const startW = settings.inspectorWidth
  resizeMove = (ev: MouseEvent) => {
    settings.inspectorWidth = Math.max(260, Math.min(480, startW - (ev.clientX - startX)))
  }
  resizeUp = () => {
    window.removeEventListener('mousemove', resizeMove!)
    window.removeEventListener('mouseup', resizeUp!)
    resizeMove = null
    resizeUp = null
    settings.scheduleSave()
  }
  window.addEventListener('mousemove', resizeMove)
  window.addEventListener('mouseup', resizeUp)
}

onBeforeUnmount(() => {
  if (resizeMove) window.removeEventListener('mousemove', resizeMove)
  if (resizeUp) window.removeEventListener('mouseup', resizeUp)
})

const queueTracks = computed<Track[]>(() =>
  player.queue.map((id) => library.trackIndex.get(id)).filter((t): t is Track => !!t),
)

function playAt(i: number): void {
  void player.playFromList(player.queue, i, player.queueName, { fromLast: true })
}

function removeAt(i: number): void {
  const q = [...player.queue]
  q.splice(i, 1)
  player.queue = q
}

// 书签
const bookmarkLabel = ref('')
const renamingId = ref<string | null>(null)
const renameValue = ref('')

watch(() => player.currentId, () => { bookmarkLabel.value = '' })

async function addBookmark(): Promise<void> {
  await player.addBookmarkAt(bookmarkLabel.value.trim())
  bookmarkLabel.value = ''
}
</script>

<template>
  <aside id="inspector" :style="{ width: settings.inspectorWidth + 'px' }">
    <div class="insp-resizer" @mousedown="startResize" />
    <div class="insp-tabs">
      <button class="insp-tab" :class="{ active: ui.inspectorTab === 'queue' }" @click="ui.inspectorTab = 'queue'">队列</button>
      <button class="insp-tab" :class="{ active: ui.inspectorTab === 'info' }" @click="ui.inspectorTab = 'info'">信息</button>
      <button class="insp-tab" :class="{ active: ui.inspectorTab === 'bookmark' }" @click="ui.inspectorTab = 'bookmark'">书签</button>
      <button class="icon-btn insp-close" title="关闭" @click="ui.inspectorOpen = false">✕</button>
    </div>

    <div class="insp-body">
      <!-- 队列 -->
      <div v-if="ui.inspectorTab === 'queue'" class="insp-queue">
        <div class="insp-queue-name">{{ player.queueName || '当前队列' }} · {{ queueTracks.length }} 首</div>
        <div v-if="!queueTracks.length" class="empty-state"><p>队列为空</p></div>
        <div
          v-for="(t, i) in queueTracks"
          :key="t.id + i"
          class="queue-row"
          :class="{ current: t.id === player.currentId }"
          @dblclick="playAt(i)"
        >
          <span class="queue-idx tnum">{{ i + 1 }}</span>
          <div class="queue-meta">
            <div class="queue-name">{{ trackName(t) }}</div>
            <div class="queue-sub">{{ trackArtist(t) }}</div>
          </div>
          <span class="queue-dur tnum">{{ fmtTime(t.duration) }}</span>
          <button class="icon-btn queue-remove" title="移出队列" @click="removeAt(i)">✕</button>
        </div>
      </div>

      <!-- 信息 -->
      <div v-else-if="ui.inspectorTab === 'info'" class="insp-info">
        <template v-if="player.current">
          <div class="info-cover">
            <svg viewBox="0 0 24 24" width="30%" height="30%" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>
          </div>
          <div class="info-name">{{ player.currentTitle }}</div>
          <div class="info-artist">{{ player.currentArtist }}</div>
          <div class="info-grid">
            <span class="info-k">格式</span><span>{{ player.current.format.toUpperCase() }}</span>
            <span class="info-k">时长</span><span class="tnum">{{ fmtTime(player.duration) }}</span>
            <span v-if="player.current.sampleRate" class="info-k">采样率</span><span v-if="player.current.sampleRate" class="tnum">{{ (player.current.sampleRate / 1000).toFixed(1) }}kHz</span>
            <span v-if="player.current.bitrate" class="info-k">比特率</span><span v-if="player.current.bitrate" class="tnum">{{ Math.round(player.current.bitrate / 1000) }}kbps</span>
          </div>
        </template>
        <div v-else class="empty-state"><p>未在播放</p></div>
      </div>

      <!-- 书签 -->
      <div v-else class="insp-bookmark">
        <div v-if="!player.currentId" class="empty-state"><p>未在播放</p></div>
        <template v-else>
          <div class="bm-add">
            <input
              v-model="bookmarkLabel"
              placeholder="书签名（可空）"
              @keydown.enter="addBookmark"
            />
            <button class="btn-ghost" @click="addBookmark">打点</button>
          </div>
          <div
            v-for="bm in player.bookmarks"
            :key="bm.id"
            class="bm-row"
          >
            <button class="bm-at tnum" title="跳转" @click="player.seek(bm.atMs / 1000)">{{ fmtTime(bm.atMs / 1000) }}</button>
            <template v-if="renamingId === bm.id">
              <input v-model="renameValue" class="bm-rename" @keydown.enter="player.renameBookmarkById(bm.id, renameValue.trim()); renamingId = null" @blur="player.renameBookmarkById(bm.id, renameValue.trim()); renamingId = null" />
            </template>
            <span v-else class="bm-label" @dblclick="renamingId = bm.id; renameValue = bm.label">{{ bm.label || '（未命名）' }}</span>
            <button class="icon-btn" title="删除书签" @click="player.removeBookmarkById(bm.id)">✕</button>
          </div>
          <div v-if="!player.bookmarks.length" class="bm-empty">当前曲目暂无书签，输入名称后点击「打点」</div>
        </template>
      </div>
    </div>
  </aside>
</template>

<style scoped>
#inspector {
  position: relative;
  display: flex;
  flex-direction: column;
  background: color-mix(in srgb, var(--bg-sidebar), transparent calc((1 - var(--sb-alpha, 1)) * 100%));
  border-left: 1px solid var(--border);
  min-height: 0;
  backdrop-filter: blur(18px);
  flex-shrink: 0;
}
.insp-resizer {
  position: absolute;
  left: -3px;
  top: 0;
  bottom: 0;
  width: 6px;
  cursor: col-resize;
  z-index: 5;
}
.insp-resizer:hover {
  background: rgb(var(--accent-rgb) / 0.35);
}
.insp-tabs {
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 8px 8px 6px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
.insp-tab {
  padding: 5px 12px;
  border-radius: 999px;
  font-size: 12px;
  color: var(--text-muted);
}
.insp-tab.active { background: var(--bg-selected); color: rgb(var(--accent-rgb)); }
.insp-close {
  margin-left: auto;
  width: 24px;
  height: 24px;
  font-size: 11px;
}
.insp-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 10px;
}

.insp-queue-name {
  font-size: 11.5px;
  color: var(--text-muted);
  padding: 4px 6px 10px;
}
.queue-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 8px;
  border-radius: var(--radius-sm);
  cursor: default;
}
.queue-row:hover { background: var(--bg-hover); }
.queue-row.current { background: var(--bg-selected); }
.queue-idx { width: 20px; font-size: 11px; color: var(--text-muted); text-align: right; flex-shrink: 0; }
.queue-meta { flex: 1; min-width: 0; }
.queue-name {
  font-size: 12.5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.queue-row.current .queue-name { color: rgb(var(--accent-rgb)); }
.queue-sub { font-size: 10.5px; color: var(--text-muted); }
.queue-dur { font-size: 10.5px; color: var(--text-muted); flex-shrink: 0; }
.queue-remove { opacity: 0; width: 22px; height: 22px; font-size: 10px; flex-shrink: 0; }
.queue-row:hover .queue-remove { opacity: 1; }

.insp-info {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding-top: 20px;
}
.info-cover {
  width: 120px;
  height: 120px;
  border-radius: var(--radius);
  background: var(--bg-input);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
}
.info-name { font-size: 14px; font-weight: 600; text-align: center; padding: 0 10px; }
.info-artist { font-size: 12px; color: var(--text-sub); }
.info-grid {
  display: grid;
  grid-template-columns: auto 1fr;
  gap: 6px 14px;
  font-size: 12px;
  margin-top: 10px;
  width: 100%;
  padding: 0 14px;
}
.info-k { color: var(--text-muted); }

.bm-add {
  display: flex;
  gap: 6px;
  margin-bottom: 12px;
}
.bm-add input {
  flex: 1;
  min-width: 0;
  padding: 6px 10px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-input);
  outline: none;
  font-size: 12px;
}
.bm-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 6px;
  border-radius: var(--radius-sm);
}
.bm-row:hover { background: var(--bg-hover); }
.bm-at {
  font-size: 11.5px;
  color: rgb(var(--accent-rgb));
  padding: 2px 6px;
  border-radius: 4px;
  background: var(--bg-selected);
  flex-shrink: 0;
}
.bm-label {
  flex: 1;
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: text;
}
.bm-rename {
  flex: 1;
  min-width: 0;
  padding: 3px 6px;
  font-size: 12px;
  border: 1px solid rgb(var(--accent-rgb));
  border-radius: 4px;
  background: var(--bg-input);
  outline: none;
}
.bm-row .icon-btn { opacity: 0; width: 22px; height: 22px; font-size: 10px; }
.bm-row:hover .icon-btn { opacity: 1; }
.bm-empty {
  font-size: 11.5px;
  color: var(--text-muted);
  padding: 10px 6px;
}
</style>
