/** 曲目/列表右键菜单与播放上下文的共用逻辑。 */
import { usePlayerStore } from '@/stores/player'
import { usePlaylistsStore } from '@/stores/playlists'
import { useUiStore } from '@/stores/ui'
import { useLibraryStore } from '@/stores/library'
import { useTaxonomyStore } from '@/stores/taxonomy'
import { api } from '@/bridge/ipc'
import type { Track } from '@/contracts/api'
import type { ContextMenuItem } from '@/stores/ui'
import { trackDir } from '@/utils/format'
import confirmHost from '@/services/confirmHost'

export function useTrackActions() {
  const player = usePlayerStore()
  const playlists = usePlaylistsStore()
  const ui = useUiStore()
  const library = useLibraryStore()
  const taxonomy = useTaxonomyStore()

  function playTrackInContext(track: Track, list: string[], name: string): void {
    // 是否出画面由「当前有没有显示画面的表面」决定（见 player store 的 videoMode），
    // 这里只负责起播；调用方负责把舞台/悬浮窗打开。
    void player.playTrack(track, list, name, { fromLast: false })
  }

  function playTrackFromLast(track: Track, list: string[], name: string): void {
    void player.playTrack(track, list, name, { fromLast: true })
  }

  function copyText(text: string, label: string): void {
    void api.clipboardWriteText(text).then((ok) => {
      ui.toast(ok ? `已复制${label}` : '复制失败', ok ? 'success' : 'error')
    })
  }

  /** 加入分类子菜单（含新建入口） */
  function buildCategoryTargets(track: Track): ContextMenuItem[] {
    const cats = taxonomy.flatCategories
    const targets: ContextMenuItem[] = cats.map((c) => ({
      label: c.name,
      action: () => void taxonomy.assignCategory(c.id, [track.id]),
    }))
    targets.push({ label: '＋ 新建分类…', separatorBefore: targets.length > 0, action: () => {
      const name = prompt('新建分类并加入')
      if (name) {
        void api.taxonomyCreateCategory(null, name).then((cat) => {
          if (cat) void taxonomy.assignCategory(cat.id, [track.id])
        })
      }
    } })
    return targets.length ? targets : [{ label: '（输入名称创建）', disabled: true }]
  }

  /** 打标签子菜单（已打的有勾选标记，点击取消） */
  function buildTagTargets(track: Track): ContextMenuItem[] {
    const tagged = new Set(taxonomy.tagsOfTrack(track.id))
    const targets: ContextMenuItem[] = taxonomy.tags.map((t) => ({
      label: (tagged.has(t.id) ? '✓ ' : '') + t.name,
      action: () => {
        if (tagged.has(t.id)) void taxonomy.untagTracks(t.id, [track.id])
        else void taxonomy.tagTracks([t.id], [track.id])
      },
    }))
    targets.push({ label: '＋ 新建标签…', separatorBefore: targets.length > 0, action: () => {
      const name = prompt('新建标签并打上')
      if (name) {
        void taxonomy.upsertTag(name).then((tag) => {
          if (tag) void taxonomy.tagTracks([tag.id], [track.id])
        })
      }
    } })
    return targets.length ? targets : [{ label: '（输入名称创建）', disabled: true }]
  }

  function trackMenuItems(track: Track, list: string[], listName: string): ContextMenuItem[] {
    const isFav = playlists.isFavorite(track.id)
    const favTargets = playlists.favs.map((f) => ({
      label: f.name,
      disabled: f.isDefault && f.trackIds.includes(track.id),
      action: () => {
        playlists.addToList(f.id, [track.id])
        ui.toast(`已加入「${f.name}」`, 'success')
      },
    }))
    const plTargets = playlists.pls.map((p) => ({
      label: p.name,
      action: () => {
        playlists.addToList(p.id, [track.id])
        ui.toast(`已加入「${p.name}」`, 'success')
      },
    }))
    const p = library.progress.get(track.id)
    const items: ContextMenuItem[] = [
      { label: '立即播放', action: () => playTrackInContext(track, list, listName) },
    ]
    if (p && !p.completed && p.positionMs > 0) {
      items.push({ label: '从上次位置继续', action: () => playTrackFromLast(track, list, listName) })
    }
    items.push(
      { label: '下一首播放', action: () => {
        const idx = player.queue.indexOf(player.currentId ?? '')
        player.queue = [...player.queue.slice(0, idx + 1), track.id, ...player.queue.slice(idx + 1)]
        ui.toast('已加入下一首播放', 'success')
      } },
      {
        label: isFav ? '取消收藏' : '加入收藏',
        action: () => {
          const added = playlists.toggleFavorite(track.id)
          ui.toast(added ? '已收藏' : '已取消收藏', 'success')
        },
      },
      {
        label: '加入收藏夹',
        children: favTargets.length ? favTargets : [{ label: '（无其他收藏夹）', disabled: true }],
      },
      {
        label: '加入播放列表',
        children: plTargets.length ? plTargets : [{ label: '（尚无播放列表）', disabled: true }],
      },
      { label: '-', separatorBefore: true },
      {
        label: '加入分类',
        children: buildCategoryTargets(track),
      },
      {
        label: '打标签',
        children: buildTagTargets(track),
      },
      {
        label: '自动打标…',
        action: () => { ui.autoTagReview = { trackIds: [track.id] } },
      },
      { label: '-', separatorBefore: true },
      { label: '打开歌词', action: () => {
        void player.playTrack(track, list, listName, { fromLast: true })
        ui.view = 'lyrics'
      } },
      {
        label: '复制标题',
        action: () => copyText(track.name || '', '标题'),
      },
      { label: '复制文件路径', action: () => copyText(track.path, '路径') },
      {
        label: '打开所在文件夹',
        action: () => void api.showItemInFolder(track.path),
      },
      {
        label: '定位到所在目录',
        action: () => {
          const dir = trackDir(track)
          if (dir) {
            ui.view = 'folder'
            ui.openFolder(dir)
          }
        },
      },
    )
    if (p && (p.positionMs > 0 || p.completed)) {
      items.push(
        { label: '-', separatorBefore: true },
        { label: '清除播放进度', danger: true, action: () => {
          void api.clearTrackProgress(track.id).then(() => {
            library.progress.delete(track.id)
            library.progress = new Map(library.progress)
            ui.toast('已清除进度', 'success')
          })
        } },
      )
    }
    return items
  }

  function showTrackMenu(e: MouseEvent, track: Track, list: string[], listName: string): void {
    e.preventDefault()
    ui.ctxMenu = { x: e.clientX, y: e.clientY, items: trackMenuItems(track, list, listName) }
  }

  async function confirmRemoveList(id: string, name: string): Promise<void> {
    const ok = await confirmHost.open({
      title: '删除列表',
      message: `确定删除「${name}」吗？列表内的文件不会被删除。`,
      confirmText: '删除',
      danger: true,
    })
    if (ok) {
      playlists.removeList(id)
      if (ui.listPath === id) {
        ui.listPath = null
        ui.view = 'smart'
      }
    }
  }

  return { playTrackInContext, playTrackFromLast, trackMenuItems, showTrackMenu, confirmRemoveList, copyText }
}
