<script setup lang="ts">
/** 搜索结果：条目 + 文件夹分区。 */
import { computed } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import { useTrackActions } from '@/composables/useTrackActions'
import TrackTable from '@/features/library/TrackTable.vue'
import { fuzzyMatchScore } from '@/utils/fuzzy'
import { normDir } from '@/utils/format'
import type { Track, FolderNode } from '@/contracts/api'

const ui = useUiStore()
const library = useLibraryStore()
const { playTrackInContext, showTrackMenu } = useTrackActions()

const query = computed(() => ui.searchQuery.trim())

const trackResults = computed<Track[]>(() => {
  if (!query.value) return []
  const q = query.value
  const scored: { t: Track; score: number }[] = []
  for (const t of library.allTracks) {
    let score = fuzzyMatchScore(t.name || '', q) * 1.0
    const artistScore = fuzzyMatchScore(t.metaArtist || t.artist || '', q)
    if (artistScore >= 0) score = Math.max(score, artistScore * 0.95)
    const albumScore = fuzzyMatchScore(t.album || '', q)
    if (albumScore >= 0) score = Math.max(score, albumScore * 0.8)
    if (score > 0) scored.push({ t, score })
  }
  return scored.sort((a, b) => b.score - a.score).slice(0, 400).map((s) => s.t)
})

const folderResults = computed<FolderNode[]>(() => {
  if (!query.value) return []
  const q = query.value
  const out: FolderNode[] = []
  const stack = [...library.folderTree]
  while (stack.length && out.length < 60) {
    const node = stack.pop()!
    stack.push(...node.children)
    if (node.trackCount > 0 && fuzzyMatchScore(node.name, q) >= 0) out.push(node)
  }
  return out
})

function openFolder(node: FolderNode): void {
  ui.view = 'folder'
  ui.openFolder(normDir(node.path))
}

function playFolder(node: FolderNode): void {
  const tracks = library.tracksOfFolder(node.path)
  if (tracks.length) playTrackInContext(tracks[0], tracks.map((t) => t.id), node.name)
}
</script>

<template>
  <div class="search-view">
    <div v-if="!query" class="empty-state">
      <p>输入关键词开始搜索（支持简繁与拼音首字母）</p>
    </div>
    <template v-else>
      <div v-if="folderResults.length" class="sv-section">
        <div class="section-title sv-section-head">文件夹 · {{ folderResults.length }}</div>
        <div class="sv-folders">
          <button v-for="f in folderResults" :key="f.path" class="sv-folder-chip" :title="f.path" @click="openFolder(f)" @dblclick="playFolder(f)">
            <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /></svg>
            {{ f.name }}
          </button>
        </div>
      </div>

      <div v-if="trackResults.length" class="sv-section sv-tracks">
        <div class="section-title sv-section-head">条目 · {{ trackResults.length }}</div>
        <div class="sv-table">
          <TrackTable :tracks="trackResults" :list-name="'搜索：' + query" />
        </div>
      </div>

      <div v-if="!trackResults.length && !folderResults.length" class="empty-state">
        <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.2"><circle cx="11" cy="11" r="8" /><line x1="21" y1="21" x2="16.65" y2="16.65" /></svg>
        <p>没有找到「{{ query }}」的相关内容</p>
      </div>
    </template>
  </div>
</template>

<style scoped>
.search-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  padding: 14px 20px 12px;
}
.sv-section { display: flex; flex-direction: column; min-height: 0; }
.sv-section-head { padding: 6px 2px; }
.sv-folders {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  padding-bottom: 10px;
}
.sv-folder-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 12px;
  border-radius: 999px;
  background: var(--bg-card);
  border: 1px solid var(--border);
  font-size: 12px;
  color: var(--text-sub);
  max-width: 260px;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  transition: border-color var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease);
}
.sv-folder-chip:hover { border-color: rgb(var(--accent-rgb)); color: rgb(var(--accent-rgb)); }
.sv-tracks { flex: 1; min-height: 0; }
.sv-table {
  flex: 1;
  min-height: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-card);
}
</style>
