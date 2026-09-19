/** 文件夹树元数据预计算（移植自旧 folder-tree.js）。 */
import type { FolderNode, Track } from '@/contracts/api'
import { normDir } from '@/utils/format'

export interface FolderMeta {
  trackCount: number
  hasMusic: boolean
  validChildCount: number
  latestMtime: number
  coverData: string | null
}

function walk(node: FolderNode, meta: Map<string, FolderMeta>): FolderMeta {
  let trackCount = node.tracks.length
  let validChildCount = 0
  let latestMtime = 0
  let cover = node.coverData
  for (const t of node.tracks) {
    if (t.fileMtime > latestMtime) latestMtime = t.fileMtime
  }
  for (const child of node.children) {
    const cm = walk(child, meta)
    if (cm.hasMusic) validChildCount++
    trackCount += cm.trackCount
    if (cm.latestMtime > latestMtime) latestMtime = cm.latestMtime
    if (!cover) cover = cm.coverData
  }
  const m: FolderMeta = {
    trackCount,
    hasMusic: trackCount > 0 || validChildCount > 0,
    validChildCount,
    latestMtime,
    coverData: cover,
  }
  meta.set(normDir(node.path), m)
  return m
}

export function buildFolderMeta(tree: FolderNode[]): Map<string, FolderMeta> {
  const meta = new Map<string, FolderMeta>()
  for (const root of tree) walk(root, meta)
  return meta
}

export function findNodeByPath(tree: FolderNode[], path: string): FolderNode | null {
  const target = normDir(path)
  for (const root of tree) {
    const r = findNodeByPathInner(root, target)
    if (r) return r
  }
  return null
}

function findNodeByPathInner(node: FolderNode, target: string): FolderNode | null {
  if (normDir(node.path) === target) return node
  for (const child of node.children) {
    const r = findNodeByPathInner(child, target)
    if (r) return r
  }
  return null
}

export function collectAllTracks(node: FolderNode): Track[] {
  const out: Track[] = []
  const stack: FolderNode[] = [node]
  while (stack.length) {
    const n = stack.pop()!
    for (const t of n.tracks) out.push(t)
    for (let i = n.children.length - 1; i >= 0; i--) stack.push(n.children[i])
  }
  return out
}
