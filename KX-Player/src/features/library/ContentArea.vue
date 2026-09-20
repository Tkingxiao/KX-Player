<script setup lang="ts">
/** 内容区分发：按 ui.view 渲染对应视图 + 顶部筛选 chips。
 *  仅首屏默认视图（SmartView）同步加载，其余视图按需异步拉取，
 *  避免启动时解析整棵视图树（AI 翻译/转换/歌词等体积较大）。 */
import { computed, defineAsyncComponent } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import SmartView from '@/features/library/SmartView.vue'
import { DURATION_BUCKETS, SMART_LABELS, type SmartCollection } from '@/utils/collections'
import { useTaxonomyStore } from '@/stores/taxonomy'

const FolderView = defineAsyncComponent(() => import('@/features/library/FolderView.vue'))
const RecentView = defineAsyncComponent(() => import('@/features/library/RecentView.vue'))
const ListView = defineAsyncComponent(() => import('@/features/playlists/ListView.vue'))
const SearchView = defineAsyncComponent(() => import('@/features/search/SearchView.vue'))
const ConvertView = defineAsyncComponent(() => import('@/features/convert/ConvertView.vue'))
const AiTranslateView = defineAsyncComponent(() => import('@/features/ai/AiTranslateView.vue'))
const LyricsView = defineAsyncComponent(() => import('@/features/lyrics/LyricsView.vue'))

const ui = useUiStore()
const library = useLibraryStore()
const taxonomy = useTaxonomyStore()

const hasTaxonomy = computed(() => ui.categoryId !== null || ui.tagIds.size > 0)
const showChips = computed(() => ui.view === 'smart' && (ui.smartKey !== 'all' || !!ui.durationKey || hasTaxonomy.value))

function clearFilters(): void {
  ui.smartKey = 'all'
  ui.durationKey = null
  ui.clearTaxonomy()
}

const chipLabels = computed(() => {
  const labels: { key: string; label: string }[] = []
  if (ui.smartKey !== 'all') {
    labels.push({ key: 'smart', label: SMART_LABELS[ui.smartKey as SmartCollection] ?? ui.smartKey })
  }
  if (ui.durationKey) {
    labels.push({ key: 'duration', label: DURATION_BUCKETS.find((b) => b.key === ui.durationKey)?.label || '' })
  }
  if (ui.categoryId !== null) {
    const cat = taxonomy.flatCategories.find((c) => c.id === ui.categoryId)
    labels.push({ key: 'category', label: cat?.name ?? '分类' })
  }
  if (ui.tagIds.size) {
    const names = taxonomy.tags.filter((t) => ui.tagIds.has(t.id)).map((t) => `#${t.name}`)
    labels.push({ key: 'tags', label: names.join(ui.tagMode === 'all' ? ' + ' : ' / ') })
  }
  return labels
})
</script>

<template>
  <main id="content">
    <Transition name="fade">
      <div v-if="library.scanning.active" class="scan-banner">
        <span class="scan-spinner" />
        <span>{{ library.scanning.stage }}</span>
        <span v-if="library.scanning.total" class="tnum">{{ library.scanning.completed }}/{{ library.scanning.total }}</span>
      </div>
    </Transition>

    <div v-if="showChips" class="filter-chips">
      <span v-for="c in chipLabels" :key="c.key" class="chip">
        {{ c.label }}
        <span
          class="chip-x"
          @click="c.key === 'smart' ? (ui.smartKey = 'all') : c.key === 'duration' ? (ui.durationKey = null) : c.key === 'category' ? (ui.categoryId = null) : ui.clearTaxonomy()"
        >✕</span>
      </span>
      <button class="chips-clear" @click="clearFilters">清除筛选</button>
    </div>

    <div class="content-area">
      <Transition name="view" mode="out-in">
        <FolderView v-if="ui.view === 'folder'" key="folder" />
        <SmartView v-else-if="ui.view === 'smart'" key="smart" />
        <RecentView v-else-if="ui.view === 'recent'" key="recent" />
        <ListView v-else-if="ui.view === 'fav' || ui.view === 'playlist'" key="list" />
        <SearchView v-else-if="ui.view === 'search'" key="search" />
        <ConvertView v-else-if="ui.view === 'tools'" key="tools" />
        <AiTranslateView v-else-if="ui.view === 'ai'" key="ai" />
        <SmartView v-else key="fallback" />
      </Transition>
    </div>

    <LyricsView v-if="ui.view === 'lyrics'" />
  </main>
</template>

<style scoped>
#content {
  position: relative;
  display: flex;
  flex-direction: column;
  flex: 1 1 auto;
  min-width: 0;
  min-height: 0;
  background: transparent;
}
.scan-banner {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 20px;
  font-size: 11.5px;
  color: var(--text-sub);
  background: var(--bg-selected);
  flex-shrink: 0;
}
.scan-spinner {
  width: 12px;
  height: 12px;
  border-radius: 50%;
  border: 2px solid rgb(var(--accent-rgb));
  border-top-color: transparent;
  animation: spin 0.9s linear infinite;
}
@keyframes spin {
  to { transform: rotate(360deg); }
}
.filter-chips {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 20px 0;
  flex-shrink: 0;
}
.chips-clear {
  font-size: 11.5px;
  color: var(--text-muted);
  text-decoration: underline;
  text-underline-offset: 3px;
}
.chips-clear:hover { color: var(--text); }
.content-area {
  flex: 1;
  min-height: 0;
  position: relative;
}
/* 视图切换：淡入 + 轻微上浮，避免内容瞬切 */
.content-area > * {
  height: 100%;
}
</style>
