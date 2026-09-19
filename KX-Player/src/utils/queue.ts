/** 播放队列模式逻辑（纯函数，可单测）。 */
export type PlayMode = 0 | 1 | 2 | 3 // 顺序循环 / 随机 / 单曲循环 / 播完停止

export function nextIndex(mode: PlayMode, current: number, length: number): number | null {
  if (length === 0) return null
  if (mode === 1) {
    if (length === 1) return 0
    let r = current
    while (r === current) r = Math.floor(Math.random() * length)
    return r
  }
  return (current + 1) % length
}

export function prevIndex(_mode: PlayMode, current: number, length: number): number | null {
  if (length === 0) return null
  return current <= 0 ? length - 1 : current - 1
}

/** 歌曲结束时的行为：mode 2 重播；mode 3 停止；其余下一首。返回 'repeat' | 'stop' | 'next' */
export function onEnded(mode: PlayMode): 'repeat' | 'stop' | 'next' {
  if (mode === 2) return 'repeat'
  if (mode === 3) return 'stop'
  return 'next'
}
