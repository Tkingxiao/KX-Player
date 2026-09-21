import { describe, it, expect } from 'vitest'
import { AppError, asAppErrorCode, toAppError, errText } from '../src/contracts/result'

describe('asAppErrorCode', () => {
  it('已知码原样通过', () => {
    expect(asAppErrorCode('INVALID_ARGUMENT')).toBe('INVALID_ARGUMENT')
    expect(asAppErrorCode('MPV_EMBED_FAILED')).toBe('MPV_EMBED_FAILED')
  })

  it('未知或缺席退成 INTERNAL，不造出不存在的字面量', () => {
    expect(asAppErrorCode('NOT_A_CODE')).toBe('INTERNAL')
    expect(asAppErrorCode(undefined)).toBe('INTERNAL')
    expect(asAppErrorCode(null)).toBe('INTERNAL')
    expect(asAppErrorCode('internal')).toBe('INTERNAL')
  })
})

describe('toAppError', () => {
  it('Rust IpcError 载荷（camelCase）保留码与 detail', () => {
    const e = toAppError({ code: 'DB_FAILED', message: '读取曲库失败', detail: 'no such column: foo' })
    expect(e).toBeInstanceOf(AppError)
    expect(e.code).toBe('DB_FAILED')
    expect(e.message).toBe('读取曲库失败')
    expect(e.detail).toBe('no such column: foo')
  })

  it('detail 缺席时保持缺席，不写成空串', () => {
    expect(toAppError({ code: 'BUSY', message: '播放器正忙' }).detail).toBeUndefined()
  })

  it('缺 code 的载荷按 INTERNAL 处理，不猜码', () => {
    expect(toAppError({ message: '未知错误' }).code).toBe('INTERNAL')
  })

  it('旧版 Err(String) 载荷仍是错误而不是异常', () => {
    const e = toAppError('目标目录已存在')
    expect(e.code).toBe('INTERNAL')
    expect(e.message).toBe('目标目录已存在')
  })

  it('前端自身抛的 Error 保留原文，stack 落到 detail', () => {
    const e = toAppError(new TypeError('x is not iterable'))
    expect(e.code).toBe('INTERNAL')
    expect(e.message).toBe('x is not iterable')
    expect(e.detail).toContain('at ')
  })

  it('已经是 AppError 时不重新包装（跨层传递不丢码）', () => {
    const first = toAppError({ code: 'CANCELLED', message: '已取消' })
    expect(toAppError(first)).toBe(first)
  })

  it('完全意外的值不会抛二次异常', () => {
    expect(toAppError(undefined).message).toBe('undefined')
    expect(toAppError(42).code).toBe('INTERNAL')
  })
})

describe('errText', () => {
  it('只给用户看得懂的那半句，不带 [CODE] 前缀与 detail', () => {
    const t = errText({ code: 'IO_FAILED', message: '无法写入封面', detail: 'EACCES' })
    expect(t).toBe('无法写入封面')
  })
})
