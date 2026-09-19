/** 收藏夹与播放列表 store（默认收藏夹不可删除/重命名）。 */
import { defineStore } from 'pinia'
import { computed } from 'vue'
import { useSettingsStore } from '@/stores/settings'

export interface UserList {
  id: string
  name: string
  trackIds: string[]
  isDefault?: boolean
}

let listSeq = 0
function newId(prefix: string): string {
  return `${prefix}_${Date.now().toString(36)}_${(++listSeq).toString(36)}${Math.random().toString(36).slice(2, 6)}`
}

export const usePlaylistsStore = defineStore('playlists', () => {
  const settings = useSettingsStore()

  const favs = computed<UserList[]>(() => settings.favs)
  const pls = computed<UserList[]>(() => settings.pls)

  function persist(): void {
    settings.scheduleSave()
  }

  function isFavorite(trackId: string): boolean {
    const def = favs.value.find((f) => f.isDefault)
    return !!def && def.trackIds.includes(trackId)
  }

  function toggleFavorite(trackId: string): boolean {
    let def = favs.value.find((f) => f.isDefault)
    if (!def) {
      def = { id: 'fav-default', name: '我的收藏', trackIds: [], isDefault: true }
      settings.favs = [def, ...favs.value]
    }
    if (def.trackIds.includes(trackId)) {
      def.trackIds = def.trackIds.filter((x) => x !== trackId)
      persist()
      return false
    }
    def.trackIds = [trackId, ...def.trackIds]
    persist()
    return true
  }

  function createList(name: string, kind: 'fav' | 'pl'): UserList | null {
    const trimmed = name.trim()
    if (!trimmed) return null
    const list: UserList = { id: newId(kind), name: trimmed, trackIds: [] }
    if (kind === 'fav') settings.favs = [...favs.value, list]
    else settings.pls = [...pls.value, list]
    persist()
    return list
  }

  function removeList(id: string): void {
    settings.favs = favs.value.filter((f) => f.id !== id && !f.isDefault)
    settings.pls = pls.value.filter((p) => p.id !== id)
    persist()
  }

  function renameList(id: string, name: string): void {
    const trimmed = name.trim()
    if (!trimmed) return
    settings.favs = favs.value.map((f) => (f.id === id && !f.isDefault ? { ...f, name: trimmed } : f))
    settings.pls = pls.value.map((p) => (p.id === id ? { ...p, name: trimmed } : p))
    persist()
  }

  /** 加入列表（去重）。返回是否新增。 */
  function addToList(id: string, trackIds: string[]): number {
    let added = 0
    const apply = (l: UserList): UserList => {
      if (l.id !== id) return l
      const set = new Set(l.trackIds)
      for (const t of trackIds) {
        if (!set.has(t)) { set.add(t); added++ }
      }
      return { ...l, trackIds: [...set] }
    }
    settings.favs = favs.value.map(apply)
    settings.pls = pls.value.map(apply)
    persist()
    return added
  }

  function removeFromList(id: string, trackIds: string[]): void {
    const set = new Set(trackIds)
    const apply = (l: UserList): UserList =>
      l.id === id ? { ...l, trackIds: l.trackIds.filter((t) => !set.has(t)) } : l
    settings.favs = favs.value.map(apply)
    settings.pls = pls.value.map(apply)
    persist()
  }

  function listById(id: string): UserList | null {
    return favs.value.find((f) => f.id === id) ?? pls.value.find((p) => p.id === id) ?? null
  }

  return { favs, pls, isFavorite, toggleFavorite, createList, removeList, renameList, addToList, removeFromList, listById }
})
