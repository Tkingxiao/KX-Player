// === Folder Tree Helpers ===

export function hasMusicRecursive(node) {
  if (!node) return false
  if (node.tracks && node.tracks.length > 0) return true
  for (const c of (node.children || [])) { if (hasMusicRecursive(c)) return true }
  return false
}

export function buildFolderMeta(tree) {
  const meta = {}
  function walk(node) {
    const p = node.path
    let trackCount = (node.tracks || []).length
    let hasMusic = trackCount > 0
    let coverData = node.coverData || null
    let validChildCount = 0
    for (const c of (node.children || [])) {
      walk(c)
      const childMeta = meta[c.path]
      if (childMeta && childMeta.hasMusic) {
        validChildCount++
        hasMusic = true
        trackCount += childMeta.trackCount
        if (!coverData && childMeta.coverData) coverData = childMeta.coverData
      }
    }
    if (!coverData && node.tracks && node.tracks.length) {
      for (const t of node.tracks) { if (t.coverData) { coverData = t.coverData; break } }
    }
    meta[p] = { trackCount, hasMusic, validChildCount, coverData }
  }
  for (const n of (tree || [])) walk(n)
  return meta
}

export function findNodeByPath(tree, path) {
  if (!tree || !path) return null
  const np = path.replace(/\\/g, '/')
  for (const n of tree) {
    const npath = n.path.replace(/\\/g, '/')
    if (npath === np) return n
    const r = findNodeByPath(n.children, np)
    if (r) return r
  }
  return null
}
