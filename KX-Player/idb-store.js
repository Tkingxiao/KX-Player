// === IndexedDB Store ===
// Tiny promise wrappers for renderer-local settings/cache stores.

let idb = null

function openIDB() {
  return new Promise((resolve, reject) => {
    if (idb) return resolve(idb)
    const req = indexedDB.open('kx-player-db', 1)
    req.onupgradeneeded = () => {
      const db = req.result
      if (!db.objectStoreNames.contains('settings')) db.createObjectStore('settings')
      if (!db.objectStoreNames.contains('cache')) db.createObjectStore('cache')
    }
    req.onsuccess = () => { idb = req.result; resolve(idb) }
    req.onerror = () => reject(req.error)
  })
}

export async function idbSet(store, key, val) {
  const db = await openIDB()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(store, 'readwrite')
    tx.objectStore(store).put(val, key)
    tx.oncomplete = () => resolve()
    tx.onerror = () => reject(tx.error)
  })
}

export async function idbGet(store, key) {
  const db = await openIDB()
  return new Promise((resolve, reject) => {
    const tx = db.transaction(store, 'readonly')
    const req = tx.objectStore(store).get(key)
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error)
  })
}
