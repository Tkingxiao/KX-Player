<script setup lang="ts">
/** 全屏歌词页：封面 + 元信息 + 歌词（点击 seek / 自动居中 / 手动滚动暂停）。 */
import { ref, computed, watch, nextTick } from 'vue'
import { useUiStore } from '@/stores/ui'
import { usePlayerStore } from '@/stores/player'
import { useLibraryStore } from '@/stores/library'
import { usePlaylistsStore } from '@/stores/playlists'
import { useTrackActions } from '@/composables/useTrackActions'
import { resolveTrackCover } from '@/utils/covers'
import { activeLyricIndex } from '@/utils/parsers/lyrics'
import { fmtTime, fmtFSize, fmtQuality, trackDir } from '@/utils/format'
import { api } from '@/bridge/ipc'

const ui = useUiStore()
const player = usePlayerStore()
const library = useLibraryStore()
const playlists = usePlaylistsStore()
const { copyText } = useTrackActions()

const coverUrl = ref<string | null>(null)
let coverReqId = 0

watch([() => player.currentId, () => library.revision], () => {
  const t = player.current
  if (!t) { coverUrl.value = null; return }
  const myId = ++coverReqId
  const hit = resolveTrackCover(t, (u) => { if (myId === coverReqId && u) coverUrl.value = u })
  coverUrl.value = hit
}, { immediate: true })

// 高亮行
const activeIdx = computed(() => activeLyricIndex(player.lyrics, player.position))

// 自动滚动 + 手动滚动暂停
const lyricsEl = ref<HTMLElement | null>(null)
const autoScroll = ref(true)
let resumeTimer: ReturnType<typeof setTimeout> | null = null
let lastIdx = -2

watch(activeIdx, async (idx) => {
  if (idx === lastIdx) return
  lastIdx = idx
  if (!autoScroll.value || idx < 0) return
  await nextTick()
  const el = lyricsEl.value?.querySelector('.lyric-line.active') as HTMLElement | null
  if (el && lyricsEl.value) {
    lyricsEl.value.scrollTo({
      top: el.offsetTop - lyricsEl.value.clientHeight / 2 + el.clientHeight / 2,
      behavior: 'smooth',
    })
  }
})

function onUserScroll(): void {
  autoScroll.value = false
  if (resumeTimer) clearTimeout(resumeTimer)
  resumeTimer = setTimeout(() => { autoScroll.value = true }, 2200)
}

function seekTo(time: number): void {
  player.seek(time)
  autoScroll.value = true
}

function goBack(): void {
  ui.goBack()
}

const tab = ref<'lyrics' | 'info'>('lyrics')

const infoRows = computed(() => {
  const t = player.current
  if (!t) return []
  return [
    ['文件名', t.name || ''],
    ['艺术家', t.metaArtist || t.artist || '佚名'],
    ['专辑', t.album || '—'],
    ['格式', fmtQuality(t)],
    ['时长', fmtTime(t.duration)],
    ['大小', fmtFSize(t.fileSize)],
    ['路径', t.path],
  ] as [string, string][]
})

const isFavorite = computed(() => (player.currentId ? playlists.isFavorite(player.currentId) : false))
</script>

<template>
  <div class="lyrics-view">
    <button class="icon-btn lv-back" title="返回（Esc）" @click="goBack">
      <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2"><polyline points="15,18 9,12 15,6" /></svg>
    </button>

    <div class="lv-main">
      <!-- 封面侧 -->
      <div class="lv-cover-side">
        <div class="lv-cover">
          <img v-if="coverUrl" :src="coverUrl" alt="" />
          <svg v-else viewBox="0 0 24 24" width="26%" height="26%" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>
        </div>
        <h1 class="lv-title" :title="player.currentTitle">{{ player.currentTitle }}</h1>
        <div class="lv-artist">{{ player.currentArtist }}</div>
        <div class="lv-badges">
          <span v-if="player.current" class="chip">{{ player.current.isVideo ? '视频 · 仅音频' : '音频' }}</span>
          <span v-if="player.lyricsInfo.format" class="chip">{{ player.lyricsInfo.format.toUpperCase() }}</span>
          <button
            class="chip chip-btn"
            :class="{ on: isFavorite }"
            @click="player.currentId && playlists.toggleFavorite(player.currentId)"
          >{{ isFavorite ? '♥ 已收藏' : '♡ 收藏' }}</button>
        </div>
        <div class="lv-progress tnum">
          {{ fmtTime(player.position) }} / {{ fmtTime(player.duration) }}
        </div>
      </div>

      <!-- 歌词 / 信息侧 -->
      <div class="lv-content">
        <div class="lv-tabs">
          <button class="lv-tab" :class="{ active: tab === 'lyrics' }" @click="tab = 'lyrics'">歌词</button>
          <button class="lv-tab" :class="{ active: tab === 'info' }" @click="tab = 'info'">信息</button>
        </div>

        <div
          v-show="tab === 'lyrics'"
          ref="lyricsEl"
          class="lv-lyrics"
          @wheel.passive="onUserScroll"
        >
          <template v-if="player.lyrics.length">
            <div
              v-for="(line, i) in player.lyrics"
              :key="i"
              class="lyric-line"
              :class="{ active: i === activeIdx }"
              @click="seekTo(line.time)"
            >{{ line.text }}</div>
            <div class="lv-lyrics-pad" />
          </template>
          <div v-else class="empty-state">
            <svg viewBox="0 0 24 24" width="36" height="36" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" /></svg>
            <p>未找到匹配的歌词文件（.lrc / .vtt / .srt）</p>
          </div>
        </div>

        <div v-show="tab === 'info'" class="lv-info">
          <div v-for="[k, v] in infoRows" :key="k" class="lv-info-row">
            <span class="lv-info-key">{{ k }}</span>
            <span class="lv-info-val" :class="{ break: k === '路径' }">{{ v }}</span>
            <button
              v-if="k === '路径'"
              class="icon-btn"
              title="复制路径"
              @click="copyText(player.current?.path || '', '路径')"
            ><svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2"><rect x="9" y="9" width="13" height="13" rx="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></svg></button>
          </div>
          <div class="lv-info-actions">
            <button class="btn-ghost" @click="player.current && copyText(trackDir(player.current) || '', '文件夹路径')">复制所在文件夹</button>
            <button class="btn-ghost" @click="player.current && api.showItemInFolder(player.current.path)">在资源管理器中显示</button>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.lyrics-view {
  position: absolute;
  inset: 0;
  z-index: 15;
  background: var(--bg);
  display: flex;
  flex-direction: column;
}
.lv-back {
  position: absolute;
  top: 12px;
  left: 16px;
  z-index: 2;
}
.lv-main {
  flex: 1;
  display: grid;
  grid-template-columns: minmax(280px, 2fr) 3fr;
  gap: 30px;
  padding: 50px 44px 24px;
  min-height: 0;
  max-width: 1280px;
  margin: 0 auto;
  width: 100%;
}
.lv-cover-side {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 14px;
  min-width: 0;
}
.lv-cover {
  width: min(300px, 32vh);
  aspect-ratio: 1;
  border-radius: var(--radius-lg);
  overflow: hidden;
  background: var(--bg-input);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  box-shadow: 0 18px 60px rgba(0, 0, 0, 0.45);
}
.lv-cover img { width: 100%; height: 100%; object-fit: cover; }
.lv-title {
  font-size: 19px;
  font-weight: 650;
  text-align: center;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.lv-artist {
  font-size: 13px;
  color: var(--text-sub);
}
.lv-badges { display: flex; gap: 8px; flex-wrap: wrap; justify-content: center; }
.chip-btn { cursor: pointer; }
.chip-btn.on { color: #e6685f; border-color: rgba(230, 58, 46, 0.4); }
.lv-progress {
  font-size: 12px;
  color: var(--text-muted);
}

.lv-content {
  display: flex;
  flex-direction: column;
  min-height: 0;
  min-width: 0;
}
.lv-tabs {
  display: flex;
  gap: 4px;
  margin-bottom: 10px;
  flex-shrink: 0;
}
.lv-tab {
  padding: 5px 14px;
  border-radius: 999px;
  font-size: 12.5px;
  color: var(--text-muted);
}
.lv-tab.active {
  background: var(--bg-selected);
  color: rgb(var(--accent-rgb));
}
.lv-lyrics {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 30px 8px;
  mask-image: linear-gradient(transparent, black 12%, black 88%, transparent);
}
.lyric-line {
  padding: 9px 4px;
  font-size: 15px;
  color: var(--text-muted);
  cursor: pointer;
  transition: color var(--dur) var(--ease), transform var(--dur) var(--ease);
  transform-origin: left center;
}
.lyric-line:hover { color: var(--text-sub); }
.lyric-line.active {
  color: rgb(var(--accent-rgb));
  font-size: 18px;
  font-weight: 600;
  transform: scale(1.01);
}
.lv-lyrics-pad { height: 30%; }

.lv-info {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 8px 4px;
}
.lv-info-row {
  display: flex;
  align-items: baseline;
  gap: 12px;
  padding: 9px 4px;
  border-bottom: 1px solid var(--border);
  font-size: 12.5px;
}
.lv-info-key {
  width: 56px;
  flex-shrink: 0;
  color: var(--text-muted);
}
.lv-info-val {
  flex: 1;
  color: var(--text);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.lv-info-val.break { white-space: normal; word-break: break-all; }
.lv-info-actions {
  display: flex;
  gap: 10px;
  margin-top: 16px;
}
</style>
