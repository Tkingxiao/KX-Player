<script setup lang="ts">
/** 侧边栏：智能集合 / 时长分档 / 文件夹 / 收藏夹 / 播放列表 + 库统计。 */
import { computed, ref } from 'vue'
import { useUiStore, type SmartKey } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import { usePlaylistsStore } from '@/stores/playlists'
import { useSettingsStore } from '@/stores/settings'
import { useTaxonomyStore } from '@/stores/taxonomy'
import { SMART_LABELS, DURATION_BUCKETS } from '@/utils/collections'
import { normDir, fmtFSize } from '@/utils/format'
import { api } from '@/bridge/ipc'
import type { ContextMenuItem } from '@/stores/ui'
import type { Category } from '@/contracts/api'

const ui = useUiStore()
const library = useLibraryStore()
const playlists = usePlaylistsStore()
const settings = useSettingsStore()
const taxonomy = useTaxonomyStore()

const collapsed = ref(false)

// P0-1：拖拽右缘分隔条调宽（180–360），持久化。
// 用 Pointer Events + setPointerCapture：拖出窗口/快速移动也不丢事件。
function startResize(e: PointerEvent): void {
  e.preventDefault()
  const target = e.currentTarget as HTMLElement
  const startX = e.clientX
  const startW = settings.sidebarWidth || 232
  let lastW = startW
  target.setPointerCapture(e.pointerId)
  document.body.classList.add('sidebar-resizing')
  const move = (ev: PointerEvent) => {
    const raw = startW + ev.clientX - startX
    lastW = Math.round(Math.max(180, Math.min(360, raw)))
    settings.sidebarWidth = lastW
  }
  const up = (ev: PointerEvent) => {
    target.removeEventListener('pointermove', move)
    target.removeEventListener('pointerup', up)
    target.removeEventListener('pointercancel', up)
    try { target.releasePointerCapture(ev.pointerId) } catch { /* already released */ }
    document.body.classList.remove('sidebar-resizing')
    settings.sidebarWidth = Math.round(Math.max(180, Math.min(360, lastW)))
    settings.scheduleSave()
  }
  target.addEventListener('pointermove', move)
  target.addEventListener('pointerup', up)
  target.addEventListener('pointercancel', up)
}

const smartKeys: SmartKey[] = ['all', 'unfinished', 'recent', 'recentAdded', 'completed', 'favorites', 'video', 'audio', 'withSubs']

const smartCounts = computed<Record<SmartKey, number>>(() => {
  const tracks = library.allTracks
  const favoriteIds = new Set(playlists.favs.find((f) => f.isDefault)?.trackIds ?? [])
  const counts = {} as Record<SmartKey, number>
  for (const key of smartKeys) counts[key] = 0
  counts.all = tracks.length
  const now = Date.now()
  for (const t of tracks) {
    const p = library.progress.get(t.id)
    const ratio = t.duration > 0 && p ? (p.positionMs / 1000) / t.duration : 0
    const completed = !!p?.completed || ratio >= 0.95
    if (!completed && ratio > 0.01) counts.unfinished++
    if (p && p.playedAt > 0) counts.recent++
    if (p && p.playedAt > 0 && p.playCount > 0) { /* counted in recent */ }
    if (completed) counts.completed++
    if (t.isVideo) counts.video++
    else counts.audio++
    if (t.lyricsPath) counts.withSubs++
    if (favoriteIds.has(t.id)) counts.favorites++
    if (t.fileMtime > 0 && now - t.fileMtime < 30 * 86400_000) counts.recentAdded++
  }
  return counts
})

const folderRoots = computed(() => library.rootFolders)

function goSmart(key: SmartKey): void {
  ui.smartKey = key
  ui.view = 'smart'
  ui.durationKey = null
}

function goDuration(key: string): void {
  ui.durationKey = key
  ui.view = 'smart'
}

function goFolder(path: string): void {
  ui.view = 'folder'
  ui.openFolder(normDir(path))
}

function goList(id: string): void {
  ui.listPath = id
  ui.view = id.startsWith('pl_') ? 'playlist' : 'fav'
}

async function newList(kind: 'fav' | 'pl'): Promise<void> {
  const name = prompt(kind === 'fav' ? '新建收藏夹' : '新建播放列表')
  if (!name) return
  const list = playlists.createList(name, kind)
  if (list) goList(list.id)
}

function folderLabel(path: string): string {
  return path.replace(/\\/g, '/').split('/').filter(Boolean).pop() || path
}

// ── 分类树（P0-11）──────────────────────────────────────────────
function goCategory(id: number | null): void {
  ui.categoryId = id
  ui.view = 'smart'
}

function showCategoryMenu(e: MouseEvent, cat: Category): void {
  e.preventDefault()
  const items: ContextMenuItem[] = [
    { label: '新建子分类', action: () => {
      const name = prompt(`在「${cat.name}」下新建子分类`)
      if (name) void taxonomy.createCategory(cat.id, name)
    } },
    { label: '重命名', action: () => {
      const name = prompt('重命名分类', cat.name)
      if (name) void taxonomy.renameCategory(cat.id, name)
    } },
    { label: '删除', danger: true, action: () => void taxonomy.deleteCategory(cat.id) },
  ]
  ui.ctxMenu = { x: e.clientX, y: e.clientY, items }
}

function createRootCategory(): void {
  const name = prompt('新建分类')
  if (name) void taxonomy.createCategory(null, name)
}

// ── 标签云（P0-12）──────────────────────────────────────────────
function toggleTagFilter(id: number): void {
  ui.toggleTag(id)
}

function toggleTagMode(): void {
  ui.tagMode = ui.tagMode === 'any' ? 'all' : 'any'
}

function showTagMenu(e: MouseEvent, tag: { id: number; name: string }): void {
  e.preventDefault()
  const items: ContextMenuItem[] = [
    { label: '重命名', action: () => {
      const name = prompt('重命名标签', tag.name)
      if (name) void taxonomy.upsertTag(name)
    } },
    { label: '删除标签', danger: true, action: () => void taxonomy.deleteTag(tag.id) },
  ]
  ui.ctxMenu = { x: e.clientX, y: e.clientY, items }
}

function createTag(): void {
  const name = prompt('新建标签')
  if (name) void taxonomy.upsertTag(name)
}

/** 侧边栏文件夹右键菜单：进入 / 打开所在位置 / AI 翻译 / 重新扫描 / 移除 */
function showFolderMenu(e: MouseEvent, path: string): void {
  e.preventDefault()
  const np = normDir(path)
  const isRoot = library.folderPaths.some((p) => normDir(p) === np)
  const items: ContextMenuItem[] = [
    { label: '进入文件夹', action: () => goFolder(np) },
    { label: '在资源管理器中打开', action: () => void api.showItemInFolder(np) },
    {
      label: 'AI 翻译此文件夹…',
      action: () => {
        ui.aiFolder = np
        ui.view = 'ai'
      },
    },
  ]
  if (isRoot) {
    items.push(
      { label: '-', separatorBefore: true },
      { label: '重新扫描此文件夹', action: () => void library.rescan(false, np) },
      { label: '从曲库移除此文件夹', danger: true, action: () => void library.removeFolder(np) },
    )
  }
  ui.ctxMenu = { x: e.clientX, y: e.clientY, items }
}
</script>

<template>
  <aside id="sidebar" :class="{ collapsed }" :style="{ width: (collapsed ? 60 : settings.sidebarWidth) + 'px' }">
    <button class="sb-collapse icon-btn" :title="collapsed ? '展开侧边栏' : '折叠侧边栏'" @click="collapsed = !collapsed">
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2"><polyline :points="collapsed ? '9,18 15,12 9,6' : '15,18 9,12 15,6'" /></svg>
    </button>
    <div v-if="!collapsed" class="sb-resizer" @pointerdown="startResize" />

    <nav class="sb-scroll">
      <!-- 智能集合 -->
      <div class="nav-section">
        <div class="section-title sb-head">音乐库</div>
        <button
          v-for="key in smartKeys"
          :key="key"
          class="nav-item"
          :class="{ active: ui.view === 'smart' && ui.smartKey === key && !ui.durationKey }"
          @click="goSmart(key)"
        >
          <span class="nav-label">{{ SMART_LABELS[key] }}</span>
          <span class="nav-count tnum">{{ smartCounts[key] || '' }}</span>
        </button>
      </div>

      <!-- 时长分档 -->
      <div class="nav-section">
        <div class="section-title sb-head">时长</div>
        <button
          v-for="b in DURATION_BUCKETS"
          :key="b.key"
          class="nav-item nav-sub"
          :class="{ active: ui.view === 'smart' && ui.durationKey === b.key }"
          @click="goDuration(b.key)"
        >
          <span class="nav-label">{{ b.label }}</span>
        </button>
      </div>

      <!-- 分类树（P0-11） -->
      <div class="nav-section">
        <div class="section-title sb-head">
          分类
          <button class="nav-add" title="新建分类" @click="createRootCategory()">＋</button>
        </div>
        <template v-if="taxonomy.categories.length">
          <button
            class="nav-item"
            :class="{ active: ui.view === 'smart' && ui.categoryId === null }"
            @click="goCategory(null)"
          >
            <span class="nav-label">全部分类</span>
          </button>
          <template v-for="cat in taxonomy.categories" :key="cat.id">
            <button
              class="nav-item"
              :class="{ active: ui.categoryId === cat.id }"
              @click="goCategory(cat.id)"
              @contextmenu="showCategoryMenu($event, cat)"
            >
              <span v-if="cat.children.length" class="cat-caret" @click.stop="ui.toggleExpanded(cat.id)">{{ ui.taxonomyExpanded.has(cat.id) ? '▾' : '▸' }}</span>
              <span v-else class="cat-caret cat-leaf" />
              <span class="nav-label">{{ cat.name }}</span>
              <span class="nav-count tnum">{{ cat.trackCount || '' }}</span>
            </button>
            <template v-if="ui.taxonomyExpanded.has(cat.id)">
              <button
                v-for="child in cat.children"
                :key="child.id"
                class="nav-item nav-cat-child"
                :class="{ active: ui.categoryId === child.id }"
                :title="child.name"
                @click="goCategory(child.id)"
                @contextmenu="showCategoryMenu($event, child)"
              >
                <span class="nav-label">{{ child.name }}</span>
                <span class="nav-count tnum">{{ child.trackCount || '' }}</span>
              </button>
            </template>
          </template>
        </template>
        <p v-else class="nav-empty">右键或 ＋ 新建分类</p>
      </div>

      <!-- 标签云（P0-12） -->
      <div class="nav-section">
        <div class="section-title sb-head">
          标签
          <button class="nav-add" title="新建标签" @click="createTag()">＋</button>
        </div>
        <template v-if="taxonomy.tags.length">
          <div class="tag-cloud">
            <button
              v-for="t in taxonomy.tags.slice(0, 24)"
              :key="t.id"
              class="tag-chip"
              :class="{ active: ui.tagIds.has(t.id) }"
              :title="t.name"
              @click="toggleTagFilter(t.id)"
              @contextmenu="showTagMenu($event, t)"
            >
              {{ t.name }}
            </button>
          </div>
          <div class="tag-mode-row">
            <button class="tag-mode" :title="ui.tagMode === 'any' ? '命中任一标签' : '命中全部标签'" @click="toggleTagMode">
              {{ ui.tagMode === 'any' ? '任一' : '全部' }}
            </button>
            <button v-if="ui.tagIds.size" class="tag-mode tag-clear" @click="ui.clearTaxonomy()">清空</button>
          </div>
        </template>
        <p v-else class="nav-empty">右键曲目即可打标签</p>
      </div>

      <!-- 文件夹 -->
      <div v-if="folderRoots.length" class="nav-section">
        <div class="section-title sb-head">文件夹</div>
        <button
          class="nav-item"
          :class="{ active: ui.view === 'folder' && !ui.folderPath }"
          @click="goFolder('')"
        >
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" class="nav-icon"><path d="M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" /><polyline points="9,22 9,12 15,12 15,22" /></svg>
          <span class="nav-label">曲库根目录</span>
          <span class="nav-count tnum">{{ folderRoots.length }}</span>
        </button>
        <button
          v-for="f in folderRoots"
          :key="f.path"
          class="nav-item nav-sub"
          :class="{ active: ui.view === 'folder' && normDir(ui.folderPath || f.path) === normDir(f.path) }"
          :title="f.path"
          @click="goFolder(f.path)"
          @contextmenu="showFolderMenu($event, f.path)"
        >
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" class="nav-icon"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /></svg>
          <span class="nav-label">{{ folderLabel(f.path) }}</span>
          <span class="nav-count tnum">{{ f.trackCount || '' }}</span>
        </button>
      </div>

      <!-- 收藏夹 -->
      <div class="nav-section">
        <div class="section-title sb-head">
          收藏夹
          <button class="nav-add" title="新建收藏夹" @click="newList('fav')">＋</button>
        </div>
        <button
          v-for="f in playlists.favs"
          :key="f.id"
          class="nav-item"
          :class="{ active: ui.view === 'fav' && ui.listPath === f.id }"
          @click="goList(f.id)"
        >
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" class="nav-icon"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z" /></svg>
          <span class="nav-label">{{ f.name }}</span>
          <span class="nav-count tnum">{{ f.trackIds.length || '' }}</span>
        </button>
      </div>

      <!-- 播放列表 -->
      <div class="nav-section">
        <div class="section-title sb-head">
          播放列表
          <button class="nav-add" title="新建播放列表" @click="newList('pl')">＋</button>
        </div>
        <button
          v-for="p in playlists.pls"
          :key="p.id"
          class="nav-item"
          :class="{ active: ui.view === 'playlist' && ui.listPath === p.id }"
          @click="goList(p.id)"
        >
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" class="nav-icon"><line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" /></svg>
          <span class="nav-label">{{ p.name }}</span>
          <span class="nav-count tnum">{{ p.trackIds.length || '' }}</span>
        </button>
      </div>

      <!-- 工具 -->
      <div class="nav-section">
        <div class="section-title sb-head">工具</div>
        <button class="nav-item" :class="{ active: ui.view === 'ai' }" @click="ui.view = 'ai'">
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" class="nav-icon"><path d="M12 3l1.9 5.7L19.6 10l-5.7 1.9L12 17l-1.9-5.1L4.4 10l5.7-1.3z" /><path d="M19 15l.9 2.6L22.5 18l-2.6.9L19 21.5l-.9-2.6L15.5 18l2.6-.4z" /></svg>
          <span class="nav-label">AI 翻译</span>
        </button>
        <button class="nav-item" :class="{ active: ui.view === 'tools' }" @click="ui.view = 'tools'">
          <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.8" class="nav-icon"><circle cx="6.5" cy="6.5" r="2.5" /><circle cx="6.5" cy="17.5" r="2.5" /><line x1="20" y1="4" x2="8.12" y2="15.88" /><line x1="14.47" y1="14.48" x2="20" y2="20" /><line x1="8.12" y1="8.12" x2="12" y2="12" /></svg>
          <span class="nav-label">格式转换</span>
        </button>
      </div>
    </nav>

    <div class="sb-footer" :title="library.libraryStats.totalSize ? '占用 ' + fmtFSize(library.libraryStats.totalSize) : ''">
      <span class="tnum">{{ library.libraryStats.count }}</span> 条目
      <template v-if="library.libraryStats.videos"> · <span class="tnum">{{ library.libraryStats.videos }}</span> 视频</template>
      <template v-if="library.libraryStats.totalSize"> · <span class="tnum">{{ fmtFSize(library.libraryStats.totalSize) }}</span></template>
      <span class="sb-scan-dot" :class="{ busy: library.scanning.active }" :title="library.scanning.active ? '扫描中' : '空闲'" />
    </div>
  </aside>
</template>

<style scoped>
#sidebar {
  display: flex;
  flex-direction: column;
  background: color-mix(in srgb, var(--bg-sidebar), transparent calc((1 - var(--sb-alpha, 1)) * 100%));
  border-right: 1px solid var(--border);
  transition: background var(--dur) var(--ease);
  min-height: 0;
  position: relative;
  z-index: 10;
  backdrop-filter: blur(18px);
  flex-shrink: 0;
}
.sb-resizer {
  position: absolute;
  right: -4px;
  top: 0;
  bottom: 0;
  width: 9px;
  cursor: col-resize;
  z-index: 30;
}
.sb-resizer:hover,
.sb-resizer:active {
  background: rgb(var(--accent-rgb) / 0.35);
}
:global(body.sidebar-resizing) {
  cursor: col-resize !important;
  user-select: none !important;
}
:global(body.sidebar-resizing *) {
  cursor: col-resize !important;
}
#sidebar.collapsed {
  width: var(--sidebar-w-collapsed);
}
#sidebar.collapsed .nav-label,
#sidebar.collapsed .nav-count,
#sidebar.collapsed .sb-head,
#sidebar.collapsed .sb-footer,
#sidebar.collapsed .nav-icon {
  display: none;
}
.sb-collapse {
  position: absolute;
  right: 4px;
  top: 4px;
  z-index: 2;
  width: 24px;
  height: 24px;
  color: var(--text-muted);
}
.sb-scroll {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 10px 8px 8px;
  min-height: 0;
}
.nav-section {
  margin-bottom: 14px;
}
.sb-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 10px 6px;
}
.nav-add {
  font-size: 12px;
  color: var(--text-muted);
  padding: 0 4px;
  border-radius: 4px;
}
.nav-add:hover {
  color: var(--text);
  background: var(--bg-hover);
}
.nav-item {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  padding: 7px 10px;
  border-radius: var(--radius-sm);
  font-size: 12.5px;
  color: var(--text-sub);
  text-align: left;
  transition: background var(--dur-fast) var(--ease), color var(--dur-fast) var(--ease);
}
.nav-item:hover {
  background: var(--bg-hover);
  color: var(--text);
}
.nav-item.active {
  background: var(--bg-selected);
  color: rgb(var(--accent-rgb));
}
.nav-item.active .nav-count {
  color: rgb(var(--accent-rgb));
}
.nav-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.nav-sub {
  padding-left: 22px;
  font-size: 12px;
}
.nav-count {
  font-size: 10.5px;
  color: var(--text-muted);
}
.cat-caret {
  width: 12px;
  flex-shrink: 0;
  font-size: 9px;
  color: var(--text-muted);
  text-align: center;
}
.cat-caret.cat-leaf {
  visibility: hidden;
}
.nav-cat-child {
  padding-left: 26px !important;
  font-size: 12px !important;
}
.nav-empty {
  font-size: 11px;
  color: var(--text-muted);
  padding: 2px 10px 4px;
}
.tag-cloud {
  display: flex;
  flex-wrap: wrap;
  gap: 5px;
  padding: 2px 10px 6px;
}
.tag-chip {
  font-size: 10.5px;
  padding: 2px 8px;
  border-radius: 999px;
  background: var(--bg-input);
  color: var(--text-sub);
  border: 1px solid transparent;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.tag-chip:hover {
  color: var(--text);
  background: var(--bg-hover);
}
.tag-chip.active {
  color: rgb(var(--accent-rgb));
  background: var(--bg-selected);
  border-color: rgb(var(--accent-rgb) / 0.4);
}
.tag-mode-row {
  display: flex;
  gap: 6px;
  padding: 0 10px;
}
.tag-mode {
  font-size: 10px;
  color: var(--text-muted);
  padding: 1px 8px;
  border-radius: 4px;
  background: var(--bg-input);
}
.tag-mode:hover {
  color: var(--text);
}
.nav-icon {
  flex-shrink: 0;
  color: currentColor;
  opacity: 0.75;
}
#sidebar.collapsed .nav-item {
  justify-content: center;
  padding: 8px;
}
.sb-footer {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 10px 14px;
  border-top: 1px solid var(--border);
  font-size: 11px;
  color: var(--text-muted);
  white-space: nowrap;
}
.sb-scan-dot {
  margin-left: auto;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--text-muted);
  opacity: 0.5;
}
.sb-scan-dot.busy {
  background: rgb(var(--accent-rgb));
  opacity: 1;
  animation: pulse 1.2s ease-in-out infinite;
}
@keyframes pulse {
  50% { opacity: 0.35; }
}
</style>
