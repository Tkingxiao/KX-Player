<script setup lang="ts">
/** 最近播放：按 playedAt 排序，可清空（确认）。 */
import { computed } from 'vue'
import { useLibraryStore } from '@/stores/library'
import { useSettingsStore } from '@/stores/settings'
import { useTrackActions } from '@/composables/useTrackActions'
import TrackTable from '@/features/library/TrackTable.vue'
import { fmtTime } from '@/utils/format'
import { fmtTime as _f } from '@/utils/format'
import confirmHost from '@/services/confirmHost'
import type { Track } from '@/contracts/api'

const library = useLibraryStore()
const settings = useSettingsStore()
const { playTrackInContext } = useTrackActions()

const tracks = computed<Track[]>(() =>
  settings.recents
    .map((id) => library.trackIndex.get(id))
    .filter((t): t is Track => !!t),
)

const totalDuration = computed(() => tracks.value.reduce((s, t) => s + (t.duration || 0), 0))

async function clearRecents(): Promise<void> {
  const ok = await confirmHost.open({
    title: '清空最近播放',
    message: '确定清空最近播放记录吗？',
    confirmText: '清空',
    danger: true,
  })
  if (ok) {
    settings.recents = []
    settings.scheduleSave()
  }
}
</script>

<template>
  <div class="recent-view">
    <div class="rv-head on-bg">
      <h1 class="rv-title">最近播放</h1>
      <span class="rv-count tnum">{{ tracks.length }} 首 · {{ fmtTime(totalDuration) }}</span>
      <button class="btn-ghost rv-clear" :disabled="!tracks.length" @click="clearRecents">清空</button>
    </div>
    <div v-if="tracks.length" class="rv-table">
      <TrackTable :tracks="tracks" list-name="最近播放" />
    </div>
    <div v-else class="empty-state on-bg">
      <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.2"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 3" /></svg>
      <p>还没有播放记录</p>
    </div>
  </div>
</template>

<style scoped>
.recent-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.rv-head {
  display: flex;
  align-items: baseline;
  gap: 12px;
  padding: 18px 20px 10px;
  flex-shrink: 0;
}
.rv-title { font-size: 20px; font-weight: 650; }
.rv-count { font-size: 12px; color: var(--text-muted); }
.rv-clear { margin-left: auto; align-self: center; }
.rv-table {
  flex: 1;
  min-height: 0;
  margin: 0 20px 12px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-card);
}
</style>
