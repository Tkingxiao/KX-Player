<script setup lang="ts">
/** 文件夹视图：面包屑 + 子文件夹（网格/列表）+ 音轨表，逐级滚动记忆（同源码展示逻辑）。 */
import { computed, ref, watch, onMounted, nextTick } from 'vue'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import { useSettingsStore } from '@/stores/settings'
import { useTrackActions } from '@/composables/useTrackActions'
import VirtualGrid from '@/components/VirtualGrid.vue'
import VirtualList from '@/components/VirtualList.vue'
import SelectMenu from '@/components/SelectMenu.vue'
import TrackTable from '@/features/library/TrackTable.vue'
import { normDir, fmtTime } from '@/utils/format'
import { cachedFolderCover, requestFolderCovers } from '@/utils/covers'
import { cardRowHeight } from '@/utils/layout'
import type { ContextMenuItem } from '@/stores/ui'
import type { FolderNode } from '@/contracts/api'

const ui = useUiStore()
const library = useLibraryStore()
const settings = useSettingsStore()
const { playTrackInContext, showTrackMenu } = useTrackActions()

const isRoot = computed(() => !ui.folderPath)

// 根级视图：汇总所有 rootFolders；子级视图：显示当前节点的子文件夹
const currentNode = computed<FolderNode | null>(() =>
  isRoot.value ? null : library.nodeByPath(ui.folderPath),
)

const visibleChildren = computed<FolderNode[]>(() => {
  if (isRoot.value) return library.rootFolders
  return (currentNode.value?.children || []).filter((c) => c.trackCount > 0)
})

const directTracks = computed(() => currentNode.value?.tracks || [])
const allFolderTracks = computed(() => {
  if (isRoot.value) return library.allTracks
  return currentNode.value ? library.tracksOfFolder(currentNode.value.path) : []
})

// ── 面包屑：从扫描根目录开始显示（曲库根目录/RJ/作品…），不暴露完整物理路径 ──
const breadcrumbs = computed(() => {
  const path = ui.folderPath
  if (!path) return []
  const np = normDir(path)
  // 找到最长匹配的扫描根目录
  let root: string | null = null
  for (const rp of library.folderPaths) {
    const nr = normDir(rp)
    if ((np === nr || np.startsWith(nr + '/')) && (!root || nr.length > normDir(root).length)) {
      root = nr
    }
  }
  if (!root) return []
  const rootNorm = normDir(root)
  const sep = path.includes('\\') ? '\\' : '/'
  const crumbs: { name: string; path: string }[] = [
    { name: rootNorm.split('/').filter(Boolean).pop() || rootNorm, path: rootNorm },
  ]
  if (np !== rootNorm) {
    let acc = rootNorm
    for (const seg of np.slice(rootNorm.length + 1).split('/')) {
      acc = acc + '/' + seg
      crumbs.push({ name: seg, path: acc })
    }
  }
  return crumbs.slice(-8)
})

function navigate(path: string): void {
  ui.openFolder(normDir(path))
}

function goBack(): void {
  const path = ui.folderPath
  if (!path) return
  const norm = normDir(path)
  for (const root of library.folderPaths) {
    const nr = normDir(root)
    if (norm === nr) { ui.openFolder(''); return }
    if (norm.startsWith(nr + '/')) {
      const rest = norm.slice(nr.length + 1)
      const segs = rest.split('/')
      segs.pop()
      ui.openFolder(nr + (segs.length ? '/' + segs.join('/') : ''))
      return
    }
  }
  ui.openFolder('')
}

// ── 排序 ──
const collator = new Intl.Collator('zh-Hans-CN', { numeric: true, sensitivity: 'base' })
const sortLabels = { name: '名称', mtime: '修改时间', count: '曲目数' } as Record<string, string>

const sortedChildren = computed(() => {
  const list = [...visibleChildren.value]
  const dir = settings.folderSortDir === 'asc' ? 1 : -1
  list.sort((a, b) => {
    if (settings.folderSort === 'mtime') return dir * ((a.tracks[0]?.fileMtime || 0) - (b.tracks[0]?.fileMtime || 0))
    if (settings.folderSort === 'count') return dir * (a.trackCount - b.trackCount)
    return dir * collator.compare(a.name, b.name)
  })
  return list
})

function cycleSort(): void {
  const order = ['name', 'mtime', 'count'] as const
  const i = order.indexOf(settings.folderSort)
  const next = order[(i + 1) % order.length]
  settings.folderSort = next
  if (i === order.length - 1) settings.folderSortDir = settings.folderSortDir === 'asc' ? 'desc' : 'asc'
  settings.scheduleSave()
}

// ── 文件夹卡片封面 ──
// 一次性把当前可见子目录批量入队（后端合批处理），避免每张卡片各发一次 IPC。
// coverTick 必须在渲染期被读取，否则封面到达后组件不会重新渲染（图片永远不出现）。
const coverTick = ref(0)
function folderCover(node: FolderNode): string | null {
  void coverTick.value // 建立响应式依赖：封面返回后触发重渲染
  const hit = cachedFolderCover(normDir(node.path))
  if (hit) return hit
  return null
}

watch(
  () => sortedChildren.value.map((c) => normDir(c.path)).join('|'),
  () => {
    const paths = sortedChildren.value.map((c) => normDir(c.path))
    if (!paths.length) return
    requestFolderCovers(paths, () => { coverTick.value++ })
  },
  { immediate: true },
)

// ── 播放 ──
function playFolder(node: FolderNode): void {
  const tracks = library.tracksOfFolder(node.path)
  if (tracks.length) playTrackInContext(tracks[0], tracks.map((t) => t.id), node.name)
}

function playAllHere(): void {
  if (allFolderTracks.value.length) {
    playTrackInContext(allFolderTracks.value[0], allFolderTracks.value.map((t) => t.id), currentNode.value?.name || '文件夹')
  }
}

// ── 文件夹右键菜单 ──
function showFolderMenu(e: MouseEvent, node: FolderNode): void {
  e.preventDefault()
  const isRootFolder = library.folderPaths.some((p) => normDir(p) === normDir(node.path))
  const items: ContextMenuItem[] = [
    { label: '进入文件夹', action: () => navigate(node.path) },
    {
      label: 'AI 翻译此文件夹…',
      action: () => {
        ui.aiFolder = normDir(node.path)
        ui.view = 'ai'
      },
    },
  ]
  if (isRootFolder) {
    items.push(
      { label: '-', separatorBefore: true },
      { label: '重新扫描此文件夹', action: () => void library.rescan(false, normDir(node.path)) },
      { label: '从曲库移除此文件夹', danger: true, action: () => void library.removeFolder(normDir(node.path)) },
    )
  }
  ui.ctxMenu = { x: e.clientX, y: e.clientY, items }
}

// ── 滚动记忆 ──
const gridScroller = ref<InstanceType<typeof VirtualGrid> | null>(null)
const listScroller = ref<InstanceType<typeof VirtualList> | null>(null)
const trackTableWrap = ref<HTMLElement | null>(null)

function restoreScroll(): void {
  const saved = ui.savedScroll(ui.folderPath || '__root__')
  nextTick(() => {
    gridScroller.value?.scrollToTop(saved)
    listScroller.value?.scrollToTop(saved)
  })
}

watch(() => ui.folderPath, () => restoreScroll())
onMounted(() => restoreScroll())

function onScroll(top: number): void {
  const key = ui.folderPath || '__root__'
  if (top > 0 || ui.savedScroll(key) > 0) ui.recordScroll(key, top)
}
</script>

<template>
  <div class="folder-view">
    <!-- 工具行 -->
    <div class="fv-toolbar">
      <button v-if="!isRoot" class="icon-btn" title="返回上一级" @click="goBack">
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><polyline points="15,18 9,12 15,6" /></svg>
      </button>
      <nav class="fv-crumbs">
        <button class="fv-crumb" @click="navigate('')">曲库根目录</button>
        <template v-for="(c, i) in breadcrumbs" :key="c.path">
          <span class="fv-crumb-sep">/</span>
          <button
            class="fv-crumb"
            :class="{ last: i === breadcrumbs.length - 1 }"
            :title="c.path"
            @click="navigate(c.path)"
          >{{ c.name }}</button>
        </template>
      </nav>
      <div class="fv-actions">
        <button v-if="allFolderTracks.length" class="btn-primary fv-playall" @click="playAllHere">
          <svg viewBox="0 0 24 24" width="12" height="12" fill="currentColor"><polygon points="6,3 21,12 6,21" /></svg>
          播放全部
        </button>
        <!-- 排序：选择栏形式（默认名称 A-Z） -->
        <SelectMenu
          :model-value="settings.folderSort"
          variant="text"
          title="文件夹排序字段"
          :options="[{ value: 'name', label: '名称' }, { value: 'mtime', label: '修改时间' }, { value: 'count', label: '曲目数' }]"
          @update:model-value="settings.folderSort = $event as 'name' | 'mtime' | 'count'; settings.scheduleSave()"
        />
        <SelectMenu
          :model-value="settings.folderSortDir"
          variant="icon"
          title="排序方向"
          :options="[{ value: 'asc', label: '升序（A→Z）' }, { value: 'desc', label: '降序（Z→A）' }]"
          @update:model-value="settings.folderSortDir = $event as 'asc' | 'desc'; settings.scheduleSave()"
        >
          <template #icon>
            <svg v-if="settings.folderSortDir === 'asc'" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><line x1="12" y1="5" x2="20" y2="5" /><line x1="12" y1="10" x2="17" y2="10" /><line x1="12" y1="15" x2="14" y2="15" /><path d="M7 4v14M7 18l-3-3M7 18l3-3" /></svg>
            <svg v-else viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><line x1="12" y1="5" x2="14" y2="5" /><line x1="12" y1="10" x2="17" y2="10" /><line x1="12" y1="15" x2="20" y2="15" /><path d="M7 20V6M7 6l-3 3M7 6l3 3" /></svg>
          </template>
        </SelectMenu>
        <!-- 视图形态：选择栏形式 -->
        <SelectMenu
          :model-value="settings.folderView"
          variant="icon"
          title="视图形态"
          :options="[{ value: 'grid', label: '卡片网格' }, { value: 'list', label: '紧凑列表' }]"
          @update:model-value="settings.folderView = $event as 'grid' | 'list'; settings.scheduleSave()"
        >
          <template #icon>
            <svg v-if="settings.folderView === 'grid'" viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" /><rect x="14" y="14" width="7" height="7" /><rect x="3" y="14" width="7" height="7" /></svg>
            <svg v-else viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2"><line x1="8" y1="6" x2="21" y2="6" /><line x1="8" y1="12" x2="21" y2="12" /><line x1="8" y1="18" x2="21" y2="18" /><line x1="3" y1="6" x2="3.01" y2="6" /><line x1="3" y1="12" x2="3.01" y2="12" /><line x1="3" y1="18" x2="3.01" y2="18" /></svg>
          </template>
        </SelectMenu>
      </div>
    </div>

    <!-- 子文件夹网格（底部渐隐，缓和被容器裁切的边缘） -->
    <div v-if="settings.folderView === 'grid' && sortedChildren.length" class="fv-grid-wrap">
      <VirtualGrid
        ref="gridScroller"
        class="fv-scroll edge-fade"
        :count="sortedChildren.length"
        :card-width="settings.gridSize"
        :card-height="cardRowHeight(settings.gridSize)"
        @scroll="onScroll"
      >
        <template #default="{ index }">
          <div
            class="folder-card"
            @dblclick.stop="playFolder(sortedChildren[index])"
            @click="navigate(sortedChildren[index].path)"
            @contextmenu="showFolderMenu($event, sortedChildren[index])"
          >
            <div class="folder-card-cover" :style="{ height: settings.gridSize + 'px' }">
              <img v-if="folderCover(sortedChildren[index])" :src="folderCover(sortedChildren[index]) || ''" alt="" loading="lazy" draggable="false" />
              <svg v-else viewBox="0 0 24 24" width="30%" height="30%" fill="none" stroke="currentColor" stroke-width="1.4"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /></svg>
              <button class="folder-card-play" title="播放" @click.stop="playFolder(sortedChildren[index])">
                <svg viewBox="0 0 24 24" width="14" height="14" fill="currentColor"><polygon points="6,3 21,12 6,21" /></svg>
              </button>
            </div>
            <div class="folder-card-info">
              <div class="folder-card-name" :title="sortedChildren[index].name">{{ sortedChildren[index].name }}</div>
              <div class="folder-card-sub tnum">{{ sortedChildren[index].trackCount }} 首</div>
            </div>
          </div>
        </template>
      </VirtualGrid>
    </div>

    <!-- 子文件夹列表 -->
    <div v-else-if="sortedChildren.length" class="fv-list-wrap">
      <VirtualList
        ref="listScroller"
        class="fv-scroll edge-fade"
        :count="sortedChildren.length"
        :row-height="64"
        :row-key="(i: number) => sortedChildren[i]?.path ?? i"
        @scroll="onScroll"
      >
        <template #default="{ index }">
          <div class="folder-row" @dblclick="playFolder(sortedChildren[index])" @click="navigate(sortedChildren[index].path)" @contextmenu="showFolderMenu($event, sortedChildren[index])">
            <div class="folder-row-cover">
              <img v-if="folderCover(sortedChildren[index])" :src="folderCover(sortedChildren[index]) || ''" alt="" loading="lazy" draggable="false" />
              <svg v-else viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="1.6"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /></svg>
            </div>
            <div class="folder-row-name">{{ sortedChildren[index].name }}</div>
            <div class="folder-row-sub tnum">{{ sortedChildren[index].trackCount }} 首</div>
          </div>
        </template>
      </VirtualList>
    </div>

    <!-- 当前目录直接音轨 -->
    <div v-if="directTracks.length" class="fv-tracks" :class="{ 'has-children': sortedChildren.length }">
      <div class="fv-tracks-head section-title">本目录曲目 · {{ directTracks.length }}</div>
      <div ref="trackTableWrap" class="fv-tracks-table">
        <TrackTable :tracks="directTracks" :list-name="currentNode?.name || '文件夹'" />
      </div>
    </div>

    <!-- 空态 -->
    <div v-if="!sortedChildren.length && !directTracks.length" class="empty-state">
      <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" /></svg>
      <p>此文件夹内没有音频</p>
    </div>

    <div v-if="!isRoot && allFolderTracks.length" class="fv-total tnum">
      共 {{ allFolderTracks.length }} 首 · {{ fmtTime(allFolderTracks.reduce((s, t) => s + (t.duration || 0), 0)) }}
    </div>
  </div>
</template>

<style scoped>
.folder-view {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.fv-toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 10px 18px 8px;
  flex-shrink: 0;
}
.fv-crumbs {
  display: flex;
  align-items: center;
  gap: 2px;
  flex: 1;
  overflow: hidden;
  white-space: nowrap;
}
.fv-crumb {
  font-size: 12.5px;
  color: var(--text-muted);
  padding: 3px 6px;
  border-radius: 4px;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 220px;
}
.fv-crumb:hover { color: var(--text); background: var(--bg-hover); }
.fv-crumb.last { color: var(--text); font-weight: 500; }
.fv-crumb-sep { color: var(--text-muted); opacity: 0.5; }
.fv-actions { display: flex; align-items: center; gap: 6px; }
.fv-playall { padding: 6px 14px; font-size: 12px; }

.fv-grid-wrap {
  flex: 1;
  min-height: 0;
  padding: 4px 18px;
}
/* 滚动区顶部/底部渐隐：避免内容被容器生硬截断 */
.fv-scroll {
  --fade-top: 6px;
  --fade-bottom: 22px;
}
.folder-card {
  cursor: pointer;
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-card);
  border: 1px solid var(--border);
  /* 固定卡片尺寸：网格列宽由 vgrid 决定，卡片自身必须撑满且不因内容（有无封面）改变大小，
     否则无图文件夹会显示为异常宽/窄的卡片 */
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  transition: transform var(--dur-fast) var(--ease), box-shadow var(--dur-fast) var(--ease), border-color var(--dur-fast) var(--ease);
}
.folder-card:hover {
  transform: translateY(-2px);
  box-shadow: var(--shadow-card);
  border-color: var(--border-strong);
}
.folder-card-cover {
  position: relative;
  /* 高度由行内 style 设定（= gridSize）；宽度始终占满 */
  width: 100%;
  background: var(--bg-input);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  overflow: hidden;
  flex-shrink: 0;
}
.folder-card-cover img {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
}
/* 无封面占位图标按比例缩放，避免撑破卡片 */
.folder-card-cover > svg {
  width: 32%;
  height: 32%;
  max-width: 48px;
  max-height: 48px;
}
.folder-card-info {
  padding: 8px 10px;
  min-width: 0;
  box-sizing: border-box;
}
.folder-card-play {
  position: absolute;
  right: 8px;
  bottom: 8px;
  width: 30px;
  height: 30px;
  border-radius: 50%;
  background: rgb(var(--accent-rgb));
  color: var(--on-accent);
  display: flex;
  align-items: center;
  justify-content: center;
  opacity: 0;
  transform: translateY(4px);
  transition: opacity var(--dur-fast) var(--ease), transform var(--dur-fast) var(--ease);
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);
}
.folder-card:hover .folder-card-play { opacity: 1; transform: translateY(0); }
.folder-card-name {
  font-size: 12.5px;
  font-weight: 500;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.folder-card-sub {
  font-size: 10.5px;
  color: var(--text-muted);
  margin-top: 2px;
}

.fv-list-wrap { flex: 1; min-height: 0; padding: 4px 18px; }
.folder-row {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 100%;
  padding: 0 12px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: background var(--dur-fast) var(--ease);
}
.folder-row:hover { background: var(--bg-hover); }
.folder-row-cover {
  width: 44px;
  height: 44px;
  border-radius: var(--radius-sm);
  overflow: hidden;
  background: var(--bg-input);
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  flex-shrink: 0;
}
.folder-row-cover img { width: 100%; height: 100%; object-fit: cover; }
.folder-row-name {
  flex: 1;
  font-size: 13px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.folder-row-sub { font-size: 11px; color: var(--text-muted); }

.fv-tracks {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  padding: 8px 18px 4px;
}
.fv-tracks.has-children {
  flex: 0 0 auto;
  height: 42%;
}
.fv-tracks-head { padding: 4px 2px 8px; flex-shrink: 0; }
.fv-tracks-table {
  flex: 1;
  min-height: 0;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  overflow: hidden;
  background: var(--bg-card);
}
.fv-total {
  padding: 6px 20px 12px;
  font-size: 11px;
  color: var(--text-muted);
  text-align: right;
  flex-shrink: 0;
}
</style>
