<script setup lang="ts">
/** 虚拟化音轨表：hover 播放键、播放中波形、表头排序与列宽拖拽、多选。 */
import { ref, computed, onMounted } from 'vue'
import type { Track } from '@/contracts/api'
import VirtualList from '@/components/VirtualList.vue'
import { usePlayerStore } from '@/stores/player'
import { useSettingsStore } from '@/stores/settings'
import { useUiStore } from '@/stores/ui'
import { usePlaylistsStore } from '@/stores/playlists'
import { useTrackActions } from '@/composables/useTrackActions'
import { fmtTime, fmtQuality, trackName, trackArtist } from '@/utils/format'
import { ROW_H } from '@/utils/layout'

const props = withDefaults(defineProps<{
  tracks: Track[]
  listName?: string
  showIndex?: boolean
}>(), { listName: '播放', showIndex: true })

const player = usePlayerStore()
const settings = useSettingsStore()
const ui = useUiStore()
const playlists = usePlaylistsStore()
const { playTrackInContext, showTrackMenu } = useTrackActions()

const ids = computed(() => props.tracks.map((t) => t.id))

// ── 排序 ──
const sortCol = ref<'' | 'name' | 'artist' | 'album' | 'duration'>('')
const sortDir = ref<'asc' | 'desc'>('asc')

function setSort(col: typeof sortCol.value): void {
  if (sortCol.value === col) {
    sortDir.value = sortDir.value === 'asc' ? 'desc' : 'asc'
  } else {
    sortCol.value = col
    sortDir.value = 'asc'
  }
}

const collator = new Intl.Collator('zh-Hans-CN', { numeric: true, sensitivity: 'base' })
const sortedTracks = computed(() => {
  if (!sortCol.value) return props.tracks
  const col = sortCol.value
  const sign = sortDir.value === 'asc' ? 1 : -1
  return [...props.tracks].sort((a, b) => {
    if (col === 'duration') return sign * ((a.duration || 0) - (b.duration || 0))
    return sign * collator.compare(String(a[col] ?? ''), String(b[col] ?? ''))
  })
})

const sortedIds = computed(() => sortedTracks.value.map((t) => t.id))

// ── 列宽 ──
const widths = ref({ artist: 170, album: 170 })

function startResize(col: 'artist' | 'album', e: MouseEvent): void {
  e.preventDefault()
  e.stopPropagation()
  const startX = e.clientX
  const startW = widths.value[col]
  const move = (ev: MouseEvent) => {
    widths.value = { ...widths.value, [col]: Math.max(80, Math.min(400, startW + ev.clientX - startX)) }
  }
  const up = () => {
    window.removeEventListener('mousemove', move)
    window.removeEventListener('mouseup', up)
  }
  window.addEventListener('mousemove', move)
  window.addEventListener('mouseup', up)
}

const gridTemplate = computed(() => {
  const idx = props.showIndex ? '44px ' : ''
  return `${idx}minmax(0, 1fr) ${widths.value.artist}px ${widths.value.album}px 56px 64px 36px`
})

// ── 播放 ──
function isCurrent(t: Track): boolean {
  return player.currentId === t.id
}

function rowClick(e: MouseEvent, t: Track, i: number): void {
  if (ui.selectionMode && (e.ctrlKey || e.metaKey || !e.shiftKey)) {
    ui.toggleSelect(t.id)
    return
  }
  void i
}

function playRow(t: Track): void {
  playTrackInContext(t, sortedIds.value, props.listName)
}

// 预载可见区封面（行内组件自行按需加载，这里无需处理）
onMounted(() => { /* noop */ })

function rowKey(i: number): string {
  return sortedTracks.value[i]?.id ?? String(i)
}
</script>

<template>
  <div class="track-table">
    <div class="tt-header" :style="{ gridTemplateColumns: gridTemplate }">
      <div v-if="showIndex" class="tt-col tt-center">#</div>
      <div class="tt-col tt-sortable" @click="setSort('name')">
        标题
        <span v-if="sortCol === 'name'" class="tt-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
      </div>
      <div class="tt-col tt-sortable" @click="setSort('artist')">
        艺术家
        <span v-if="sortCol === 'artist'" class="tt-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
        <span class="tt-resizer" @mousedown="startResize('artist', $event)" />
      </div>
      <div class="tt-col tt-sortable" @click="setSort('album')">
        专辑
        <span v-if="sortCol === 'album'" class="tt-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
        <span class="tt-resizer" @mousedown="startResize('album', $event)" />
      </div>
      <div class="tt-col tt-sortable tt-center" @click="setSort('duration')">
        时长
        <span v-if="sortCol === 'duration'" class="tt-arrow">{{ sortDir === 'asc' ? '▲' : '▼' }}</span>
      </div>
      <div class="tt-col tt-center">格式</div>
      <div class="tt-col tt-center" />
    </div>

    <div class="tt-body">
      <VirtualList :count="sortedTracks.length" :row-height="ROW_H" :row-key="rowKey">
        <template #default="{ index }">
          <div
            class="tt-row"
            :class="{
              current: isCurrent(sortedTracks[index]),
              selected: ui.selectionMode && ui.selection.has(sortedTracks[index].id),
            }"
            :style="{ gridTemplateColumns: gridTemplate }"
            @dblclick="playRow(sortedTracks[index])"
            @click="rowClick($event, sortedTracks[index], index)"
            @contextmenu="showTrackMenu($event, sortedTracks[index], sortedIds, listName)"
          >
            <div v-if="showIndex" class="tt-cell tt-center tt-index">
              <span v-if="isCurrent(sortedTracks[index]) && player.playing" class="wave"><span /><span /><span /></span>
              <button v-else class="tt-play-btn" title="播放" @click.stop="playRow(sortedTracks[index])">
                <svg viewBox="0 0 24 24" width="12" height="12" fill="currentColor"><polygon points="6,3 21,12 6,21" /></svg>
              </button>
              <span v-if="!isCurrent(sortedTracks[index])" class="tt-num tnum">{{ index + 1 }}</span>
            </div>
            <div class="tt-cell tt-name" :title="sortedTracks[index].path">
              {{ trackName(sortedTracks[index]) }}
              <span v-if="sortedTracks[index].isVideo" class="tt-video">视频</span>
            </div>
            <div class="tt-cell tt-sub" :title="trackArtist(sortedTracks[index])">{{ trackArtist(sortedTracks[index]) }}</div>
            <div class="tt-cell tt-sub" :title="sortedTracks[index].album">{{ sortedTracks[index].album || '—' }}</div>
            <div class="tt-cell tt-center tnum tt-dur">{{ fmtTime(sortedTracks[index].duration) }}</div>
            <div class="tt-cell tt-center">
              <span class="tt-format">{{ fmtQuality(sortedTracks[index]).split(' ')[0] }}</span>
            </div>
            <div class="tt-cell tt-center">
              <button
                class="tt-heart"
                :class="{ on: playlists.isFavorite(sortedTracks[index].id) }"
                title="收藏"
                @click.stop="playlists.toggleFavorite(sortedTracks[index].id)"
              >
                <svg viewBox="0 0 24 24" width="13" height="13" :fill="playlists.isFavorite(sortedTracks[index].id) ? 'currentColor' : 'none'" stroke="currentColor" stroke-width="2"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z" /></svg>
              </button>
            </div>
          </div>
        </template>
      </VirtualList>
    </div>
  </div>
</template>

<style scoped>
.track-table {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.tt-header,
.tt-row {
  display: grid;
  align-items: center;
  gap: 12px;
  padding: 0 12px;
}
.tt-header {
  height: 36px;
  font-size: 11px;
  color: var(--text-muted);
  border-bottom: 1px solid var(--border);
  user-select: none;
  flex-shrink: 0;
}
.tt-body {
  flex: 1;
  min-height: 0;
}
.tt-row {
  height: 100%;
  border-radius: var(--radius-sm);
  cursor: default;
  transition: background var(--dur-fast) var(--ease);
}
.tt-row:hover {
  background: var(--bg-hover);
}
.tt-row.current {
  background: var(--bg-selected);
}
.tt-row.selected {
  background: var(--bg-selected);
  box-shadow: inset 2px 0 0 rgb(var(--accent-rgb));
}
.tt-cell {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12.5px;
  min-width: 0;
}
.tt-sub {
  color: var(--text-sub);
  font-size: 12px;
}
.tt-center { text-align: center; }
.tt-index {
  position: relative;
  color: var(--text-muted);
  font-size: 12px;
}
.tt-num { display: inline; }
.tt-row:hover .tt-num { display: none; }
.tt-row:hover .tt-play-btn { display: inline-flex; }
.tt-play-btn {
  display: none;
  align-items: center;
  justify-content: center;
  color: var(--text);
  width: 22px;
  height: 22px;
  border-radius: 50%;
}
.tt-play-btn:hover { color: rgb(var(--accent-rgb)); }
.tt-row.current .tt-num, .tt-row.current .tt-play-btn { display: none; }
.tt-name {
  font-weight: 500;
  display: flex;
  align-items: center;
  gap: 6px;
}
.tt-row.current .tt-name { color: rgb(var(--accent-rgb)); }
.tt-video {
  flex-shrink: 0;
  font-size: 9.5px;
  padding: 0 5px;
  border-radius: 3px;
  background: var(--bg-input);
  color: var(--text-muted);
}
.tt-dur { color: var(--text-muted); font-size: 11.5px; }
.tt-format {
  font-size: 9.5px;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--bg-input);
  color: var(--text-muted);
  text-transform: uppercase;
}
.tt-heart {
  display: inline-flex;
  color: var(--text-muted);
  opacity: 0;
  transition: opacity var(--dur-fast) var(--ease);
}
.tt-row:hover .tt-heart { opacity: 1; }
.tt-heart.on { opacity: 1; color: var(--danger); }
.tt-sortable { cursor: pointer; }
.tt-sortable:hover { color: var(--text); }
.tt-arrow { font-size: 8px; margin-left: 3px; }
.tt-col { position: relative; }
.tt-resizer {
  position: absolute;
  right: -7px;
  top: 0;
  width: 8px;
  height: 100%;
  cursor: col-resize;
  z-index: 1;
}
</style>
