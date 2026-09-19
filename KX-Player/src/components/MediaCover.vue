<script setup lang="ts">
/** 媒体封面：懒加载 file:// 封面 + 进度环 + 视频/时长角标。 */
import { ref, computed, watch } from 'vue'
import type { Track } from '@/contracts/api'
import { resolveTrackCover } from '@/utils/covers'
import { fmtTime, trackDir } from '@/utils/format'
import { useLibraryStore } from '@/stores/library'

const props = withDefaults(defineProps<{
  track: Track
  size?: number
  showProgress?: boolean
}>(), { size: 0, showProgress: true })

const library = useLibraryStore()
const url = ref<string | null>(null)
const failed = ref(false)
let requestId = 0

function load(): void {
  const myId = ++requestId
  failed.value = false
  const hit = resolveTrackCover(props.track, (u) => {
    if (myId === requestId && u) url.value = u
  })
  url.value = hit
}

watch([() => props.track.id, () => library.revision], load, { immediate: true })

// 文件夹封面懒加载完成后可能补上（依赖 progress map 无关；这里靠 resolve 回调）

const progressRatio = computed(() => {
  if (!props.showProgress) return 0
  const p = library.progress.get(props.track.id)
  if (!p || !props.track.duration) return 0
  return Math.min(1, (p.positionMs / 1000) / props.track.duration)
})

const completed = computed(() => {
  const p = library.progress.get(props.track.id)
  return !!p?.completed
})

const durationLabel = computed(() => fmtTime(props.track.duration))
</script>

<template>
  <div class="media-cover" :class="{ 'is-completed': completed }">
    <img v-if="url && !failed" :src="url" alt="" loading="lazy" draggable="false" @error="failed = true" />
    <div v-else class="cover-placeholder">
      <svg v-if="track.isVideo" viewBox="0 0 24 24" width="26%" height="26%" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="2" y="3" width="20" height="18" rx="2" /><polygon points="10,8 16,12 10,16" /></svg>
      <svg v-else viewBox="0 0 24 24" width="26%" height="26%" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>
    </div>
    <div class="cover-badge">
      <span v-if="track.isVideo" class="badge-type">▶</span>
      <span v-else class="badge-type">♪</span>
      <span class="badge-time tnum">{{ durationLabel }}</span>
    </div>
    <div
      v-if="showProgress && progressRatio > 0.02 && !completed"
      class="cover-progress"
      :style="{ width: Math.round(progressRatio * 100) + '%' }"
    />
  </div>
</template>

<style scoped>
.media-cover {
  position: relative;
  width: 100%;
  height: 100%;
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-input);
  color: var(--text-muted);
}
.media-cover img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.cover-placeholder {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, var(--bg-input), var(--bg-hover));
}
.cover-badge {
  position: absolute;
  right: 4px;
  bottom: 4px;
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 2px 6px;
  border-radius: 4px;
  background: rgba(0, 0, 0, 0.62);
  color: #fff;
  font-size: 10px;
  backdrop-filter: blur(4px);
}
.badge-type {
  font-size: 9px;
  opacity: 0.85;
}
.cover-progress {
  position: absolute;
  left: 0;
  bottom: 0;
  height: 2px;
  background: rgb(var(--accent-rgb));
}
.is-completed .cover-badge {
  opacity: 0.55;
}
</style>
