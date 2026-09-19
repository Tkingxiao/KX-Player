<script setup lang="ts">
/** 收藏夹 / 播放列表详情：头图 + 曲目表，支持移除与右键。 */
import { computed, ref } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import { usePlaylistsStore } from '@/stores/playlists'
import { useTrackActions } from '@/composables/useTrackActions'
import { usePlayerStore } from '@/stores/player'
import TrackTable from '@/features/library/TrackTable.vue'
import { cachedFolderCover, requestFolderCovers, cachedTrackCover } from '@/utils/covers'
import { normDir, trackDir, fmtTime, trackName } from '@/utils/format'
import type { Track } from '@/contracts/api'

const ui = useUiStore()
const library = useLibraryStore()
const playlists = usePlaylistsStore()
const { playTrackInContext, showTrackMenu, confirmRemoveList } = useTrackActions()
const player = usePlayerStore()

const list = computed(() => (ui.listPath ? playlists.listById(ui.listPath) : null))
const isFav = computed(() => !!list.value && (list.value.id.startsWith('fav') || ui.view === 'fav'))

const tracks = computed<Track[]>(() => {
  if (!list.value) return []
  return list.value.trackIds
    .map((id) => library.trackIndex.get(id))
    .filter((t): t is Track => !!t)
})

const renaming = ref(false)
const renameValue = ref('')

function startRename(): void {
  if (!list.value || list.value.isDefault) return
  renaming.value = true
  renameValue.value = list.value.name
}

function commitRename(): void {
  if (list.value && renaming.value) playlists.renameList(list.value.id, renameValue.value)
  renaming.value = false
}

function playAll(): void {
  if (tracks.value.length) playTrackInContext(tracks.value[0], tracks.value.map((t) => t.id), list.value?.name || '')
}

function removeFromList(t: Track): void {
  if (list.value) playlists.removeFromList(list.value.id, [t.id])
}

// 头图：第一个有封面的曲目 → 文件夹封面
const coverTick = ref(0)
const headerCover = computed(() => {
  void coverTick.value
  for (const t of tracks.value) {
    const hit = cachedTrackCover([t.id])
    if (hit) return hit
  }
  const dir = tracks.value.length ? trackDir(tracks.value[0]) : null
  if (dir) {
    const hit = cachedFolderCover(dir)
    if (hit) return hit
    requestFolderCovers([dir], () => { coverTick.value++ })
  }
  return null
})

const totalDuration = computed(() => tracks.value.reduce((s, t) => s + (t.duration || 0), 0))
</script>

<template>
  <div class="list-view">
    <div v-if="list" class="lv-inner">
      <div class="lv-header">
        <div class="lv-cover">
          <img v-if="headerCover" :src="headerCover" alt="" />
          <svg v-else viewBox="0 0 24 24" width="34%" height="34%" fill="none" stroke="currentColor" stroke-width="1.4"><path v-if="!isFav" d="M9 18V5l12-2v13" /><circle v-if="!isFav" cx="6" cy="18" r="3" /><circle v-if="!isFav" cx="18" cy="16" r="3" /><path v-else d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z" /></svg>
        </div>
        <div class="lv-meta">
          <div class="section-title">{{ isFav ? '收藏夹' : '播放列表' }}</div>
          <input
            v-if="renaming"
            v-model="renameValue"
            class="lv-title-input"
            autofocus
            @keydown.enter="commitRename"
            @keydown.esc="renaming = false"
            @blur="commitRename"
          />
          <h1 v-else class="lv-title" :title="list.isDefault ? '默认收藏夹不可重命名' : '双击重命名'" @dblclick="startRename">{{ list.name }}</h1>
          <div class="lv-sub tnum">{{ tracks.length }} 首 · {{ fmtTime(totalDuration) }}</div>
          <div class="lv-actions">
            <button class="btn-primary" :disabled="!tracks.length" @click="playAll">
              <svg viewBox="0 0 24 24" width="12" height="12" fill="currentColor"><polygon points="6,3 21,12 6,21" /></svg>
              播放全部
            </button>
            <button
              v-if="!list.isDefault"
              class="btn-ghost danger-ghost"
              @click="confirmRemoveList(list.id, list.name)"
            >删除列表</button>
          </div>
        </div>
      </div>

      <div v-if="tracks.length" class="lv-table">
        <TrackTable :tracks="tracks" :list-name="list.name" />
      </div>
      <div v-else class="empty-state">
        <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.2"><line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /></svg>
        <p>还没有曲目，右键任意曲目即可加入</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.list-view {
  height: 100%;
  min-height: 0;
  overflow-y: auto;
}
.lv-inner {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  padding: 0 20px 12px;
}
.lv-header {
  display: flex;
  gap: 20px;
  padding: 22px 0 18px;
  flex-shrink: 0;
}
.lv-cover {
  width: 140px;
  height: 140px;
  border-radius: var(--radius-lg);
  overflow: hidden;
  background: var(--bg-input);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  box-shadow: var(--shadow-card);
  flex-shrink: 0;
}
.lv-cover img { width: 100%; height: 100%; object-fit: cover; }
.lv-meta { display: flex; flex-direction: column; justify-content: flex-end; gap: 6px; min-width: 0; }
.lv-title {
  font-size: 26px;
  font-weight: 700;
  letter-spacing: -0.01em;
  cursor: text;
}
.lv-title-input {
  font-size: 22px;
  font-weight: 700;
  background: var(--bg-input);
  border: 1px solid rgb(var(--accent-rgb));
  border-radius: var(--radius-sm);
  padding: 2px 8px;
  outline: none;
  width: 320px;
}
.lv-sub { font-size: 12px; color: var(--text-muted); }
.lv-actions { display: flex; gap: 10px; margin-top: 8px; }
.danger-ghost:hover { color: #e6685f; border-color: rgba(230, 58, 46, 0.5); }
.lv-table {
  flex: 1;
  min-height: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-card);
}
</style>
