// === KX-Player Virtual List ===
// High-performance virtual scrolling list renderer.
// Only renders visible rows + buffer, reuses DOM on scroll.

import { $ } from './utils.js'

function _releaseImages(root) {
  if (!root || !root.querySelectorAll) return
  root.querySelectorAll('img').forEach(img => {
    try {
      img.removeAttribute('src')
      img.removeAttribute('srcset')
    } catch { /* ignore */ }
  })
}

export function destroyVirtualListContainer(c, clearDom = true) {
  if (!c) return
  if (c._vlRO) { c._vlRO.disconnect(); c._vlRO = null }
  if (c._vlRAF) { cancelAnimationFrame(c._vlRAF); c._vlRAF = 0 }
  if (c._vlResizeTimer) { clearTimeout(c._vlResizeTimer); c._vlResizeTimer = null }
  if (c._vlScrollFn) { c.removeEventListener('scroll', c._vlScrollFn); c._vlScrollFn = null }
  if (c._vlClickFn) { c.removeEventListener('click', c._vlClickFn); c._vlClickFn = null }
  if (c._vlDblClickFn) { c.removeEventListener('dblclick', c._vlDblClickFn); c._vlDblClickFn = null }
  c._vlRender = null
  c._vlRebuild = null
  c._vlItems = null
  c._vlCols = null
  if (clearDom) {
    _releaseImages(c)
    c.innerHTML = ''
  }
}

export function destroyVirtualLists(root) {
  if (!root || !root.querySelectorAll) return
  const nodes = []
  if (root.matches && (root.matches('.vl-container') || root.matches('.virtual-vl'))) nodes.push(root)
  nodes.push(...root.querySelectorAll('.vl-container, .virtual-vl'))
  nodes.forEach(node => destroyVirtualListContainer(node, true))
}

// Resize throttler: suspend VL renders during window resize to avoid jank
let _resizeActive = false
let _resizeTimer = null
export function _startResizeThrottle() {
  _resizeActive = true
  clearTimeout(_resizeTimer)
  _resizeTimer = setTimeout(() => { _resizeActive = false }, 300)
}

export function _flushResizeThrottle() {
  _resizeActive = false
  clearTimeout(_resizeTimer)
  _resizeTimer = null
}

export function virtualList(containerId, items, rowHeight, renderItem, onClick, onVisibleItems) {
  const c = $(containerId)
  if (!c) return
  // Remove previous listeners and observer to prevent accumulation
  destroyVirtualListContainer(c, true)
  if (!items.length) { c.innerHTML = '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u6682\u65e0\u5185\u5bb9</h3></div>'; return }
  const totalH = items.length * rowHeight
  const spacer = document.createElement('div'); spacer.style.height = totalH + 'px'; spacer.style.position = 'relative'
  const view = document.createElement('div'); view.style.position = 'absolute'; view.style.top = '0'; view.style.left = '0'; view.style.right = '0'
  spacer.appendChild(view); c.appendChild(spacer)
  // Buffer kept small: each extra row can carry a cover <img> whose decoded
  // bitmap costs width*height*4 bytes in the renderer process.
  const buffer = 6
  let lastStart = -1
  let lastEnd = -1

  function render(force = false) {
    if (_resizeActive) return
    const scrollTop = c.scrollTop, clientH = c.clientHeight || 600
    const start = Math.max(0, Math.floor(scrollTop / rowHeight) - buffer)
    const end = Math.min(items.length, Math.ceil((scrollTop + clientH) / rowHeight) + buffer)
    if (!force && start === lastStart && end === lastEnd) return
    lastStart = start
    lastEnd = end
    view.style.top = (start * rowHeight) + 'px'
    let html = ''
    for (let i = start; i < end; i++) html += renderItem(items[i], i)
    view.innerHTML = html
    if (onVisibleItems) {
      try { onVisibleItems(items.slice(start, end), start, end) } catch { /* ignore */ }
    }
  }

  function scheduleRender(force = false) {
    if (c._vlRAF) return
    c._vlRAF = requestAnimationFrame(() => {
      c._vlRAF = 0
      render(force)
    })
  }

  c._vlRender = () => render(true)
  c._vlItems = items
  render(true)
  const scrollFn = () => scheduleRender(false)
  c._vlScrollFn = scrollFn
  c.addEventListener('scroll', scrollFn, { passive: true })
  const ro = new ResizeObserver(() => {
    if (_resizeActive) return
    clearTimeout(c._vlResizeTimer)
    c._vlResizeTimer = setTimeout(() => render(true), 200)
  })
  ro.observe(c); c._vlRO = ro
  if (onClick) {
    const clickFn = e => {
      const playBtn = e.target.closest('.idx-play-btn')
      if (playBtn) {
        const row = e.target.closest('.song-row')
        if (row && row.dataset.tid) {
          e.stopPropagation()
          onClick(row.dataset.tid, true)
        }
      }
    }
    const dblClickFn = e => {
      const row = e.target.closest('.song-row')
      if (row && row.dataset.tid) {
        e.stopPropagation()
        onClick(row.dataset.tid, true)
      }
    }
    c._vlClickFn = clickFn; c._vlDblClickFn = dblClickFn
    c.addEventListener('click', clickFn)
    c.addEventListener('dblclick', dblClickFn)
  }
}


export function virtualFolderList(containerId, items, rowHeight, renderItem, onVisibleItems) {
  const c = $(containerId)
  if (!c) return
  destroyVirtualListContainer(c, true)
  if (!items.length) { c.innerHTML = '<div class="empty-state"><div class="empty-state-icon">\u266a</div><h3>\u65e0\u5339\u914d\u6587\u4ef6\u5939</h3></div>'; return }
  const totalH = items.length * rowHeight
  const spacer = document.createElement('div'); spacer.style.height = totalH + 'px'; spacer.style.position = 'relative'
  const view = document.createElement('div'); view.style.position = 'absolute'; view.style.top = '0'; view.style.left = '0'; view.style.right = '0'
  spacer.appendChild(view); c.appendChild(spacer)
  // Buffer kept small: each extra row can carry a cover <img> whose decoded
  // bitmap costs width*height*4 bytes in the renderer process.
  const buffer = 6
  let lastStart = -1
  let lastEnd = -1

  function render(force = false) {
    const scrollTop = c.scrollTop, clientH = c.clientHeight || 600
    const start = Math.max(0, Math.floor(scrollTop / rowHeight) - buffer)
    const end = Math.min(items.length, Math.ceil((scrollTop + clientH) / rowHeight) + buffer)
    if (!force && start === lastStart && end === lastEnd) return
    lastStart = start
    lastEnd = end
    view.style.top = (start * rowHeight) + 'px'
    let html = ''
    for (let i = start; i < end; i++) html += renderItem(items[i], i)
    view.innerHTML = html
    if (onVisibleItems) {
      try { onVisibleItems(items.slice(start, end), start, end) } catch { /* ignore */ }
    }
  }

  function scheduleRender(force = false) {
    if (c._vlRAF) return
    c._vlRAF = requestAnimationFrame(() => {
      c._vlRAF = 0
      render(force)
    })
  }

  c._vlRender = () => render(true)
  c._vlItems = items
  render(true)
  const scrollFn = () => scheduleRender(false)
  c._vlScrollFn = scrollFn
  c.addEventListener('scroll', scrollFn, { passive: true })
  const ro = new ResizeObserver(() => {
    clearTimeout(c._vlResizeTimer)
    c._vlResizeTimer = setTimeout(() => render(true), 200)
  })
  ro.observe(c); c._vlRO = ro
}

export function invalidateVL(containerId) {
  const c = $(containerId)
  if (c && c._vlRender) c._vlRender()
}
