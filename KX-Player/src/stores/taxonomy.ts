/** 分类与标签 store（P0-11/12/13）：分类树、标签云、track↔tag/category 映射与打标操作。 */
import { defineStore } from 'pinia'
import { ref, shallowRef, computed } from 'vue'
import { api } from '@/bridge/ipc'
import { useUiStore } from '@/stores/ui'
import type { Category, Tag, TagSuggestion } from '@/contracts/api'

export const useTaxonomyStore = defineStore('taxonomy', () => {
  const categories = shallowRef<Category[]>([])
  const tags = shallowRef<Tag[]>([])
  /** tagId → trackIds（筛选用，懒加载后维护） */
  const tracksByTag = shallowRef<Map<number, Set<string>>>(new Map())
  /** categoryId → trackIds（含子孙） */
  const tracksByCategory = shallowRef<Map<number, Set<string>>>(new Map())
  const loaded = ref(false)

  function flatten(nodes: Category[], out: Category[] = []): Category[] {
    for (const n of nodes) {
      out.push(n)
      flatten(n.children, out)
    }
    return out
  }

  const flatCategories = computed<Category[]>(() => flatten(categories.value))

  function rebuildMappings(): void {
    void api.taxonomyAllTrackTags().then((rows) => {
      const m = new Map<number, Set<string>>()
      for (const { trackId, tagId } of rows) {
        if (!m.has(tagId)) m.set(tagId, new Set())
        m.get(tagId)!.add(trackId)
      }
      tracksByTag.value = m
    })
    void api.taxonomyAllCategoryItems().then((rows) => {
      // 子孙分类合并（tree 模式）
      const direct = new Map<number, Set<string>>()
      for (const { categoryId, trackId } of rows) {
        if (!direct.has(categoryId)) direct.set(categoryId, new Set())
        direct.get(categoryId)!.add(trackId)
      }
      const m = new Map<number, Set<string>>()
      for (const cat of flatten(categories.value)) {
        const ids = new Set<string>(direct.get(cat.id) ?? [])
        for (const child of flatten(cat.children)) {
          for (const t of direct.get(child.id) ?? []) ids.add(t)
        }
        m.set(cat.id, ids)
      }
      tracksByCategory.value = m
    })
  }

  async function refresh(): Promise<void> {
    categories.value = await api.taxonomyListCategories()
    tags.value = await api.taxonomyListTags()
    rebuildMappings()
    loaded.value = true
  }

  const ui = useUiStore()

  /** 某条目已打的标签 id 集（右键菜单勾选态） */
  function tagsOfTrack(trackId: string): number[] {
    const out: number[] = []
    for (const [tagId, ids] of tracksByTag.value) {
      if (ids.has(trackId)) out.push(tagId)
    }
    return out
  }

  async function createCategory(parentId: number | null, name: string): Promise<void> {
    const cat = await api.taxonomyCreateCategory(parentId, name)
    if (cat) {
      ui.toast(`已创建分类「${name}」`, 'success')
      await refresh()
    } else {
      ui.toast('创建分类失败', 'error')
    }
  }

  async function renameCategory(id: number, name: string): Promise<void> {
    if (await api.taxonomyRenameCategory(id, name)) {
      await refresh()
    } else {
      ui.toast('分类重命名失败，名称可能已存在', 'error')
    }
  }

  async function deleteCategory(id: number): Promise<void> {
    if (await api.taxonomyDeleteCategory(id)) {
      // 当前筛选命中的分类（或其子孙）被删除时，清空分类筛选，避免悬空 id 导致结果恒为空
      if (ui.categoryId !== null) {
        const doomed = new Set<number>([id])
        const collect = (nodes: Category[]): void => {
          for (const n of nodes) {
            doomed.add(n.id)
            collect(n.children)
          }
        }
        const findNode = (nodes: Category[]): Category | null => {
          for (const n of nodes) {
            if (n.id === id) return n
            const hit = findNode(n.children)
            if (hit) return hit
          }
          return null
        }
        const target = findNode(categories.value)
        if (target) collect(target.children)
        if (doomed.has(ui.categoryId)) ui.categoryId = null
      }
      ui.toast('已删除分类', 'success')
      await refresh()
    }
  }

  async function assignCategory(categoryId: number, trackIds: string[]): Promise<void> {
    if (await api.taxonomyAssignCategory(categoryId, trackIds)) {
      ui.toast(`已加入分类（${trackIds.length} 项）`, 'success')
      await refresh()
    }
  }

  async function unassignCategory(categoryId: number, trackIds: string[]): Promise<void> {
    if (await api.taxonomyUnassignCategory(categoryId, trackIds)) await refresh()
  }

  async function tagTracks(tagIds: number[], trackIds: string[], mode: 'add' | 'replace' = 'add'): Promise<void> {
    if (await api.taxonomyTagTracks(tagIds, trackIds, mode)) {
      ui.toast(`已打标签（${trackIds.length} 项）`, 'success')
      await refresh()
    }
  }

  async function untagTracks(tagId: number, trackIds: string[]): Promise<void> {
    if (await api.taxonomyUntagTracks(tagId, trackIds)) await refresh()
  }

  async function upsertTag(name: string, color?: string | null): Promise<Tag | null> {
    const t = await api.taxonomyUpsertTag(name, color ?? null)
    if (t) {
      await refresh()
    } else {
      ui.toast('新建标签失败', 'error')
    }
    return t
  }

  async function renameTag(id: number, name: string): Promise<void> {
    if (await api.taxonomyRenameTag(id, name)) {
      ui.toast(`已重命名为「${name.trim()}」`, 'success')
      await refresh()
    } else {
      ui.toast('标签重命名失败，名称可能已存在', 'error')
    }
  }

  async function deleteTag(id: number): Promise<void> {
    if (await api.taxonomyDeleteTag(id)) {
      if (ui.tagIds.has(id)) {
        const next = new Set(ui.tagIds)
        next.delete(id)
        ui.tagIds = next
      }
      await refresh()
    }
  }

  /** 自动打标：生成建议 → 弹确认（AutoTagReview 由调用方渲染）→ 应用 */
  async function suggestTags(trackIds: string[]): Promise<{ trackId: string; suggestions: TagSuggestion[] }[]> {
    return api.taxonomySuggestTags(trackIds)
  }

  async function applySuggested(accepted: { trackId: string; tagName: string }[]): Promise<void> {
    if (accepted.length && (await api.taxonomyApplySuggestedTags(accepted))) {
      ui.toast(`已写入 ${accepted.length} 条标签`, 'success')
      await refresh()
    }
  }

  return {
    categories, tags, tracksByTag, tracksByCategory, flatCategories, loaded,
    refresh, tagsOfTrack, createCategory, renameCategory, deleteCategory,
    assignCategory, unassignCategory, tagTracks, untagTracks,
    upsertTag, renameTag, deleteTag, suggestTags, applySuggested,
  }
})
