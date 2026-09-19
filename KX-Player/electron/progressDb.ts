import fs from 'node:fs'
import Database from 'better-sqlite3'

/**
 * 播放进度与书签持久化（ASMR 长音频续播）。
 * 独立模块、独立连接访问与曲库同一个 SQLite 文件（WAL 支持多连接）。
 * 所有语句（含 DDL）一律 prepare() 参数绑定/静态预编译，不拼接 SQL。
 */

export interface PlayProgressRecord {
  trackId: string
  positionMs: number
  completed: boolean
  playCount: number
  playedAt: number
  lastSpeed: number | null
}

export interface BookmarkRecord {
  id: string
  trackId: string
  atMs: number
  label: string
  createdAt: number
}

type AppDatabase = Database.Database

let db: AppDatabase | null = null
let dbFilePath = ''

function openDatabase(filePath: string): AppDatabase {
  if (db && dbFilePath === filePath) return db
  if (db) {
    try { db.close() } catch { /* ignore */ }
  }
  db = new Database(filePath)
  dbFilePath = filePath
  db.pragma('journal_mode = WAL')
  db.pragma('cache_size = -4000')
  initializeSchema(db)
  return db
}

function initializeSchema(database: AppDatabase) {
  // 静态 DDL，逐条预编译执行，幂等
  const ddl = [
    `CREATE TABLE IF NOT EXISTS play_progress (
      track_id     TEXT PRIMARY KEY,
      position_ms  INTEGER NOT NULL DEFAULT 0,
      completed    INTEGER NOT NULL DEFAULT 0,
      play_count   INTEGER NOT NULL DEFAULT 1,
      played_at    INTEGER NOT NULL DEFAULT 0,
      last_speed   REAL
    )`,
    `CREATE INDEX IF NOT EXISTS idx_progress_played_at ON play_progress(played_at DESC)`,
    `CREATE TABLE IF NOT EXISTS bookmarks (
      id         TEXT PRIMARY KEY,
      track_id   TEXT NOT NULL,
      at_ms      INTEGER NOT NULL,
      label      TEXT NOT NULL DEFAULT '',
      created_at INTEGER NOT NULL
    )`,
    `CREATE INDEX IF NOT EXISTS idx_bookmarks_track ON bookmarks(track_id, at_ms)`,
  ]
  for (const stmt of ddl) database.prepare(stmt).run()
}

export function closeProgressDb(): void {
  if (db) {
    try { db.close() } catch { /* ignore */ }
    db = null
    dbFilePath = ''
  }
}

// ── 播放进度（≥95% 视为听完，由前端判定）────────────────────────
export function getAllProgress(dbPath: string): PlayProgressRecord[] {
  if (!fs.existsSync(dbPath)) return []
  const database = openDatabase(dbPath)
  const rows = database.prepare('SELECT * FROM play_progress').all() as Record<string, unknown>[]
  return rows.map((r) => ({
    trackId: String(r.track_id),
    positionMs: Number(r.position_ms) || 0,
    completed: Number(r.completed) === 1,
    playCount: Number(r.play_count) || 0,
    playedAt: Number(r.played_at) || 0,
    lastSpeed: r.last_speed == null ? null : Number(r.last_speed),
  }))
}

/** completed 传 null 表示保留原值（播放中回写位置时） */
export function setProgress(
  dbPath: string,
  trackId: string,
  positionMs: number,
  completed: boolean | null,
  lastSpeed: number | null,
): void {
  const database = openDatabase(dbPath)
  database.prepare(`
    INSERT INTO play_progress (track_id, position_ms, completed, play_count, played_at, last_speed)
    VALUES (?, ?, ?, 1, ?, ?)
    ON CONFLICT(track_id) DO UPDATE SET
      position_ms = excluded.position_ms,
      completed = CASE WHEN excluded.completed < 0 THEN play_progress.completed ELSE excluded.completed END,
      play_count = play_progress.play_count + 1,
      played_at = excluded.played_at,
      last_speed = excluded.last_speed
  `).run(trackId, Math.max(0, Math.floor(positionMs)), completed == null ? -1 : (completed ? 1 : 0), Date.now(), lastSpeed)
}

export function markCompleted(dbPath: string, trackId: string, completed: boolean): void {
  const database = openDatabase(dbPath)
  database.prepare(`
    INSERT INTO play_progress (track_id, position_ms, completed, play_count, played_at)
    VALUES (?, 0, ?, 1, ?)
    ON CONFLICT(track_id) DO UPDATE SET completed = excluded.completed
  `).run(trackId, completed ? 1 : 0, Date.now())
}

export function clearProgress(dbPath: string, trackId: string): void {
  const database = openDatabase(dbPath)
  database.prepare('DELETE FROM play_progress WHERE track_id = ?').run(trackId)
}

/** 曲库重建后清理悬空进度与书签（条目被移除时） */
export function pruneProgressAndBookmarks(dbPath: string): void {
  if (!fs.existsSync(dbPath)) return
  const database = openDatabase(dbPath)
  database.prepare('DELETE FROM play_progress WHERE track_id NOT IN (SELECT id FROM tracks)').run()
  database.prepare('DELETE FROM bookmarks WHERE track_id NOT IN (SELECT id FROM tracks)').run()
}

// ── 书签 ────────────────────────────────────────────────────────
export function listBookmarks(dbPath: string, trackId: string): BookmarkRecord[] {
  if (!fs.existsSync(dbPath)) return []
  const database = openDatabase(dbPath)
  const rows = database.prepare(
    'SELECT * FROM bookmarks WHERE track_id = ? ORDER BY at_ms ASC',
  ).all(trackId) as Record<string, unknown>[]
  return rows.map((r) => ({
    id: String(r.id),
    trackId: String(r.track_id),
    atMs: Number(r.at_ms) || 0,
    label: String(r.label ?? ''),
    createdAt: Number(r.created_at) || 0,
  }))
}

export function addBookmark(dbPath: string, trackId: string, atMs: number, label: string): BookmarkRecord {
  const database = openDatabase(dbPath)
  const id = `bm_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 8)}`
  const createdAt = Date.now()
  database.prepare(
    'INSERT INTO bookmarks (id, track_id, at_ms, label, created_at) VALUES (?, ?, ?, ?, ?)',
  ).run(id, trackId, Math.max(0, Math.floor(atMs)), label, createdAt)
  return { id, trackId, atMs: Math.floor(atMs), label, createdAt }
}

export function renameBookmark(dbPath: string, id: string, label: string): void {
  const database = openDatabase(dbPath)
  database.prepare('UPDATE bookmarks SET label = ? WHERE id = ?').run(label, id)
}

export function removeBookmark(dbPath: string, id: string): void {
  const database = openDatabase(dbPath)
  database.prepare('DELETE FROM bookmarks WHERE id = ?').run(id)
}
