// === UI Feedback ===
// Confirm modal and import/progress toast helpers.

import { $, esc } from './utils.js'

export function showConfirm(title, message) {
  return new Promise(resolve => {
    $('confirm-title').textContent = title
    $('confirm-message').textContent = message
    $('confirm-modal').classList.remove('hidden')
    const okBtn = $('confirm-ok-btn')
    const cancelBtn = $('confirm-cancel-btn')
    const cleanup = () => {
      $('confirm-modal').classList.add('hidden')
      okBtn.removeEventListener('click', okHandler)
      cancelBtn.removeEventListener('click', cancelHandler)
      document.removeEventListener('keydown', keyHandler)
    }
    const okHandler = () => { cleanup(); resolve(true) }
    const cancelHandler = () => { cleanup(); resolve(false) }
    const keyHandler = (e) => {
      if (e.key === 'Escape') { e.preventDefault(); cancelHandler() }
      if (e.key === 'Enter') { e.preventDefault(); okHandler() }
    }
    okBtn.addEventListener('click', okHandler)
    cancelBtn.addEventListener('click', cancelHandler)
    document.addEventListener('keydown', keyHandler)
    okBtn.focus()
  })
}

let toastCounter = 0

export function addT(fn) {
  const id = 'toast-' + ++toastCounter
  $('import-toasts').insertAdjacentHTML('beforeend',
    `<div class="import-toast" id="${id}"><div class="toast-header"><span class="toast-name">${esc(fn)}</span><span class="toast-status" id="${id}-status">\u626b\u63cf\u4e2d...</span></div><div class="toast-progress-bar"><div class="toast-progress-fill" id="${id}-bar" style="width:0%"></div></div><div class="toast-detail" id="${id}-detail"></div></div>`)
  return id
}

export function updT(id, status, pct, detail) {
  const s = $(`${id}-status`), b = $(`${id}-bar`), d = $(`${id}-detail`)
  if (s) s.textContent = status
  if (b) { b.style.width = pct + '%'; if (pct > 0) b.closest('.toast-progress-bar').style.display = 'block' }
  if (d) d.textContent = detail || ''
}

export function rmT(id) {
  const t = $(id)
  if (t) {
    setTimeout(() => {
      t.classList.add('toast-exit')
      setTimeout(() => t.remove(), 500)
    }, 1500)
  }
}
