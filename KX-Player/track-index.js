// === Track Index ===
// Small helper around the library track array for O(1) id lookups.

export function createTrackIndex() {
  let byId = new Map()

  return {
    rebuild(tracks) {
      byId = new Map((tracks || []).map(t => [t.id, t]))
    },
    get(id) {
      return id ? byId.get(id) || null : null
    },
    fromIds(ids) {
      const out = []
      for (const id of (ids || [])) {
        const t = byId.get(id)
        if (t) out.push(t)
      }
      return out
    },
  }
}
