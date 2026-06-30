import fs from 'node:fs'
import path from 'node:path'
import initSqlJs from 'sql.js'

export interface TrackRecord {
  id: string
  name: string
  path: string
  duration: number
  artist: string
  album: string
  format: string
  isVideo: boolean
  coverPath: string | null
  coverData: string | null
  lyricsPath: string | null
  fileMtime: number
  fileSize: number
  metaTitle: string | null
  metaArtist: string | null
  genre: string | null
  bitrate: number | null
  sampleRate: number | null
  albumCoverData?: string | null
}

export interface AlbumRecord {
  name: string
  artist: string
  coverPath: string | null
  coverData: string | null
  tracks: TrackRecord[]
}

export interface ArtistRecord {
  name: string
  path: string
  albums: AlbumRecord[]
}

export interface FolderNodeRecord {
  name: string
  path: string
  children: FolderNodeRecord[]
  tracks: TrackRecord[]
  trackCount: number
  coverData: string | null
}

export interface LibrarySnapshot {
  folderPaths: string[]
  artists: ArtistRecord[]
  folderTree: FolderNodeRecord[]
  allTracks: TrackRecord[]
  fileCount: number
  scannedAt: number
}

type SqlJsDatabase = InstanceType<Awaited<ReturnType<typeof initSqlJs>>['Database']>

let SQL: Awaited<ReturnType<typeof initSqlJs>> | null = null
let db: SqlJsDatabase | null = null
let dbFilePath = ''

function normalizePath(input: string): string {
  return input.replace(/\\/g, '/').replace(/\/+$/, '')
}

async function ensureSql() {
  if (SQL) return SQL
  const candidateDirs = [
    path.join(process.resourcesPath || '', 'sqljs'),
    path.join(process.cwd(), 'node_modules', 'sql.js', 'dist'),
  ].filter(Boolean)
  SQL = await initSqlJs({
    locateFile: (file) => {
      for (const dir of candidateDirs) {
        const fullPath = path.join(dir, file)
        if (fs.existsSync(fullPath)) return fullPath
      }
      return path.join(candidateDirs[0] || process.cwd(), file)
    },
  })
  return SQL
}

function ensureParentDir(filePath: string) {
  const dir = path.dirname(filePath)
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true })
}

function openDatabase(filePath: string, sql: Awaited<ReturnType<typeof initSqlJs>>) {
  if (db && dbFilePath === filePath) return db
  if (db) {
    try { db.close() } catch { /* ignore */ }
  }

  ensureParentDir(filePath)
  if (fs.existsSync(filePath)) {
    db = new sql.Database(fs.readFileSync(filePath))
  } else {
    db = new sql.Database()
  }
  dbFilePath = filePath
  initializeSchema(db)
  return db
}

function initializeSchema(database: SqlJsDatabase) {
  database.exec(`
    CREATE TABLE IF NOT EXISTS library_meta (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS artists (
      artist_id INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL,
      root_path TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_artists_name ON artists(name);

    CREATE TABLE IF NOT EXISTS albums (
      album_id INTEGER PRIMARY KEY AUTOINCREMENT,
      artist_id INTEGER NOT NULL,
      name TEXT NOT NULL,
      artist_name TEXT NOT NULL,
      cover_path TEXT,
      cover_data TEXT,
      FOREIGN KEY (artist_id) REFERENCES artists(artist_id)
    );
    CREATE INDEX IF NOT EXISTS idx_albums_artist_id ON albums(artist_id);

    CREATE TABLE IF NOT EXISTS tracks (
      id TEXT PRIMARY KEY,
      artist_id INTEGER NOT NULL,
      album_id INTEGER NOT NULL,
      name TEXT NOT NULL,
      path TEXT NOT NULL,
      duration INTEGER NOT NULL,
      artist TEXT NOT NULL,
      album TEXT NOT NULL,
      format TEXT NOT NULL,
      is_video INTEGER NOT NULL,
      cover_path TEXT,
      cover_data TEXT,
      lyrics_path TEXT,
      file_mtime REAL NOT NULL,
      file_size INTEGER NOT NULL,
      meta_title TEXT,
      meta_artist TEXT,
      genre TEXT,
      bitrate INTEGER,
      sample_rate INTEGER,
      album_cover_data TEXT,
      FOREIGN KEY (artist_id) REFERENCES artists(artist_id),
      FOREIGN KEY (album_id) REFERENCES albums(album_id)
    );
    CREATE INDEX IF NOT EXISTS idx_tracks_album_id ON tracks(album_id);
    CREATE INDEX IF NOT EXISTS idx_tracks_artist_id ON tracks(artist_id);
    CREATE INDEX IF NOT EXISTS idx_tracks_path ON tracks(path);

    -- Add columns to existing databases; each ALTER TABLE is wrapped in try-catch
    -- because sql.js exec() throws on any failing statement.
    try { database.exec('ALTER TABLE tracks ADD COLUMN genre TEXT') } catch { /* column may already exist */ }
    try { database.exec('ALTER TABLE tracks ADD COLUMN bitrate INTEGER') } catch { /* column may already exist */ }
    try { database.exec('ALTER TABLE tracks ADD COLUMN sample_rate INTEGER') } catch { /* column may already exist */ }

    CREATE TABLE IF NOT EXISTS folder_nodes (
      path TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      parent_path TEXT,
      track_count INTEGER NOT NULL,
      cover_data TEXT
    );
    CREATE INDEX IF NOT EXISTS idx_folder_nodes_parent_path ON folder_nodes(parent_path);

    CREATE TABLE IF NOT EXISTS folder_tracks (
      folder_path TEXT NOT NULL,
      track_id TEXT NOT NULL,
      PRIMARY KEY (folder_path, track_id),
      FOREIGN KEY (folder_path) REFERENCES folder_nodes(path),
      FOREIGN KEY (track_id) REFERENCES tracks(id)
    );
    CREATE INDEX IF NOT EXISTS idx_folder_tracks_track_id ON folder_tracks(track_id);
  `)
}

function saveToDisk(database: SqlJsDatabase, filePath: string) {
  ensureParentDir(filePath)
  const data = database.export()
  fs.writeFileSync(filePath, Buffer.from(data))
}

function clearSnapshot(database: SqlJsDatabase) {
  database.exec(`
    DELETE FROM folder_tracks;
    DELETE FROM folder_nodes;
    DELETE FROM tracks;
    DELETE FROM albums;
    DELETE FROM artists;
    DELETE FROM library_meta;
  `)
}

function insertMeta(database: SqlJsDatabase, key: string, value: string) {
  const stmt = database.prepare(`INSERT INTO library_meta (key, value) VALUES (?, ?)`)
  stmt.run([key, value])
  stmt.free()
}

export async function saveLibrarySnapshot(filePath: string, snapshot: LibrarySnapshot): Promise<void> {
  const sql = await ensureSql()
  const database = openDatabase(filePath, sql)

  database.exec('BEGIN TRANSACTION')
  try {
    clearSnapshot(database)
    insertMeta(database, 'folderPaths', JSON.stringify(snapshot.folderPaths.map(normalizePath)))
    insertMeta(database, 'fileCount', String(snapshot.fileCount))
    insertMeta(database, 'scannedAt', String(snapshot.scannedAt))

    const insertArtist = database.prepare(`INSERT INTO artists (name, root_path) VALUES (?, ?)`)
    const insertAlbum = database.prepare(`INSERT INTO albums (artist_id, name, artist_name, cover_path, cover_data) VALUES (?, ?, ?, ?, ?)`)
    const insertTrack = database.prepare(`
      INSERT INTO tracks (
        id, artist_id, album_id, name, path, duration, artist, album, format, is_video,
        cover_path, cover_data, lyrics_path, file_mtime, file_size, meta_title, meta_artist,
        genre, bitrate, sample_rate, album_cover_data
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `)
    const insertFolder = database.prepare(`INSERT INTO folder_nodes (path, name, parent_path, track_count, cover_data) VALUES (?, ?, ?, ?, ?)`)
    const insertFolderTrack = database.prepare(`INSERT INTO folder_tracks (folder_path, track_id) VALUES (?, ?)`)

    const folderParentMap = new Map<string, string | null>()
    const queue = [...snapshot.folderTree]
    let qi = 0
    while (qi < queue.length) {
      const node = queue[qi++]
      const normalizedNodePath = normalizePath(node.path)
      for (const child of node.children) {
        folderParentMap.set(normalizePath(child.path), normalizedNodePath)
        queue.push(child)
      }
      if (!folderParentMap.has(normalizedNodePath)) folderParentMap.set(normalizedNodePath, null)
    }

    for (const artist of snapshot.artists) {
      insertArtist.run([artist.name, normalizePath(artist.path)])
      const artistId = Number(database.exec('SELECT last_insert_rowid() AS id')[0].values[0][0])

      for (const album of artist.albums) {
        insertAlbum.run([artistId, album.name, album.artist, album.coverPath, album.coverData])
        const albumId = Number(database.exec('SELECT last_insert_rowid() AS id')[0].values[0][0])

        for (const track of album.tracks) {
          insertTrack.run([
            track.id,
            artistId,
            albumId,
            track.name,
            normalizePath(track.path),
            track.duration,
            track.artist,
            track.album,
            track.format,
            track.isVideo ? 1 : 0,
            track.coverPath,
            track.coverData,
            track.lyricsPath,
            track.fileMtime,
            track.fileSize,
            track.metaTitle,
            track.metaArtist,
            track.genre ?? null,
            track.bitrate ?? null,
            track.sampleRate ?? null,
            album.coverData ?? null,
          ])
        }
      }
    }

    const folderStack = [...snapshot.folderTree]
    let fi = 0
    while (fi < folderStack.length) {
      const node = folderStack[fi++]
      const normalizedNodePath = normalizePath(node.path)
      insertFolder.run([
        normalizedNodePath,
        node.name,
        folderParentMap.get(normalizedNodePath) ?? null,
        node.trackCount,
        node.coverData,
      ])
      for (const track of node.tracks) {
        insertFolderTrack.run([normalizedNodePath, track.id])
      }
      folderStack.push(...node.children)
    }

    database.exec('COMMIT')
    saveToDisk(database, filePath)
  } catch (error) {
    database.exec('ROLLBACK')
    throw error
  } finally {
    try { insertArtist.free() } catch { /* ignore */ }
    try { insertAlbum.free() } catch { /* ignore */ }
    try { insertTrack.free() } catch { /* ignore */ }
    try { insertFolder.free() } catch { /* ignore */ }
    try { insertFolderTrack.free() } catch { /* ignore */ }
  }
}

function getSingleMeta(database: SqlJsDatabase, key: string): string | null {
  const stmt = database.prepare(`SELECT value FROM library_meta WHERE key = ? LIMIT 1`)
  stmt.bind([key])
  const value = stmt.step() ? String(stmt.getAsObject().value) : null
  stmt.free()
  return value
}

function rowsFromStmt<T>(stmt: any, mapper: (row: Record<string, unknown>) => T): T[] {
  const rows: T[] = []
  while (stmt.step()) {
    rows.push(mapper(stmt.getAsObject()))
  }
  stmt.free()
  return rows
}

export async function loadLibrarySnapshot(filePath: string): Promise<LibrarySnapshot | null> {
  const sql = await ensureSql()
  if (!fs.existsSync(filePath)) return null
  const database = openDatabase(filePath, sql)

  const folderPathsRaw = getSingleMeta(database, 'folderPaths')
  if (!folderPathsRaw) return null

  const tracksStmt = database.prepare(`
    SELECT
      t.id, t.name, t.path, t.duration, t.artist, t.album, t.format, t.is_video,
      t.cover_path, t.cover_data, t.lyrics_path, t.file_mtime, t.file_size,
      t.meta_title, t.meta_artist, t.genre, t.bitrate, t.sample_rate,
      t.album_cover_data, t.album_id, t.artist_id
    FROM tracks t
    ORDER BY t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.name COLLATE NOCASE
  `)
  const trackRows = rowsFromStmt(tracksStmt, (row) => ({
    id: String(row.id),
    name: String(row.name),
    path: String(row.path),
    duration: Number(row.duration) || 0,
    artist: String(row.artist),
    album: String(row.album),
    format: String(row.format),
    isVideo: Number(row.is_video) === 1,
    coverPath: row.cover_path ? String(row.cover_path) : null,
    coverData: row.cover_data ? String(row.cover_data) : null,
    lyricsPath: row.lyrics_path ? String(row.lyrics_path) : null,
    fileMtime: Number(row.file_mtime) || 0,
    fileSize: Number(row.file_size) || 0,
    metaTitle: row.meta_title ? String(row.meta_title) : null,
    metaArtist: row.meta_artist ? String(row.meta_artist) : null,
    genre: row.genre ? String(row.genre) : null,
    bitrate: row.bitrate ? Number(row.bitrate) : null,
    sampleRate: row.sample_rate ? Number(row.sample_rate) : null,
    albumCoverData: row.album_cover_data ? String(row.album_cover_data) : null,
    albumId: Number(row.album_id),
    artistId: Number(row.artist_id),
  }))

  const albumsStmt = database.prepare(`
    SELECT album_id, artist_id, name, artist_name, cover_path, cover_data
    FROM albums
    ORDER BY artist_name COLLATE NOCASE, name COLLATE NOCASE
  `)
  const albumRows = rowsFromStmt(albumsStmt, (row) => ({
    albumId: Number(row.album_id),
    artistId: Number(row.artist_id),
    name: String(row.name),
    artist: String(row.artist_name),
    coverPath: row.cover_path ? String(row.cover_path) : null,
    coverData: row.cover_data ? String(row.cover_data) : null,
  }))

  const artistsStmt = database.prepare(`
    SELECT artist_id, name, root_path
    FROM artists
    ORDER BY name COLLATE NOCASE
  `)
  const artistRows = rowsFromStmt(artistsStmt, (row) => ({
    artistId: Number(row.artist_id),
    name: String(row.name),
    path: String(row.root_path),
  }))

  const tracksByAlbum = new Map<number, TrackRecord[]>()
  const tracksById = new Map<string, TrackRecord>()
  for (const row of trackRows) {
    const track: TrackRecord = {
      id: row.id,
      name: row.name,
      path: row.path,
      duration: row.duration,
      artist: row.artist,
      album: row.album,
      format: row.format,
      isVideo: row.isVideo,
      coverPath: row.coverPath,
      coverData: row.coverData,
      lyricsPath: row.lyricsPath,
      fileMtime: row.fileMtime,
      fileSize: row.fileSize,
      metaTitle: row.metaTitle,
      metaArtist: row.metaArtist,
      genre: row.genre,
      bitrate: row.bitrate,
      sampleRate: row.sampleRate,
      albumCoverData: row.albumCoverData,
    }
    if (!tracksByAlbum.has(row.albumId)) tracksByAlbum.set(row.albumId, [])
    tracksByAlbum.get(row.albumId)!.push(track)
    tracksById.set(track.id, track)
  }

  const albumsByArtist = new Map<number, AlbumRecord[]>()
  for (const album of albumRows) {
    const record: AlbumRecord = {
      name: album.name,
      artist: album.artist,
      coverPath: album.coverPath,
      coverData: album.coverData,
      tracks: tracksByAlbum.get(album.albumId) ?? [],
    }
    if (!albumsByArtist.has(album.artistId)) albumsByArtist.set(album.artistId, [])
    albumsByArtist.get(album.artistId)!.push(record)
  }

  const artists: ArtistRecord[] = artistRows.map((artist) => ({
    name: artist.name,
    path: artist.path,
    albums: albumsByArtist.get(artist.artistId) ?? [],
  }))

  const folderNodesStmt = database.prepare(`
    SELECT path, name, parent_path, track_count, cover_data
    FROM folder_nodes
    ORDER BY path COLLATE NOCASE
  `)
  const folderRows = rowsFromStmt(folderNodesStmt, (row) => ({
    path: String(row.path),
    name: String(row.name),
    parentPath: row.parent_path ? String(row.parent_path) : null,
    trackCount: Number(row.track_count) || 0,
    coverData: row.cover_data ? String(row.cover_data) : null,
  }))

  const folderTracksStmt = database.prepare(`
    SELECT folder_path, track_id
    FROM folder_tracks
    ORDER BY folder_path COLLATE NOCASE
  `)
  const folderTrackRows = rowsFromStmt(folderTracksStmt, (row) => ({
    folderPath: String(row.folder_path),
    trackId: String(row.track_id),
  }))

  const folderTrackMap = new Map<string, TrackRecord[]>()
  for (const row of folderTrackRows) {
    const track = tracksById.get(row.trackId)
    if (!track) continue
    if (!folderTrackMap.has(row.folderPath)) folderTrackMap.set(row.folderPath, [])
    folderTrackMap.get(row.folderPath)!.push(track)
  }

  const nodeMap = new Map<string, FolderNodeRecord>()
  for (const row of folderRows) {
    nodeMap.set(row.path, {
      name: row.name,
      path: row.path,
      children: [],
      tracks: folderTrackMap.get(row.path) ?? [],
      trackCount: row.trackCount,
      coverData: row.coverData,
    })
  }

  const folderTree: FolderNodeRecord[] = []
  for (const row of folderRows) {
    const node = nodeMap.get(row.path)!
    if (row.parentPath && nodeMap.has(row.parentPath)) {
      nodeMap.get(row.parentPath)!.children.push(node)
    } else {
      folderTree.push(node)
    }
  }

  const allTracks = trackRows.map((row) => tracksById.get(row.id)!).filter(Boolean)
  const fileCount = Number(getSingleMeta(database, 'fileCount') || allTracks.length)
  const scannedAt = Number(getSingleMeta(database, 'scannedAt') || 0)

  let parsedFolderPaths: string[]
  try {
    parsedFolderPaths = JSON.parse(folderPathsRaw)
  } catch {
    return null
  }

  return {
    folderPaths: parsedFolderPaths,
    artists,
    folderTree,
    allTracks,
    fileCount,
    scannedAt,
  }
}

export async function loadTrackMetadataIndex(filePath: string): Promise<Map<string, {
  duration: number
  coverData: string | null
  title: string | null
  artist: string | null
  fileMtime: number
  fileSize: number
  genre: string | null
  bitrate: number | null
  sampleRate: number | null
}>> {
  const sql = await ensureSql()
  if (!fs.existsSync(filePath)) return new Map()
  const database = openDatabase(filePath, sql)

  const stmt = database.prepare(`
    SELECT path, duration, cover_data, meta_title, meta_artist, file_mtime, file_size,
           genre, bitrate, sample_rate
    FROM tracks
  `)

  const rows = rowsFromStmt(stmt, (row) => ({
    path: String(row.path),
    duration: Number(row.duration) || 0,
    coverData: row.cover_data ? String(row.cover_data) : null,
    title: row.meta_title ? String(row.meta_title) : null,
    artist: row.meta_artist ? String(row.meta_artist) : null,
    fileMtime: Number(row.file_mtime) || 0,
    fileSize: Number(row.file_size) || 0,
    genre: row.genre ? String(row.genre) : null,
    bitrate: row.bitrate ? Number(row.bitrate) : null,
    sampleRate: row.sample_rate ? Number(row.sample_rate) : null,
  }))

  const index = new Map<string, {
    duration: number
    coverData: string | null
    title: string | null
    artist: string | null
    fileMtime: number
    fileSize: number
    genre: string | null
    bitrate: number | null
    sampleRate: number | null
  }>()

  for (const row of rows) index.set(row.path, row)
  return index
}
