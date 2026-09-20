<script setup lang="ts">
/**
 * 检查器：队列 / 字幕 / 信息 / 书签 四 Tab。
 * 新增字幕样式面板：映射 mpv sub-* 属性（P0-24/25）。
 */
import { ref, computed, watch, onBeforeUnmount, onMounted } from 'vue'
import { useUiStore } from '@/stores/ui'
import { usePlayerStore } from '@/stores/player'
import { useLibraryStore } from '@/stores/library'
import { useSettingsStore } from '@/stores/settings'
import { fmtTime, trackName, trackArtist } from '@/utils/format'
import type { Track, SubStyle } from '@/contracts/api'

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
    if (resizeMove) window.removeEventListener('mousemove', resizeMove)
    if (resizeUp) window.removeEventListener('mouseup', resizeUp)
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

// 字幕样式
const SUB_POS_PRESETS = [
  { label: '上', value: 20 },
  { label: '中', value: 50 },
  { label: '下', value: 95 },
]

const subStyle = ref<SubStyle>({
  fontSize: 45,
  color: '#ffffff',
  borderColor: '#000000',
  borderSize: 1.5,
  shadowOffset: 1,
  pos: 95,
})

let subDebounce: ReturnType<typeof setTimeout> | null = null
function pushSubStyle(): void {
  if (subDebounce) clearTimeout(subDebounce)
  subDebounce = setTimeout(() => {
    void player.setSubStyle?.(subStyle.value)
  }, 100)
}

function setSubPos(v: number): void {
  subStyle.value.pos = v
  pushSubStyle()
}

function resetSubStyle(): void {
  subStyle.value = {
    fontSize: 45,
    color: '#ffffff',
    borderColor: '#000000',
    borderSize: 1.5,
    shadowOffset: 1,
    pos: 95,
  }
  pushSubStyle()
}

onMounted(() => {
  // 初始同步一次
  pushSubStyle()
})
</script>

<template>
  <aside id="inspector" :style="{ width: settings.inspectorWidth + 'px' }">
    <div class="insp-resizer" @mousedown="startResize" />
    <div class="insp-tabs" role="tablist">
      <button class="insp-tab" role="tab" :class="{ active: ui.inspectorTab === 'queue' }" @click="ui.inspectorTab = 'queue'">队列</button>
      <button class="insp-tab" role="tab" :class="{ active: ui.inspectorTab === 'subtitle' }" @click="ui.inspectorTab = 'subtitle'">字幕</button>
      <button class="insp-tab" role="tab" :class="{ active: ui.inspectorTab === 'info' }" @click="ui.inspectorTab = 'info'">信息</button>
      <button class="insp-tab" role="tab" :class="{ active: ui.inspectorTab === 'bookmark' }" @click="ui.inspectorTab = 'bookmark'">书签</button>
      <button class="icon-btn insp-close" title="关闭" aria-label="关闭检查器" @click="ui.inspectorOpen = false">✕</button>
    </div>

    <div class="insp-body">
      <!-- 队列 -->
      <div v-if="ui.inspectorTab === 'queue'" class="insp-queue">
        <div class="insp-queue-name">{{ player.queueName || '当前队列' }} · {{ queueTracks.length }} 首</div>
        <div v-if="!queueTracks.length" class="empty-state">
          <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="1.2"><line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" /></svg>
          <p>队列为空</p>
          <p class="empty-hint">在曲库中双击条目即可开始播放</p>
        </div>
        <div
          v-for="(t, i) in queueTracks"
          :key="t.id + i"
          class="queue-row"
          :class="{ current: t.id === player.currentId }"
          @dblclick="playAt(i)"
        >
          <span class="queue-idx tnum">{{ i + 1 }}</span>
          <div class="queue-meta">
            <div class="queue-name" :title="trackName(t)">{{ trackName(t) }}</div>
            <div class="queue-sub">{{ trackArtist(t) }}</div>
          </div>
          <span class="queue-dur tnum">{{ fmtTime(t.duration) }}</span>
          <button class="icon-btn queue-remove" title="移出队列" aria-label="移出队列" @click="removeAt(i)">✕</button>
        </div>
      </div>

      <!-- 字幕：轨道选择 + 偏移 + 样式（mpv sub-* 属性遥控） -->
      <div v-else-if="ui.inspectorTab === 'subtitle'" class="insp-subtitle">
        <!-- 轨道选择 -->
        <div class="sub-group">
          <label>轨道</label>
          <div class="sub-track-row">
            <template v-if="player.subtitleTracks.length">
              <button
                v-for="t in player.subtitleTracks"
                :key="t.id"
                class="sub-pos-btn"
                :class="{ active: t.selected }"
                :title="t.title"
                @click="player.selectSubtitle(t.id)"
              >{{ t.id <= 0 ? '关闭' : (t.title || `轨 ${t.id}`) }}</button>
            </template>
            <span v-else class="sub-empty">{{ player.currentId ? '未找到字幕轨' : '未在播放' }}</span>
          </div>
        </div>
        <!-- 显隐 + 偏移 -->
        <div class="sub-group">
          <label>显示</label>
          <div class="sub-track-row">
            <button
              class="sub-pos-btn"
              :class="{ active: player.subtitleVisible }"
              :disabled="!player.subtitleTracks.length"
              :title="player.subtitleTracks.length ? '' : '未找到字幕'"
              @click="player.toggleSubtitleVisible()"
            >{{ player.subtitleVisible ? '已开启' : '已关闭' }}</button>
          </div>
        </div>
        <div class="sub-group">
          <label>偏移</label>
          <div class="sub-delay-row">
            <button class="sub-pos-btn" title="字幕提前 0.1s" @click="player.nudgeSubtitleDelay(-0.1)">−0.1</button>
            <button class="sub-delay-val tnum" title="点击归零" @click="player.nudgeSubtitleDelay(-player.subtitleDelay)">{{ player.subtitleDelay > 0 ? '+' : '' }}{{ player.subtitleDelay.toFixed(1) }}s</button>
            <button class="sub-pos-btn" title="字幕延后 0.1s" @click="player.nudgeSubtitleDelay(0.1)">+0.1</button>
          </div>
        </div>
        <div class="sub-group">
          <label>位置</label>
          <div class="sub-pos-row">
            <button
              v-for="p in SUB_POS_PRESETS"
              :key="p.value"
              class="sub-pos-btn"
              :class="{ active: subStyle.pos === p.value }"
              @click="setSubPos(p.value)"
            >{{ p.label }}</button>
          </div>
        </div>
        <div class="sub-group">
          <label>字号</label>
          <input v-model.number="subStyle.fontSize" type="range" min="12" max="80" step="1" @input="pushSubStyle" />
          <span class="sub-val tnum">{{ subStyle.fontSize }}</span>
        </div>
        <div class="sub-group">
          <label>主色</label>
          <input v-model="subStyle.color" type="color" @input="pushSubStyle" />
          <span class="sub-hex">{{ subStyle.color }}</span>
        </div>
        <div class="sub-group">
          <label>描边色</label>
          <input v-model="subStyle.borderColor" type="color" @input="pushSubStyle" />
          <span class="sub-hex">{{ subStyle.borderColor }}</span>
        </div>
        <div class="sub-group">
          <label>描边宽度</label>
          <input v-model.number="subStyle.borderSize" type="range" min="0" max="4" step="0.1" @input="pushSubStyle" />
          <span class="sub-val tnum">{{ subStyle.borderSize }}</span>
        </div>
        <div class="sub-group">
          <label>阴影偏移</label>
          <input v-model.number="subStyle.shadowOffset" type="range" min="0" max="4" step="0.5" @input="pushSubStyle" />
          <span class="sub-val tnum">{{ subStyle.shadowOffset }}</span>
        </div>
        <div class="sub-actions">
          <button class="btn-ghost" @click="resetSubStyle">重置</button>
        </div>
        <div class="sub-note">
          <p>所有字幕样式由 mpv / libass 渲染，变更即时生效。</p>
          <p>双语字幕：主轨 + 次轨同时加载，次轨字号由 mpv 统一缩放。</p>
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
            <button class="icon-btn" title="删除书签" aria-label="删除书签" @click="player.removeBookmarkById(bm.id)">✕</button>
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
.empty-hint {
  font-size: 11px;
  opacity: 0.75;
}

.insp-subtitle {
  display: flex;
  flex-direction: column;
  gap: 14px;
  padding: 6px 6px 12px;
}
.sub-group {
  display: grid;
  grid-template-columns: 56px 1fr 44px;
  align-items: center;
  gap: 10px;
  font-size: 12px;
}
.sub-group label {
  color: var(--text-sub);
}
.sub-group input[type="range"] {
  width: 100%;
  accent-color: rgb(var(--accent-rgb));
}
.sub-group input[type="color"] {
  width: 100%;
  height: 28px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  background: var(--bg-input);
  cursor: pointer;
}
.sub-val, .sub-hex {
  font-size: 11px;
  color: var(--text-muted);
  text-align: right;
}
.sub-pos-row {
  display: flex;
  gap: 6px;
}
.sub-pos-btn {
  flex: 1;
  padding: 5px 0;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  font-size: 11px;
  color: var(--text-sub);
}
.sub-pos-btn:hover { background: var(--bg-hover); }
.sub-pos-btn.active {
  border-color: rgb(var(--accent-rgb));
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
}
.sub-pos-btn:disabled {
  opacity: 0.45;
  cursor: default;
}
.sub-track-row {
  grid-column: 2 / 4;
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
}
.sub-track-row .sub-pos-btn {
  flex: 0 1 auto;
  padding: 5px 10px;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.sub-delay-row {
  grid-column: 2 / 4;
  display: flex;
  gap: 6px;
}
.sub-delay-row .sub-pos-btn { flex: 1; }
.sub-delay-val {
  flex: 1;
  padding: 5px 0;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  font-size: 11px;
  color: rgb(var(--accent-rgb));
  text-align: center;
}
.sub-delay-val:hover { background: var(--bg-hover); }
.sub-empty {
  font-size: 11px;
  color: var(--text-muted);
  padding: 5px 0;
}
.sub-actions {
  display: flex;
  justify-content: flex-end;
}
.sub-note {
  font-size: 11px;
  color: var(--text-muted);
  line-height: 1.6;
}
</style>