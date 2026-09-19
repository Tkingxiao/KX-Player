<script setup lang="ts">
/** 播放列表面板（右滑出）：当前队列，点击跳播。 */
import { computed } from 'vue'
import { useUiStore } from '@/stores/ui'
import { usePlayerStore } from '@/stores/player'
import { useLibraryStore } from '@/stores/library'
import { fmtTime, trackName, trackArtist } from '@/utils/format'
import type { Track } from '@/contracts/api'

const ui = useUiStore()
const player = usePlayerStore()
const library = useLibraryStore()

const tracks = computed<Track[]>(() =>
  player.queue.map((id) => library.trackIndex.get(id)).filter((t): t is Track => !!t),
)

function playAt(i: number): void {
  void player.playFromList(player.queue, i, player.queueName, { fromLast: true })
}
</script>

<template>
  <Teleport to="body">
    <Transition name="slide-right">
      <div v-if="ui.queueOpen" class="queue-panel">
        <div class="qp-head">
          <span class="qp-title">{{ player.queueName || '当前播放' }}</span>
          <button class="icon-btn" title="关闭" @click="ui.queueOpen = false">✕</button>
        </div>
        <div class="qp-body">
          <div
            v-for="(t, i) in tracks"
            :key="t.id + i"
            class="qp-row"
            :class="{ current: t.id === player.currentId }"
            @dblclick="playAt(i)"
          >
            <div class="qp-cover">
              <img v-if="t.coverPath" :src="t.coverPath" alt="" loading="lazy" />
              <span v-if="t.id === player.currentId && player.playing" class="wave"><span /><span /><span /></span>
            </div>
            <div class="qp-meta">
              <div class="qp-name">{{ trackName(t) }}</div>
              <div class="qp-sub">{{ trackArtist(t) }}</div>
            </div>
            <span class="qp-dur tnum">{{ fmtTime(t.duration) }}</span>
          </div>
          <div v-if="!tracks.length" class="empty-state"><p>队列为空</p></div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.queue-panel {
  position: fixed;
  top: var(--titlebar-h);
  right: 0;
  bottom: var(--playerbar-h);
  width: 320px;
  z-index: 500;
  display: flex;
  flex-direction: column;
  background: var(--bg-elevated);
  border-left: 1px solid var(--border);
  box-shadow: var(--shadow-pop);
}
.qp-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px 10px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
.qp-title { font-size: 13px; font-weight: 600; }
.qp-body {
  flex: 1;
  overflow-y: auto;
  padding: 8px;
}
.qp-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 7px 8px;
  border-radius: var(--radius-sm);
  cursor: default;
}
.qp-row:hover { background: var(--bg-hover); }
.qp-row.current { background: var(--bg-selected); }
.qp-cover {
  position: relative;
  width: 36px;
  height: 36px;
  border-radius: 5px;
  background: var(--bg-input);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}
.qp-cover img { width: 100%; height: 100%; object-fit: cover; }
.qp-cover .wave { position: absolute; }
.qp-meta { flex: 1; min-width: 0; }
.qp-name {
  font-size: 12.5px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.qp-row.current .qp-name { color: rgb(var(--accent-rgb)); }
.qp-sub {
  font-size: 10.5px;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.qp-dur { font-size: 10.5px; color: var(--text-muted); flex-shrink: 0; }
</style>
