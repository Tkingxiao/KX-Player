<script setup lang="ts">
/** 智能集合视图：卡片网格 / 紧凑列表两形态，时长分档过滤，双击播放。 */
import { computed, ref } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import { usePlaylistsStore } from '@/stores/playlists'
import { useSettingsStore } from '@/stores/settings'
import { useTaxonomyStore } from '@/stores/taxonomy'
import { useTrackActions } from '@/composables/useTrackActions'
import { usePlayerStore } from '@/stores/player'
import VirtualGrid from '@/components/VirtualGrid.vue'
import VirtualList from '@/components/VirtualList.vue'
import MediaCover from '@/components/MediaCover.vue'
import TrackTable from '@/features/library/TrackTable.vue'
import { SMART_LABELS, DURATION_BUCKETS, matchSmart, type DurationBucket } from '@/utils/collections'
import { trackName, trackArtist, fmtTime } from '@/utils/format'
import type { Track } from '@/contracts/api'

const ui = useUiStore()
const library = useLibraryStore()
const playlists = usePlaylistsStore()
const settings = useSettingsStore()
const taxonomy = useTaxonomyStore()
const { playTrackInContext, showTrackMenu } = useTrackActions()
const player = usePlayerStore()

const favoriteIds = computed(() => new Set(playlists.favs.find((f) => f.isDefault)?.trackIds ?? []))

// 分类/标签组合筛选（P0-16：与智能集合/时长叠加，「且」关系）
function matchTaxonomy(t: Track): boolean {
  if (ui.categoryId !== null) {
    const ids = taxonomy.tracksByCategory.get(ui.categoryId)
    if (!ids || !ids.has(t.id)) return false
  }
  if (ui.tagIds.size) {
    let anyHit = false
    for (const tagId of ui.tagIds) {
      const hit = taxonomy.tracksByTag.get(tagId)?.has(t.id) ?? false
      if (hit) anyHit = true
      if (ui.tagMode === 'all' && !hit) return false
    }
    if (ui.tagMode === 'any' && !anyHit) return false
  }
  return true
}

const filteredTracks = computed<Track[]>(() => {
  const key = ui.smartKey
  const bucket = ui.durationKey ? DURATION_BUCKETS.find((b) => b.key === ui.durationKey) : null
  let list: Track[] = []
  if (key === 'recent') {
    // 按最近播放排序
    list = library.allTracks
      .filter((t) => (library.progress.get(t.id)?.playedAt ?? 0) > 0)
      .sort((a, b) => (library.progress.get(b.id)?.playedAt ?? 0) - (library.progress.get(a.id)?.playedAt ?? 0))
  } else if (key === 'recentAdded') {
    list = [...library.allTracks].sort((a, b) => (b.fileMtime || 0) - (a.fileMtime || 0))
  } else if (key === 'favorites') {
    list = library.allTracks.filter((t) => favoriteIds.value.has(t.id))
  } else {
    list = library.allTracks.filter((t) => matchSmart(key, t, library.progress.get(t.id), favoriteIds.value))
  }
  if (bucket) {
    list = list.filter((t) => t.duration >= bucket.min && t.duration < bucket.max)
  }
  if (ui.categoryId !== null || ui.tagIds.size) {
    list = list.filter(matchTaxonomy)
  }
  return list
})

const viewMode = computed(() => (key: string) => (key === 'recent' ? 'list' : settings.folderView))

// ── 卡片网格 ──
const gridScroller = ref<InstanceType<typeof VirtualGrid> | null>(null)

function playCard(t: Track): void {
  // P0-2：按介质自动选默认形态——视频双击直接进舞台
  playTrackInContext(t, filteredTracks.value.map((x) => x.id), SMART_LABELS[ui.smartKey])
  if (t.isVideo) ui.view = 'stage'
}

const title = computed(() => {
  if (ui.durationKey) return DURATION_BUCKETS.find((b) => b.key === ui.durationKey)?.label ?? ''
  return SMART_LABELS[ui.smartKey]
})

function rowKey(i: number): string {
  return filteredTracks.value[i]?.id ?? String(i)
}
</script>

<template>
  <div class="smart-view">
    <div class="sv-head">
      <h1 class="sv-title">{{ title }}</h1>
      <span class="sv-count tnum">{{ filteredTracks.length }} 首</span>
      <div class="sv-head-actions">
        <button
          class="icon-btn"
          :title="settings.folderView === 'grid' ? '切换为列表' : '切换为网格'"
          @click="settings.folderView = settings.folderView === 'grid' ? 'list' : 'grid'; settings.scheduleSave()"
        >
          <svg v-if="settings.folderView === 'grid'" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" /></svg>
          <svg v-else viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" /><rect x="14" y="14" width="7" height="7" /><rect x="3" y="14" width="7" height="7" /></svg>
        </button>
      </div>
    </div>

    <!-- 网格形态 -->
    <div v-if="settings.folderView === 'grid' && filteredTracks.length" class="sv-grid">
      <VirtualGrid
        ref="gridScroller"
        :count="filteredTracks.length"
        :card-width="settings.gridSize"
        :card-height="settings.gridSize + 58"
      >
        <template #default="{ index }">
          <div class="media-card" @dblclick="playCard(filteredTracks[index])" @contextmenu="showTrackMenu($event, filteredTracks[index], filteredTracks.map((t) => t.id), title)">
            <div class="media-card-cover" :style="{ height: settings.gridSize + 'px' }">
              <MediaCover :track="filteredTracks[index]" />
              <button class="media-card-play" title="播放" @click.stop="playCard(filteredTracks[index])">
                <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><polygon points="6,3 21,12 6,21" /></svg>
              </button>
            </div>
            <div class="media-card-info">
              <div class="media-card-name" :title="trackName(filteredTracks[index])">{{ trackName(filteredTracks[index]) }}</div>
              <div class="media-card-sub">{{ trackArtist(filteredTracks[index]) }} · {{ fmtTime(filteredTracks[index].duration) }}</div>
            </div>
          </div>
        </template>
      </VirtualGrid>
    </div>

    <!-- 列表形态 -->
    <div v-else-if="filteredTracks.length" class="sv-list">
      <TrackTable :tracks="filteredTracks" :list-name="title" />
    </div>

    <!-- 空态 -->
    <div v-else class="empty-state">
      <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>
      <p>{{ ui.smartKey === 'unfinished' ? '没有未听完的条目' : '这里还是空的' }}</p>
      <button v-if="!library.allTracks.length" class="btn-primary" @click="library.importFolder()">导入文件夹</button>
    </div>
  </div>
</template>

<style scoped>
.smart-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.sv-head {
  display: flex;
  align-items: baseline;
  gap: 12px;
  padding: 18px 20px 10px;
  flex-shrink: 0;
}
.sv-title {
  font-size: 20px;
  font-weight: 650;
  letter-spacing: -0.01em;
}
.sv-count {
  font-size: 12px;
  color: var(--text-muted);
}
.sv-head-actions {
  margin-left: auto;
  display: flex;
  gap: 4px;
  align-self: center;
}

.sv-grid {
  flex: 1;
  min-height: 0;
  padding: 4px 20px 12px;
}
.media-card {
  cursor: pointer;
  border-radius: var(--radius);
  background: var(--bg-card);
  border: 1px solid var(--border);
  overflow: hidden;
  transition: transform var(--dur-fast) var(--ease), box-shadow var(--dur-fast) var(--ease), border-color var(--dur-fast) var(--ease);
}
.media-card:hover {
  transform: translateY(-2px);
  box-shadow: var(--shadow-card);
  border-color: var(--border-strong);
}
.media-card-cover { position: relative; }
.media-card-play {
  position: absolute;
  right: 8px;
  bottom: 8px;
  width: 30px;
  height: 30px;
  border-radius: 50%;
  background: rgb(var(--accent-rgb));
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  opacity: 0;
  transform: translateY(4px);
  transition: opacity var(--dur-fast) var(--ease), transform var(--dur-fast) var(--ease);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);
}
.media-card:hover .media-card-play { opacity: 1; transform: translateY(0); }
.media-card-info { padding: 9px 11px 10px; }
.media-card-name {
  font-size: 12.5px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.media-card-sub {
  font-size: 10.5px;
  color: var(--text-muted);
  margin-top: 2px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.sv-list {
  flex: 1;
  min-height: 0;
  margin: 0 20px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-card);
}
</style>
