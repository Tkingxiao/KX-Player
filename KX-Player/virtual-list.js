// === KX-Player Virtual List ===
// High-performance virtual scrolling list renderer.
// Only renders visible rows + buffer, reuses DOM on scroll.

import { $ } from './utils.js'

// Resize throttler: suspend VL renders during window resize to avoid jank
let _resizeActive = false
let _resizeTimer = null
export function _startResizeThrottle() {
  _resizeActive = true
  clearTimeout(_resizeTimer)
  _resizeTimer = setTimeout(() => { _resizeActive = false }, 300)
}

export function virtualList(containerId, items, rowHeight, renderItem, onClick) {
  const c = $(containerId)
  if (!c) return
  // Remove previous listeners and observer to prevent accumulation
  if (c._vlRO) { c._vlRO.disconnect(); c._vlRO = null }
  if (c._vlScrollFn) { c.removeEventListener('scroll', c._vlScrollFn) }
  if (c._vlClickFn) { c.removeEventListener('click', c._vlClickFn) }
  if (c._vlDblClickFn) { c.removeEventListener('dblclick', c._vlDblClickFn) }
  c.innerHTML = ''
  if (!items.length) { c.innerHTML = '<div class="empty-state"><div class="empty-state-icon">♪</div><h3>暂无内容</h3></div>'; return }
  const totalH = items.length * rowHeight
  const spacer = document.createElement('div'); spacer.style.height = totalH + 'px'; spacer.style.position = 'relative'
  const view = document.createElement('div'); view.style.position = 'absolute'; view.style.top = '0'; view.style.left = '0'; view.style.right = '0'
  spacer.appendChild(view); c.appendChild(spacer)
  const buffer = 10
  function render() {
    if (_resizeActive) return
    const scrollTop = c.scrollTop, clientH = c.clientHeight || 600
    const start = Math.max(0, Math.floor(scrollTop / rowHeight) - buffer)
    const end = Math.min(items.length, Math.ceil((scrollTop + clientH) / rowHeight) + buffer)
    view.style.top = (start * rowHeight) + 'px'
    let html = ''
    for (let i = start; i < end; i++) html += renderItem(items[i], i)
    view.innerHTML = html
  }
  c._vlRender = render; c._vlItems = items; render()
  const scrollFn = () => render()
  c._vlScrollFn = scrollFn
  c.addEventListener('scroll', scrollFn, { passive: true })
  let resizeTimer
  const ro = new ResizeObserver(() => {
    if (_resizeActive) return
    clearTimeout(resizeTimer)
    resizeTimer = setTimeout(render, 200)
  })
  ro.observe(c); c._vlRO = ro
  if (onClick) {
    const clickFn = e => {
      const playBtn = e.target.closest('.idx-play-btn')
      if (playBtn) { const row = e.target.closest('.song-row'); if (row && row.dataset.tid) onClick(row.dataset.tid, true) }
    }
    const dblClickFn = e => {
      const row = e.target.closest('.song-row')
      if (row && row.dataset.tid) onClick(row.dataset.tid, true)
    }
    c._vlClickFn = clickFn; c._vlDblClickFn = dblClickFn
    c.addEventListener('click', clickFn)
    c.addEventListener('dblclick', dblClickFn)
  }
}

export function invalidateVL(containerId) {
  const c = $(containerId)
  if (c && c._vlRender) c._vlRender()
}