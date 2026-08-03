var __create = Object.create;
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getProtoOf = Object.getPrototypeOf;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __copyProps = (to, from, except, desc) => {
  if (from && typeof from === "object" || typeof from === "function") {
    for (let key of __getOwnPropNames(from))
      if (!__hasOwnProp.call(to, key) && key !== except)
        __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
  }
  return to;
};
var __toESM = (mod, isNodeMode, target) => (target = mod != null ? __create(__getProtoOf(mod)) : {}, __copyProps(
  // If the importer is in node compatibility mode or this is not an ESM
  // file that has been converted to a CommonJS file using a Babel-
  // compatible transform (i.e. "__esModule" has not been set), then set
  // "default" to the CommonJS "module.exports" for node compatibility.
  isNodeMode || !mod || !mod.__esModule ? __defProp(target, "default", { value: mod, enumerable: true }) : target,
  mod
));

// KX-Player/electron/main.ts
var import_electron = require("electron");
var import_node_path5 = __toESM(require("node:path"));
var import_node_fs5 = __toESM(require("node:fs"));
var import_promises2 = __toESM(require("node:fs/promises"));
var import_node_child_process = require("node:child_process");

// KX-Player/electron/fileScanner.ts
var import_node_fs2 = __toESM(require("node:fs"));
var import_node_path2 = __toESM(require("node:path"));
var import_node_os = __toESM(require("node:os"));
var import_node_crypto2 = __toESM(require("node:crypto"));
var import_node_worker_threads = require("node:worker_threads");
var import_chokidar = __toESM(require("chokidar"));
var musicMetadata = __toESM(require("music-metadata"));

// KX-Player/electron/coverService.ts
var import_node_fs = __toESM(require("node:fs"));
var import_promises = __toESM(require("node:fs/promises"));
var import_node_path = __toESM(require("node:path"));
var import_node_crypto = __toESM(require("node:crypto"));
var import_sharp = __toESM(require("sharp"));
var COVER_EXTS = /* @__PURE__ */ new Set([".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif"]);
var COVER_NAMES = ["cover", "folder", "front", "albumart", "album", "art", "jacket", "\u5C01\u9762", "\u4E13\u8F91\u5C01\u9762", "\u4E13\u8F91\u56FE", "\u30B8\u30E3\u30B1\u30C3\u30C8"];
var NON_COVER_HINTS = ["ui", "\u8BF4\u660E", "screenshot", "screen", "manual", "readme", "player", "capture", "shot", "ss", "banner", "icon", "thumb", "thumbnail", "small", "icon"];
var _coversDir = "";
function initCoverDir(userDataDir) {
  _coversDir = import_node_path.default.join(userDataDir, "covers");
  if (!import_node_fs.default.existsSync(_coversDir)) import_node_fs.default.mkdirSync(_coversDir, { recursive: true });
}
function coversDir() {
  if (!_coversDir) throw new Error("coverService not initialized");
  return _coversDir;
}
function getCoversDir() {
  return coversDir();
}
async function compressToJpeg(input) {
  if (input.length < 10 * 1024) return input;
  try {
    return await (0, import_sharp.default)(input).resize(400, 400, { fit: "inside", withoutEnlargement: true }).jpeg({ quality: 70 }).toBuffer();
  } catch {
    return input;
  }
}
async function saveTrackCover(trackId, dataUrl) {
  if (!dataUrl) return false;
  try {
    const match = dataUrl.match(/^data:image\/\w+;base64,(.+)$/);
    if (!match) {
      console.warn("[cover] saveTrackCover regex mismatch for", trackId, dataUrl.slice(0, 60));
      return false;
    }
    const buffer = Buffer.from(match[1], "base64");
    const compressed = await compressToJpeg(buffer);
    const filePath = import_node_path.default.join(coversDir(), `${trackId}.jpg`);
    import_node_fs.default.writeFileSync(filePath, compressed);
    return true;
  } catch (e) {
    console.error("[cover] saveTrackCover failed for", trackId, ":", e?.message || e);
    return false;
  }
}
async function saveFolderCover(folderPath, dataUrl) {
  if (!dataUrl) return false;
  try {
    const match = dataUrl.match(/^data:image\/\w+;base64,(.+)$/);
    if (!match) return false;
    const buffer = Buffer.from(match[1], "base64");
    const compressed = await compressToJpeg(buffer);
    const hash = import_node_crypto.default.createHash("md5").update(folderPath).digest("hex");
    const filePath = import_node_path.default.join(coversDir(), `folder_${hash}.jpg`);
    import_node_fs.default.writeFileSync(filePath, compressed);
    return true;
  } catch {
    return false;
  }
}
async function saveExternalCover(folderPath, extFilePath) {
  try {
    const buffer = import_node_fs.default.readFileSync(extFilePath);
    const compressed = await compressToJpeg(buffer);
    const hash = import_node_crypto.default.createHash("md5").update(folderPath).digest("hex");
    const filePath = import_node_path.default.join(coversDir(), `folder_${hash}.jpg`);
    import_node_fs.default.writeFileSync(filePath, compressed);
    return true;
  } catch {
    return false;
  }
}
function getTrackCoverPath(trackId) {
  const p = import_node_path.default.join(coversDir(), `${trackId}.jpg`);
  return import_node_fs.default.existsSync(p) ? p : null;
}
function findExternalCoverInDir(dirPath) {
  try {
    const entries = import_node_fs.default.readdirSync(dirPath);
    for (const name of COVER_NAMES) {
      for (const ext of ["jpg", "jpeg", "png", "webp", "bmp"]) {
        const p = import_node_path.default.join(dirPath, `${name}.${ext}`);
        if (import_node_fs.default.existsSync(p)) {
          try {
            const stat = import_node_fs.default.statSync(p);
            if (stat.size > 0 && stat.size < 15 * 1024 * 1024) return p;
          } catch {
          }
        }
      }
    }
    for (const entry of entries) {
      const ext = import_node_path.default.extname(entry).toLowerCase();
      if (!COVER_EXTS.has(ext)) continue;
      const name = entry.toLowerCase();
      const isNonCover = NON_COVER_HINTS.some((h) => name.includes(h));
      if (isNonCover) continue;
      const p = import_node_path.default.join(dirPath, entry);
      try {
        const stat = import_node_fs.default.statSync(p);
        if (stat.size > 0 && stat.size < 15 * 1024 * 1024) return p;
      } catch {
      }
    }
  } catch {
  }
  return null;
}
var _folderCoverMap = {};
var _folderCoverMapPath = "";
function loadFolderCoverMap(userDataDir) {
  _folderCoverMapPath = import_node_path.default.join(userDataDir, "folder-cover-map.json");
  try {
    if (import_node_fs.default.existsSync(_folderCoverMapPath)) {
      _folderCoverMap = JSON.parse(import_node_fs.default.readFileSync(_folderCoverMapPath, "utf-8"));
    }
  } catch {
    _folderCoverMap = {};
  }
}
function saveFolderCoverMap() {
  try {
    import_node_fs.default.writeFileSync(_folderCoverMapPath, JSON.stringify(_folderCoverMap), "utf-8");
  } catch {
  }
}
function setFolderCoverMapping(folderPath) {
  _folderCoverMap[folderPath] = import_node_crypto.default.createHash("md5").update(folderPath).digest("hex");
  saveFolderCoverMap();
}
async function getTrackCoversBatchAsync(trackIds) {
  const result = {};
  await Promise.all(trackIds.map(async (id) => {
    const p = getTrackCoverPath(id);
    if (!p) return;
    try {
      const buffer = await import_promises.default.readFile(p);
      result[id] = `data:image/jpeg;base64,${buffer.toString("base64")}`;
    } catch {
    }
  }));
  return result;
}
async function getFolderCoversBatchAsync(folderPaths) {
  const result = {};
  await Promise.all(folderPaths.map(async (folderPath) => {
    const hash = _folderCoverMap[folderPath];
    if (!hash) return;
    const p = import_node_path.default.join(coversDir(), `folder_${hash}.jpg`);
    try {
      const buffer = await import_promises.default.readFile(p);
      result[folderPath] = `data:image/jpeg;base64,${buffer.toString("base64")}`;
    } catch {
    }
  }));
  return result;
}
async function getAllFolderCoversFromMapAsync() {
  const entries = Object.entries(_folderCoverMap);
  const result = {};
  await Promise.all(entries.map(async ([folderPath, hash]) => {
    const p = import_node_path.default.join(coversDir(), `folder_${hash}.jpg`);
    try {
      const buffer = await import_promises.default.readFile(p);
      result[folderPath] = `data:image/jpeg;base64,${buffer.toString("base64")}`;
    } catch {
    }
  }));
  return result;
}

// KX-Player/electron/fileScanner.ts
var AUDIO_EXTS = /* @__PURE__ */ new Set([".mp3", ".flac", ".wav", ".ogg", ".m4a", ".aac", ".wma", ".opus", ".ape", ".wv", ".aiff", ".alac"]);
var VIDEO_EXTS = /* @__PURE__ */ new Set([".mp4", ".mkv", ".avi", ".mov", ".webm", ".flv", ".wmv"]);
var ALL_EXTS = /* @__PURE__ */ new Set([...AUDIO_EXTS, ...VIDEO_EXTS]);
function longPath(p) {
  if (process.platform !== "win32") return p;
  if (p.length > 240 && !p.startsWith("\\\\?\\")) {
    return "\\\\?\\" + p.replace(/\//g, "\\");
  }
  return p;
}
async function discoverFiles(folderPaths) {
  const results = [];
  const visited = /* @__PURE__ */ new Set();
  async function walk(dirPath) {
    const lp = longPath(dirPath);
    let entries = [];
    try {
      entries = await import_node_fs2.default.promises.readdir(lp, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const fullPath = import_node_path2.default.join(dirPath, entry.name);
      if (entry.isDirectory()) {
        if (!visited.has(fullPath) && visited.size < 1e4) {
          visited.add(fullPath);
          await walk(fullPath);
        }
      } else if (entry.isFile() || entry.isSymbolicLink()) {
        const ext = import_node_path2.default.extname(entry.name).toLowerCase();
        if (ALL_EXTS.has(ext)) {
          results.push(fullPath);
        }
      }
    }
  }
  for (const fp of folderPaths) {
    visited.add(fp);
    await walk(fp);
  }
  return results;
}
var SCAN_TIMEOUT_MS = 15e3;
var LARGE_FILE_SCAN_TIMEOUT_MS = 3e4;
var CHOKIDAR_DELAY = 1e3;
var YIELD_INTERVAL = 500;
var COVER_FILE_EXTS = /* @__PURE__ */ new Set([".jpg", ".jpeg", ".png", ".webp", ".bmp", ".gif"]);
var MAX_COVER_BYTES = 15 * 1024 * 1024;
var COVER_NAME_HINTS = ["cover", "folder", "front", "albumart", "album", "art", "jacket", "\u30B8\u30E3\u30B1\u30C3\u30C8", "\u5C01\u9762", "\u4E13\u8F91\u5C01\u9762", "\u4E13\u8F91\u56FE"];
var NON_COVER_HINTS2 = ["ui", "\u7487\u5B58\u69D1", "screenshot", "screen", "manual", "readme", "player", "capture", "shot", "ss", "banner", "icon", "thumb", "thumbnail", "small", "icon"];
function coverToBase64(filePath) {
  try {
    const data = import_node_fs2.default.readFileSync(filePath);
    if (!data.length || data.length > MAX_COVER_BYTES) return null;
    const ext = import_node_path2.default.extname(filePath).slice(1).toLowerCase();
    return `data:image/${ext === "jpg" ? "jpeg" : ext};base64,${data.toString("base64")}`;
  } catch {
    return null;
  }
}
function getImageDimensions(filePath) {
  try {
    const fd = import_node_fs2.default.openSync(filePath, "r");
    try {
      const head = Buffer.alloc(32);
      import_node_fs2.default.readSync(fd, head, 0, 32, 0);
      const ext = import_node_path2.default.extname(filePath).toLowerCase();
      if (ext === ".png") {
        if (head.toString("ascii", 1, 4) === "PNG") {
          return { width: head.readUInt32BE(16), height: head.readUInt32BE(20) };
        }
      } else if (ext === ".jpg" || ext === ".jpeg") {
        let i = 2;
        while (i < 65536) {
          const buf = Buffer.alloc(16);
          import_node_fs2.default.readSync(fd, buf, 0, 16, i);
          if (buf[0] !== 255) {
            i++;
            continue;
          }
          const marker = buf[1];
          if (marker === 217 || marker === 216) {
            i += 2;
            continue;
          }
          const len = buf.readUInt16BE(2);
          if (marker >= 192 && marker <= 207 && marker !== 196 && marker !== 200 && marker !== 204) {
            return { height: buf.readUInt16BE(5), width: buf.readUInt16BE(7) };
          }
          i += 2 + len;
          if (len < 2) break;
        }
      } else if (ext === ".webp") {
        if (head.toString("ascii", 0, 4) === "RIFF" && head.toString("ascii", 8, 12) === "WEBP") {
          const chunk = head.toString("ascii", 12, 16);
          if (chunk === "VP8 ") {
            return { width: head.readUInt16LE(26) & 16383, height: head.readUInt16LE(28) & 16383 };
          } else if (chunk === "VP8L") {
            const bits = head.readUInt32LE(21);
            return { width: (bits & 16383) + 1, height: (bits >> 14 & 16383) + 1 };
          } else if (chunk === "VP8X") {
            return { width: head.readUInt24BE(24) + 1, height: head.readUInt24BE(27) + 1 };
          }
        }
      } else if (ext === ".bmp") {
        return { width: head.readUInt32LE(18), height: Math.abs(head.readInt32LE(22)) };
      } else if (ext === ".gif") {
        return { width: head.readUInt16LE(6), height: head.readUInt16LE(8) };
      }
    } finally {
      import_node_fs2.default.closeSync(fd);
    }
  } catch {
  }
  return null;
}
function scoreCoverCandidate(filePath, depth) {
  const name = import_node_path2.default.basename(filePath).toLowerCase();
  const ext = import_node_path2.default.extname(filePath).toLowerCase();
  const stat = import_node_fs2.default.statSync(filePath);
  const sizeKB = stat.size / 1024;
  for (const hint of NON_COVER_HINTS2) {
    if (name.includes(hint)) return -1e3;
  }
  let score = 0;
  for (const hint of COVER_NAME_HINTS) {
    if (name.includes(hint)) score += 100;
  }
  if (ext === ".jpg" || ext === ".jpeg") score += 10;
  if (ext === ".png") score += 5;
  score -= depth * 30;
  const dims = getImageDimensions(filePath);
  if (dims) {
    const ratio = dims.width / dims.height;
    if (ratio >= 1.2 && ratio <= 1.5) score += 60;
    else if (ratio >= 0.9 && ratio <= 1.1) score += 40;
    else if (ratio > 2.5 || ratio < 0.4) score -= 50;
    const longSide = Math.max(dims.width, dims.height);
    if (longSide >= 400 && longSide <= 1200) score += 20;
    else if (longSide > 1600) score -= 20;
  } else {
    if (sizeKB >= 30 && sizeKB <= 600) score += 10;
    else if (sizeKB > 1e3) score -= 10;
  }
  return score;
}
function findExternalCover(dirPath, maxDepth = 1) {
  let bestPath = null;
  let bestScore = -Infinity;
  function scan(currentDir, depth) {
    if (depth > maxDepth) return;
    let entries = [];
    try {
      entries = import_node_fs2.default.readdirSync(currentDir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      if (entry.isDirectory()) {
        scan(import_node_path2.default.join(currentDir, entry.name), depth + 1);
        continue;
      }
      const ext = import_node_path2.default.extname(entry.name).toLowerCase();
      if (!COVER_FILE_EXTS.has(ext)) continue;
      const filePath = import_node_path2.default.join(currentDir, entry.name);
      const score = scoreCoverCandidate(filePath, depth);
      if (score > bestScore) {
        bestScore = score;
        bestPath = filePath;
      }
    }
  }
  scan(dirPath, 0);
  if (!bestPath) return null;
  if (bestScore < 0) return null;
  return coverToBase64(bestPath);
}
function findAnyImage(dirPath) {
  let entries = [];
  try {
    entries = import_node_fs2.default.readdirSync(dirPath, { withFileTypes: true });
  } catch {
    return null;
  }
  let bestPath = null;
  let bestSize = 0;
  for (const entry of entries) {
    if (entry.isDirectory()) continue;
    const ext = import_node_path2.default.extname(entry.name).toLowerCase();
    if (!COVER_FILE_EXTS.has(ext)) continue;
    const filePath = import_node_path2.default.join(dirPath, entry.name);
    const score = scoreCoverCandidate(filePath, 0);
    if (score > -100 && score > bestSize) {
      bestSize = score;
      bestPath = filePath;
    }
  }
  if (!bestPath) return null;
  return coverToBase64(bestPath);
}
var watchers = [];
var onChangeCallback = null;
function hashPath(p) {
  return import_node_crypto2.default.createHash("md5").update(p).digest("hex").slice(0, 12);
}
function yieldToEventLoop() {
  return new Promise((resolve) => setImmediate(resolve));
}
function normalizeName(filename) {
  let name = import_node_path2.default.basename(filename, import_node_path2.default.extname(filename));
  name = name.replace(/^[\d]+[\s.\-_]+/, "").replace(/[_\-]/g, " ").trim();
  return name || import_node_path2.default.basename(filename);
}
function throttleProgress(callback, total) {
  let lastReported = -1;
  let timer = null;
  return (completed) => {
    if (completed === total || completed - lastReported >= Math.max(1, Math.floor(total * 0.02)) || lastReported < 0) {
      if (timer) {
        clearTimeout(timer);
        timer = null;
      }
      lastReported = completed;
      callback(completed, total);
    } else if (!timer) {
      timer = setTimeout(() => {
        timer = null;
        lastReported = completed;
        callback(completed, total);
      }, 120);
    }
  };
}
var _workerPool = [];
var _workerPath = null;
function _getWorkerPath() {
  if (_workerPath) return _workerPath;
  const candidates = [
    import_node_path2.default.join(__dirname, "workers", "metadata-worker.js"),
    import_node_path2.default.join(__dirname, "..", "dist-electron", "workers", "metadata-worker.js"),
    import_node_path2.default.join(process.cwd(), "dist-electron", "workers", "metadata-worker.js")
  ];
  for (const p of candidates) {
    if (import_node_fs2.default.existsSync(p)) {
      _workerPath = p;
      return p;
    }
  }
  return null;
}
function _terminateWorkerPool() {
  if (_workerPool.length === 0) return;
  for (const w of _workerPool) {
    try {
      w.terminate();
    } catch {
    }
  }
  _workerPool = [];
}
function distributeFilesBySize(files, workerCount) {
  if (files.length === 0) return [];
  const effectiveWorkers = Math.min(workerCount, files.length);
  const chunks = Array.from({ length: effectiveWorkers }, () => []);
  if (files.length > 5e3) {
    for (let i = 0; i < files.length; i++) {
      chunks[i % effectiveWorkers].push(files[i]);
    }
    return chunks.filter((c) => c.length > 0);
  }
  const sizes = new Float64Array(effectiveWorkers);
  const fileSizes = [];
  for (const f of files) {
    try {
      fileSizes.push({ path: f, size: import_node_fs2.default.statSync(f).size });
    } catch {
      fileSizes.push({ path: f, size: 0 });
    }
  }
  fileSizes.sort((a, b) => b.size - a.size);
  for (const { path: filePath, size } of fileSizes) {
    let minIdx = 0;
    for (let i = 1; i < effectiveWorkers; i++) {
      if (sizes[i] < sizes[minIdx]) minIdx = i;
    }
    chunks[minIdx].push(filePath);
    sizes[minIdx] += size;
  }
  return chunks.filter((c) => c.length > 0);
}
async function enrichWithWorkers(files, existingMeta = /* @__PURE__ */ new Map(), onProgress) {
  const results = /* @__PURE__ */ new Map();
  const normalFiles = [];
  const total = files.length;
  let completed = 0;
  const reportProgress = onProgress ? throttleProgress(onProgress, total) : () => {
  };
  console.time("[scan] cacheCheck");
  for (const [i, filePath] of files.entries()) {
    if (i > 0 && i % YIELD_INTERVAL === 0) await yieldToEventLoop();
    const stat = getFileStat(filePath);
    const cached = existingMeta.get(filePath.replace(/\\/g, "/"));
    const ext = import_node_path2.default.extname(filePath).toLowerCase();
    const shouldRefreshZeroVideoDuration = cached && VIDEO_EXTS.has(ext) && (cached.duration || 0) <= 0;
    if (stat && cached && !shouldRefreshZeroVideoDuration && cached.fileMtime === stat.mtime && cached.fileSize === stat.size) {
      results.set(filePath, {
        duration: cached.duration,
        coverData: null,
        // Cover preserved by saveLibrarySnapshot's DB fallback
        title: cached.title,
        artist: cached.artist,
        genre: cached.genre,
        bitrate: cached.bitrate,
        sampleRate: cached.sampleRate
      });
      completed += 1;
      reportProgress(completed);
    } else {
      normalFiles.push(filePath);
    }
  }
  console.timeEnd("[scan] cacheCheck");
  if (normalFiles.length === 0) {
    reportProgress(total);
    return results;
  }
  const physicalCores = Math.max(1, import_node_os.default.cpus().length - 1);
  const cpuCount = Math.min(16, Math.max(2, Math.ceil(physicalCores * 1.5)));
  const chunks = distributeFilesBySize(normalFiles, cpuCount);
  const LARGE_FILE_SIZE = 100 * 1024 * 1024;
  function hasLargeFiles(chunk) {
    return chunk.some((f) => {
      try {
        return import_node_fs2.default.statSync(f).size > LARGE_FILE_SIZE;
      } catch {
        return false;
      }
    });
  }
  const workerPath = _getWorkerPath();
  if (!workerPath) {
    for (const f of normalFiles) {
      results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null });
    }
    completed += normalFiles.length;
    reportProgress(total);
    return results;
  }
  _terminateWorkerPool();
  console.time("[scan] workerCreate");
  for (let i = 0; i < chunks.length; i++) {
    try {
      const perFileTimeout = hasLargeFiles(chunks[i]) ? LARGE_FILE_SCAN_TIMEOUT_MS : SCAN_TIMEOUT_MS;
      const w = new import_node_worker_threads.Worker(workerPath, { workerData: { files: chunks[i], timeoutMs: perFileTimeout } });
      _workerPool.push(w);
    } catch (e) {
      console.error("[scan] worker creation failed:", e);
      break;
    }
  }
  console.timeEnd("[scan] workerCreate");
  console.time("[scan] workerBatch");
  const workerPromises = chunks.map((chunk, idx) => {
    return new Promise((resolve) => {
      const worker = _workerPool[idx];
      if (!worker) {
        for (const f of chunk) {
          results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null });
        }
        completed += chunk.length;
        reportProgress(completed);
        resolve();
        return;
      }
      const chunkTimeout = Math.max(1e4, Math.min(6e5, chunk.length * SCAN_TIMEOUT_MS));
      let hasResponded = false;
      let chunkTimer = null;
      const clearChunkTimer = () => {
        if (chunkTimer) {
          clearTimeout(chunkTimer);
          chunkTimer = null;
        }
      };
      const messageHandler = (msg) => {
        if (msg.type === "progress") {
          clearChunkTimer();
          chunkTimer = setTimeout(handleTimeout, chunkTimeout);
          return;
        }
        if (hasResponded) return;
        hasResponded = true;
        clearChunkTimer();
        worker.removeListener("message", messageHandler);
        worker.removeListener("error", errorHandler);
        if (msg.type === "result") {
          for (const r of msg.results) {
            results.set(r.path, {
              duration: r.duration || 0,
              coverData: r.coverB64 || null,
              title: r.title || null,
              artist: r.artist || null,
              genre: r.genre || null,
              bitrate: r.bitrate || null,
              sampleRate: r.sampleRate || null
            });
          }
          completed += chunk.length;
          reportProgress(completed);
          resolve();
        } else if (msg.type === "error") {
          console.error("[scan] worker reported error:", msg.message);
          for (const f of chunk) {
            results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null });
          }
          completed += chunk.length;
          reportProgress(completed);
          resolve();
        }
      };
      worker.on("message", messageHandler);
      const errorHandler = (err) => {
        console.error("[scan] worker error:", err?.message || err);
        clearChunkTimer();
        worker.removeListener("message", messageHandler);
        worker.removeListener("error", errorHandler);
        if (!hasResponded) {
          hasResponded = true;
          for (const f of chunk) {
            results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null });
          }
          completed += chunk.length;
          reportProgress(completed);
        }
        resolve();
      };
      worker.on("error", errorHandler);
      const handleTimeout = () => {
        if (!hasResponded) {
          console.warn("[scan] worker timeout for chunk", idx, "size", chunk.length);
          hasResponded = true;
          worker.removeListener("message", messageHandler);
          worker.removeListener("error", errorHandler);
          try {
            worker.terminate();
          } catch {
          }
          _workerPool = _workerPool.filter((w) => w !== worker);
          for (const f of chunk) {
            results.set(f, { duration: 0, coverData: null, title: null, artist: null, genre: null, bitrate: null, sampleRate: null });
          }
          completed += chunk.length;
          reportProgress(completed);
          resolve();
        }
      };
      chunkTimer = setTimeout(handleTimeout, chunkTimeout);
    });
  });
  await Promise.all(workerPromises);
  console.timeEnd("[scan] workerBatch");
  reportProgress(total);
  _terminateWorkerPool();
  return results;
}
function getFileStat(filePath) {
  try {
    const st = import_node_fs2.default.statSync(filePath);
    return { mtime: st.mtimeMs, size: st.size };
  } catch {
    return null;
  }
}
async function groupTracksByFolder(files, metaResults, rootPaths) {
  const artistMap = /* @__PURE__ */ new Map();
  const normalizedRoots = rootPaths.map((rp) => rp.replace(/\\/g, "/").replace(/\/+$/, ""));
  for (const [fi, fp] of files.entries()) {
    if (fi > 0 && fi % YIELD_INTERVAL === 0) await yieldToEventLoop();
    const meta = metaResults.get(fp);
    if (!meta) continue;
    const st = getFileStat(fp);
    if (!st) continue;
    const nfp = fp.replace(/\\/g, "/");
    let matchedRoot = null;
    for (let ri = 0; ri < normalizedRoots.length; ri++) {
      const nrp = normalizedRoots[ri];
      if (nfp === nrp || nfp.startsWith(nrp + "/")) {
        matchedRoot = nrp;
        break;
      }
    }
    if (!matchedRoot) continue;
    const rel = import_node_path2.default.relative(matchedRoot, fp);
    const parts = rel.split(import_node_path2.default.sep);
    let artistName = import_node_path2.default.basename(matchedRoot);
    let albumName;
    if (parts.length >= 2) {
      albumName = parts[0];
    } else {
      albumName = artistName;
    }
    if (meta.artist && meta.artist.trim()) {
      artistName = meta.artist.trim();
    }
    if (!artistMap.has(artistName)) {
      artistMap.set(artistName, { path: matchedRoot, albums: /* @__PURE__ */ new Map() });
    }
    const artist = artistMap.get(artistName);
    if (!artist.albums.has(albumName)) {
      const albumDirPath = parts.length >= 1 ? import_node_path2.default.join(matchedRoot, parts[0]) : matchedRoot;
      artist.albums.set(albumName, {
        name: albumName,
        artist: artistName,
        dirPath: albumDirPath,
        coverPath: null,
        coverData: null,
        tracks: []
      });
    }
    const album = artist.albums.get(albumName);
    const trackExt = import_node_path2.default.extname(fp).toLowerCase();
    album.tracks.push({
      id: hashPath(fp),
      name: meta.title && meta.title.trim() ? meta.title.trim() : normalizeName(fp),
      path: nfp,
      duration: meta.duration,
      artist: meta.artist && meta.artist.trim() ? meta.artist.trim() : "\u6D63\u6C2C\u6095",
      album: albumName,
      format: trackExt.replace(".", ""),
      isVideo: VIDEO_EXTS.has(trackExt),
      coverPath: null,
      coverData: meta.coverData,
      lyricsPath: null,
      fileMtime: st.mtime,
      fileSize: st.size,
      metaTitle: meta.title,
      metaArtist: meta.artist,
      genre: meta.genre || null,
      bitrate: meta.bitrate || null,
      sampleRate: meta.sampleRate || null
    });
  }
  for (const [, artist] of artistMap) {
    for (const [, album] of artist.albums) {
      for (const track of album.tracks) {
        const metaResult = metaResults.get(track.path);
        if (metaResult?.coverData) {
          album.coverData = metaResult.coverData;
          break;
        }
      }
      if (!album.coverData && album.dirPath) {
        album.coverData = findExternalCover(album.dirPath);
      }
    }
  }
  return [...artistMap.entries()].map(([name, a]) => ({
    name,
    path: a.path,
    albums: [...a.albums.values()]
  }));
}
async function buildFolderTree(files, metaResults, rootPaths) {
  const nodeMap = /* @__PURE__ */ new Map();
  const cleanRoots = rootPaths.map((rp) => rp.replace(/\\/g, "/").replace(/\/+$/, ""));
  const roots = [];
  function getOrCreateNode(dirPath, dirName) {
    if (nodeMap.has(dirPath)) return nodeMap.get(dirPath);
    const node = { name: dirName, path: dirPath, children: [], tracks: [], trackCount: 0, coverData: null };
    nodeMap.set(dirPath, node);
    return node;
  }
  for (const [fi, fp] of files.entries()) {
    if (fi > 0 && fi % YIELD_INTERVAL === 0) await yieldToEventLoop();
    const meta = metaResults.get(fp);
    if (!meta) continue;
    const st = getFileStat(fp);
    if (!st) continue;
    const nfp = fp.replace(/\\/g, "/");
    let matchedRoot = null;
    for (let ri = 0; ri < cleanRoots.length; ri++) {
      if (nfp === cleanRoots[ri] || nfp.startsWith(cleanRoots[ri] + "/")) {
        matchedRoot = cleanRoots[ri];
        break;
      }
    }
    if (!matchedRoot) continue;
    const dir = import_node_path2.default.dirname(fp).replace(/\\/g, "/");
    const dirName = import_node_path2.default.basename(dir);
    const parentDir = import_node_path2.default.dirname(dir).replace(/\\/g, "/");
    const node = getOrCreateNode(dir, dirName);
    const trackExt = import_node_path2.default.extname(fp).toLowerCase();
    node.tracks.push({
      id: hashPath(fp),
      name: meta.title && meta.title.trim() ? meta.title.trim() : normalizeName(fp),
      path: nfp,
      duration: meta.duration,
      artist: meta.artist && meta.artist.trim() ? meta.artist.trim() : "\u6D63\u6C2C\u6095",
      album: dirName,
      format: trackExt.replace(".", ""),
      isVideo: VIDEO_EXTS.has(trackExt),
      coverPath: null,
      coverData: meta.coverData,
      lyricsPath: null,
      fileMtime: st.mtime,
      fileSize: st.size,
      metaTitle: meta.title,
      metaArtist: meta.artist,
      genre: meta.genre || null,
      bitrate: meta.bitrate || null,
      sampleRate: meta.sampleRate || null
    });
    let isRoot = true;
    for (let ri = 0; ri < cleanRoots.length; ri++) {
      if (parentDir === cleanRoots[ri] || parentDir.startsWith(cleanRoots[ri] + "/")) {
        isRoot = false;
        const pName = import_node_path2.default.basename(parentDir);
        const parentNode = getOrCreateNode(parentDir, pName);
        if (!parentNode.children.some((c) => c.path === dir)) {
          parentNode.children.push(node);
        }
        break;
      }
    }
    if (isRoot) {
      if (!roots.some((r) => r.path === dir)) {
        roots.push(node);
      }
    }
  }
  function findFolderCover(rootNode) {
    const external = findExternalCover(rootNode.path.replace(/\//g, import_node_path2.default.sep));
    if (external) return external;
    for (const t of rootNode.tracks) {
      const meta = metaResults.get(t.path);
      if (meta?.coverData) return meta.coverData;
    }
    for (const c of rootNode.children) {
      const r = findFolderCover(c);
      if (r) return r;
    }
    return null;
  }
  function computeNodeStats(node) {
    node.children.sort((a, b) => a.name.localeCompare(b.name));
    node.tracks.sort((a, b) => a.name.localeCompare(b.name));
    let trackCount = node.tracks.length;
    for (const child of node.children) {
      computeNodeStats(child);
      trackCount += child.trackCount;
    }
    node.trackCount = trackCount;
    node.coverData = findFolderCover(node);
  }
  for (const [, node] of nodeMap) {
    node.children.sort((a, b) => a.name.localeCompare(b.name));
    node.tracks.sort((a, b) => a.name.localeCompare(b.name));
    node.trackCount = node.tracks.length;
  }
  roots.sort((a, b) => a.name.localeCompare(b.name));
  for (let ri = 0; ri < cleanRoots.length; ri++) {
    const nrp = cleanRoots[ri];
    let hasContent = false;
    for (const [, node] of nodeMap) {
      const nn = node.path.replace(/\\/g, "/");
      if (nn === nrp || nn.startsWith(nrp + "/")) {
        hasContent = true;
        break;
      }
    }
    if (!hasContent) continue;
    const rn = getOrCreateNode(nrp, import_node_path2.default.basename(nrp));
    for (let i = roots.length - 1; i >= 0; i--) {
      const r = roots[i];
      const nr = r.path.replace(/\\/g, "/");
      if (nr.startsWith(nrp + "/")) {
        const nr2 = r.path.replace(/\\/g, "/");
        if (!rn.children.some((c) => c.path.replace(/\\/g, "/") === nr2)) rn.children.push(r);
        roots.splice(i, 1);
      }
    }
    const nrn = rn.path.replace(/\\/g, "/");
    if (!roots.some((r) => r.path.replace(/\\/g, "/") === nrn)) roots.push(rn);
  }
  for (const r of roots) computeNodeStats(r);
  return roots;
}
var COVER_EXTRACT_TIMEOUT = 3e3;
async function fillAlbumCovers(artists, options = {}) {
  const loadCachedCovers = options.loadCachedCovers !== false;
  const onlyTrackPaths = options.onlyTrackPaths;
  let existingCovers = /* @__PURE__ */ new Set();
  if (loadCachedCovers) {
    try {
      const coversDir2 = getCoversDir();
      if (import_node_fs2.default.existsSync(coversDir2)) {
        const files = import_node_fs2.default.readdirSync(coversDir2);
        for (const f of files) {
          if (f.startsWith("folder_") || !f.endsWith(".jpg")) continue;
          existingCovers.add(f.slice(0, -4));
        }
      }
    } catch {
    }
  }
  for (const artist of artists) {
    for (const album of artist.albums) {
      if (onlyTrackPaths) {
        const hasChangedTrack = album.tracks.some((track) => onlyTrackPaths.has(track.path) || onlyTrackPaths.has(track.path.replace(/\\/g, "/")));
        if (!hasChangedTrack) continue;
      }
      if (album.coverData) continue;
      let foundCached = false;
      if (loadCachedCovers) {
        for (const track of album.tracks) {
          const trackId = hashPath(track.path);
          if (existingCovers.has(trackId)) {
            try {
              const coversDir2 = getCoversDir();
              const coverPath = import_node_path2.default.join(coversDir2, `${trackId}.jpg`);
              const buffer = import_node_fs2.default.readFileSync(coverPath);
              if (buffer.length > 0) {
                album.coverData = `data:image/jpeg;base64,${buffer.toString("base64")}`;
                track.coverData = album.coverData;
                foundCached = true;
                break;
              }
            } catch {
            }
          }
        }
      }
      if (foundCached) continue;
      if (album.dirPath) {
        const externalCover = findExternalCover(album.dirPath, 1);
        if (externalCover) {
          album.coverData = externalCover;
          continue;
        }
        const anyImage = findAnyImage(album.dirPath);
        if (anyImage) {
          album.coverData = anyImage;
          continue;
        }
      }
      const targetTrack = album.tracks[0];
      if (targetTrack) {
        try {
          const meta = await Promise.race([
            musicMetadata.parseFile(targetTrack.path, {
              duration: false,
              skipCovers: false
            }),
            new Promise((_, reject) => setTimeout(() => reject(new Error("timeout")), COVER_EXTRACT_TIMEOUT))
          ]);
          if (meta.common.picture && meta.common.picture.length > 0) {
            const pic = meta.common.picture[0];
            const data = Buffer.isBuffer(pic.data) ? pic.data : Buffer.from(pic.data);
            if (data.length > 0 && data.length <= 15 * 1024 * 1024) {
              let fmt = pic.format || "image/jpeg";
              if (!fmt.startsWith("image/")) fmt = `image/${fmt}`;
              const b64 = `data:${fmt};base64,${data.toString("base64")}`;
              album.coverData = b64;
              targetTrack.coverData = b64;
            }
          }
        } catch {
        }
      }
    }
  }
}
async function scanFoldersWithProgress(folderPaths, existingMeta = /* @__PURE__ */ new Map(), onProgress, onStage) {
  console.time("[scan] total");
  onStage?.("\u9359\u6220\u5E47\u93C2\u56E6\u6B22...");
  console.time("[scan] discoverFiles");
  const files = await discoverFiles(folderPaths);
  console.timeEnd("[scan] discoverFiles");
  const totalFiles = files.length;
  onProgress?.(0, totalFiles);
  onStage?.(`\u7459\uFF46\u703D\u934F\u51A9\u669F\u93B9?.. (${totalFiles} \u6D93\uE045\u6783\u6D60?`);
  console.time("[scan] enrichWithWorkers");
  const metaResults = await enrichWithWorkers(files, existingMeta, onProgress);
  console.timeEnd("[scan] enrichWithWorkers");
  onStage?.("\u93C1\u5BF8\u608A\u7F01\u64B4\u702F...");
  console.time("[scan] groupTracksByFolder");
  const artists = await groupTracksByFolder(files, metaResults, folderPaths);
  console.timeEnd("[scan] groupTracksByFolder");
  console.time("[scan] buildFolderTree");
  const folderTree = await buildFolderTree(files, metaResults, folderPaths);
  console.timeEnd("[scan] buildFolderTree");
  console.time("[scan] fillAlbumCovers");
  await fillAlbumCovers(artists);
  console.timeEnd("[scan] fillAlbumCovers");
  const allTracks = [];
  let ti = 0;
  console.time("[scan] assembleAllTracks");
  for (const artist of artists) {
    for (const album of artist.albums) {
      for (const track of album.tracks) {
        track.albumCoverData = album.coverData || track.coverData || null;
        allTracks.push(track);
        ti++;
        if (ti % YIELD_INTERVAL === 0) await yieldToEventLoop();
      }
    }
  }
  console.timeEnd("[scan] assembleAllTracks");
  console.time("[scan] propagateCoversToFolderTree");
  const pathToCover = /* @__PURE__ */ new Map();
  for (const t of allTracks) {
    const cover = t.coverData || t.albumCoverData;
    if (cover) {
      const trackDir = import_node_path2.default.dirname(t.path).replace(/\\/g, "/");
      if (!pathToCover.has(trackDir)) pathToCover.set(trackDir, cover);
    }
  }
  function propagateCoversToNode(node) {
    const coverFromTrack = pathToCover.get(node.path.replace(/\\/g, "/"));
    if (coverFromTrack && !node.coverData) {
      node.coverData = coverFromTrack;
    }
    for (const child of node.children) {
      propagateCoversToNode(child);
      if (!node.coverData && child.coverData) {
        node.coverData = child.coverData;
      }
    }
  }
  for (const rootNode of folderTree) {
    propagateCoversToNode(rootNode);
  }
  console.timeEnd("[scan] propagateCoversToFolderTree");
  const tracksWithCover = allTracks.filter((t) => t.coverData || t.albumCoverData).length;
  const albumsWithCover = artists.reduce((a, ar) => a + ar.albums.filter((al) => al.coverData).length, 0);
  const totalAlbums = artists.reduce((a, ar) => a + ar.albums.length, 0);
  console.log(`[scan] cover coverage: ${tracksWithCover}/${allTracks.length} tracks, ${albumsWithCover}/${totalAlbums} albums`);
  console.timeEnd("[scan] total");
  return { artists, folderTree, allTracks, fileCount: totalFiles };
}
async function scanFoldersIncremental(allFolderPaths, existingMeta, onProgress, onStage) {
  console.time("[scan-incr] total");
  console.time("[scan-incr] discoverFiles");
  const files = await discoverFiles(allFolderPaths);
  console.timeEnd("[scan-incr] discoverFiles");
  const totalFiles = files.length;
  const changedFiles = [];
  for (const filePath of files) {
    const normalizedPath = filePath.replace(/\\/g, "/");
    const cached = existingMeta.get(normalizedPath);
    if (!cached) {
      changedFiles.push(filePath);
      continue;
    }
    const stat = getFileStat(filePath);
    const ext = import_node_path2.default.extname(filePath).toLowerCase();
    const shouldRefreshZeroVideoDuration = VIDEO_EXTS.has(ext) && (cached.duration || 0) <= 0;
    if (!stat || shouldRefreshZeroVideoDuration || cached.fileMtime !== stat.mtime || cached.fileSize !== stat.size) {
      changedFiles.push(filePath);
    }
  }
  console.log(`[scan-incr] ${changedFiles.length} new/changed out of ${totalFiles} files`);
  const changedFileSet = new Set(changedFiles);
  const metaResults = /* @__PURE__ */ new Map();
  for (const filePath of files) {
    const normalizedPath = filePath.replace(/\\/g, "/");
    const cached = existingMeta.get(normalizedPath);
    if (cached && !changedFileSet.has(filePath)) {
      metaResults.set(filePath, {
        duration: cached.duration,
        coverData: cached.coverData,
        title: cached.title,
        artist: cached.artist,
        genre: cached.genre,
        bitrate: cached.bitrate,
        sampleRate: cached.sampleRate
      });
    }
  }
  if (changedFiles.length > 0) {
    onStage?.("\u7459\uFF46\u703D\u93C2\u7248\u6783\u6D60\u8DFA\u5393\u93C1\u7248\u5D41...");
    const workerResults = await enrichWithWorkers(changedFiles, existingMeta, (completed, total) => {
      onProgress?.(completed, total);
    });
    for (const [filePath, meta] of workerResults) {
      metaResults.set(filePath, meta);
    }
  } else {
    onProgress?.(0, 0);
  }
  onStage?.("\u93CB\u52EB\u7F13\u95CA\u5145\u7BB0\u6434?..");
  console.time("[scan-incr] groupTracksByFolder");
  const artists = await groupTracksByFolder(files, metaResults, allFolderPaths);
  console.timeEnd("[scan-incr] groupTracksByFolder");
  console.time("[scan-incr] buildFolderTree");
  const folderTree = await buildFolderTree(files, metaResults, allFolderPaths);
  console.timeEnd("[scan-incr] buildFolderTree");
  await fillAlbumCovers(artists, { loadCachedCovers: false, onlyTrackPaths: changedFileSet });
  console.time("[scan-incr] assembleAllTracks");
  const allTracks = [];
  let ti = 0;
  for (const artist of artists) {
    for (const album of artist.albums) {
      for (const track of album.tracks) {
        track.albumCoverData = album.coverData || track.coverData || null;
        allTracks.push(track);
        ti++;
        if (ti % YIELD_INTERVAL === 0) await yieldToEventLoop();
      }
    }
  }
  console.timeEnd("[scan-incr] assembleAllTracks");
  const pathToCover = /* @__PURE__ */ new Map();
  for (const t of allTracks) {
    const cover = t.coverData || t.albumCoverData;
    if (cover) {
      const trackDir = import_node_path2.default.dirname(t.path).replace(/\\/g, "/");
      if (!pathToCover.has(trackDir)) pathToCover.set(trackDir, cover);
    }
  }
  function propagateCovers(node) {
    const c = pathToCover.get(node.path.replace(/\\/g, "/"));
    if (c && !node.coverData) node.coverData = c;
    for (const child of node.children) {
      propagateCovers(child);
      if (!node.coverData && child.coverData) node.coverData = child.coverData;
    }
  }
  for (const rootNode of folderTree) propagateCovers(rootNode);
  const tracksWithCover = allTracks.filter((t) => t.coverData || t.albumCoverData).length;
  console.log(`[scan-incr] cover coverage: ${tracksWithCover}/${allTracks.length} tracks`);
  console.timeEnd("[scan-incr] total");
  const changedPaths = new Set(changedFiles.map((f) => f.replace(/\\/g, "/")));
  return { artists, folderTree, allTracks, fileCount: totalFiles, changedPaths };
}
async function startWatching(folderPaths, onChange) {
  stopWatching();
  onChangeCallback = onChange;
  for (const fp of folderPaths) {
    try {
      const watcher = import_chokidar.default.watch(fp, {
        ignored: /(^|[\/\\])\../,
        persistent: true,
        ignoreInitial: true,
        depth: 99,
        awaitWriteFinish: { stabilityThreshold: 500, pollInterval: 100 }
      });
      let timer = null;
      const scheduleChange = () => {
        if (!onChangeCallback) return;
        if (timer) clearTimeout(timer);
        timer = setTimeout(() => {
          timer = null;
          onChangeCallback?.();
        }, CHOKIDAR_DELAY);
      };
      watcher.on("add", scheduleChange);
      watcher.on("change", scheduleChange);
      watcher.on("unlink", scheduleChange);
      watcher.on("addDir", scheduleChange);
      watcher.on("unlinkDir", scheduleChange);
      watchers.push(watcher);
    } catch (e) {
      console.error(`[watcher] Failed to watch ${fp}:`, e);
    }
  }
}
function stopWatching() {
  for (const w of watchers) {
    try {
      w.close();
    } catch {
    }
  }
  watchers = [];
  onChangeCallback = null;
}
function terminateWorkerPool() {
  _terminateWorkerPool();
}

// KX-Player/electron/libraryDb.ts
var import_node_fs3 = __toESM(require("node:fs"));
var import_node_path3 = __toESM(require("node:path"));
var import_better_sqlite3 = __toESM(require("better-sqlite3"));
var db = null;
var dbFilePath = "";
function normalizePath(input) {
  return input.replace(/\\/g, "/").replace(/\/+$/, "");
}
function ensureParentDir(filePath) {
  const dir = import_node_path3.default.dirname(filePath);
  if (!import_node_fs3.default.existsSync(dir)) import_node_fs3.default.mkdirSync(dir, { recursive: true });
}
function openDatabase(filePath) {
  if (db && dbFilePath === filePath) return db;
  if (db) {
    try {
      db.close();
    } catch {
    }
  }
  ensureParentDir(filePath);
  db = new import_better_sqlite3.default(filePath);
  dbFilePath = filePath;
  db.pragma("journal_mode = WAL");
  initializeSchema(db);
  return db;
}
function initializeSchema(database) {
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
  `);
  try {
    database.exec("ALTER TABLE tracks ADD COLUMN genre TEXT");
  } catch {
  }
  try {
    database.exec("ALTER TABLE tracks ADD COLUMN bitrate INTEGER");
  } catch {
  }
  try {
    database.exec("ALTER TABLE tracks ADD COLUMN sample_rate INTEGER");
  } catch {
  }
}
function clearSnapshot(database) {
  database.exec(`
    DELETE FROM folder_tracks;
    DELETE FROM folder_nodes;
    DELETE FROM tracks;
    DELETE FROM albums;
    DELETE FROM artists;
    DELETE FROM library_meta;
  `);
}
function insertMeta(database, key, value) {
  database.prepare(`INSERT INTO library_meta (key, value) VALUES (?, ?)`).run(key, value);
}
function saveLibrarySnapshot(filePath, snapshot) {
  const database = openDatabase(filePath);
  const txWrite = database.transaction(() => {
    clearSnapshot(database);
    insertMeta(database, "folderPaths", JSON.stringify(snapshot.folderPaths.map(normalizePath)));
    insertMeta(database, "fileCount", String(snapshot.fileCount));
    insertMeta(database, "scannedAt", String(snapshot.scannedAt));
    const insertArtist = database.prepare(`INSERT INTO artists (name, root_path) VALUES (?, ?)`);
    const insertAlbum = database.prepare(`INSERT INTO albums (artist_id, name, artist_name, cover_path, cover_data) VALUES (?, ?, ?, ?, ?)`);
    const insertTrack = database.prepare(`
      INSERT INTO tracks (
        id, artist_id, album_id, name, path, duration, artist, album, format, is_video,
        cover_path, cover_data, lyrics_path, file_mtime, file_size, meta_title, meta_artist,
        genre, bitrate, sample_rate, album_cover_data
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    const insertFolder = database.prepare(`INSERT INTO folder_nodes (path, name, parent_path, track_count, cover_data) VALUES (?, ?, ?, ?, ?)`);
    const insertFolderTrack = database.prepare(`INSERT INTO folder_tracks (folder_path, track_id) VALUES (?, ?)`);
    const folderParentMap = /* @__PURE__ */ new Map();
    const queue = [...snapshot.folderTree];
    let qi = 0;
    while (qi < queue.length) {
      const node = queue[qi++];
      const normalizedNodePath = normalizePath(node.path);
      for (const child of node.children) {
        folderParentMap.set(normalizePath(child.path), normalizedNodePath);
        queue.push(child);
      }
      if (!folderParentMap.has(normalizedNodePath)) folderParentMap.set(normalizedNodePath, null);
    }
    for (const artist of snapshot.artists) {
      const artistRes = insertArtist.run(artist.name, normalizePath(artist.path));
      const artistId = Number(artistRes.lastInsertRowid);
      for (const album of artist.albums) {
        const albumRes = insertAlbum.run(artistId, album.name, album.artist, album.coverPath, null);
        const albumId = Number(albumRes.lastInsertRowid);
        for (const track of album.tracks) {
          insertTrack.run(
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
            null,
            // cover_data -> filesystem
            track.lyricsPath,
            track.fileMtime,
            track.fileSize,
            track.metaTitle,
            track.metaArtist,
            track.genre ?? null,
            track.bitrate ?? null,
            track.sampleRate ?? null,
            null
            // album_cover_data -> filesystem
          );
        }
      }
    }
    const folderStack = [...snapshot.folderTree];
    let fi = 0;
    while (fi < folderStack.length) {
      const node = folderStack[fi++];
      const normalizedNodePath = normalizePath(node.path);
      insertFolder.run(
        normalizedNodePath,
        node.name,
        folderParentMap.get(normalizedNodePath) ?? null,
        node.trackCount,
        null
      );
      for (const track of node.tracks) {
        insertFolderTrack.run(normalizedNodePath, track.id);
      }
      folderStack.push(...node.children);
    }
  });
  txWrite();
}
function getSingleMeta(database, key) {
  const row = database.prepare(`SELECT value FROM library_meta WHERE key = ? LIMIT 1`).get(key);
  return row ? String(row.value) : null;
}
function rowsFromAll(database, sql, mapper) {
  const rows = database.prepare(sql).all();
  return rows.map(mapper);
}
function loadLibrarySnapshot(filePath, options) {
  const lean = !!(options && options.lean);
  if (!import_node_fs3.default.existsSync(filePath)) return null;
  const database = openDatabase(filePath);
  const folderPathsRaw = getSingleMeta(database, "folderPaths");
  if (!folderPathsRaw) return null;
  const trackRows = rowsFromAll(database, `
    SELECT
      t.id, t.name, t.path, t.duration, t.artist, t.album, t.format, t.is_video,
      t.cover_path, t.cover_data, t.lyrics_path, t.file_mtime, t.file_size,
      t.meta_title, t.meta_artist, t.genre, t.bitrate, t.sample_rate,
      t.album_cover_data, t.album_id, t.artist_id
    FROM tracks t
    ORDER BY t.artist COLLATE NOCASE, t.album COLLATE NOCASE, t.name COLLATE NOCASE
  `, (row) => ({
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
    artistId: Number(row.artist_id)
  }));
  let albumRows = [];
  let artistRows = [];
  if (!lean) {
    albumRows = rowsFromAll(database, `
      SELECT album_id, artist_id, name, artist_name, cover_path, cover_data
      FROM albums
      ORDER BY artist_name COLLATE NOCASE, name COLLATE NOCASE
    `, (row) => ({
      albumId: Number(row.album_id),
      artistId: Number(row.artist_id),
      name: String(row.name),
      artist: String(row.artist_name),
      coverPath: row.cover_path ? String(row.cover_path) : null,
      coverData: row.cover_data ? String(row.cover_data) : null
    }));
    artistRows = rowsFromAll(database, `
      SELECT artist_id, name, root_path
      FROM artists
      ORDER BY name COLLATE NOCASE
    `, (row) => ({
      artistId: Number(row.artist_id),
      name: String(row.name),
      path: String(row.root_path)
    }));
  }
  const tracksByAlbum = /* @__PURE__ */ new Map();
  const tracksById = /* @__PURE__ */ new Map();
  for (const row of trackRows) {
    const track = {
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
      albumCoverData: row.albumCoverData
    };
    if (!lean && !tracksByAlbum.has(row.albumId)) tracksByAlbum.set(row.albumId, []);
    if (!lean) tracksByAlbum.get(row.albumId).push(track);
    tracksById.set(track.id, track);
  }
  let artists = [];
  if (!lean) {
    const albumsByArtist = /* @__PURE__ */ new Map();
    for (const album of albumRows) {
      const record = {
        name: album.name,
        artist: album.artist,
        coverPath: album.coverPath,
        coverData: album.coverData,
        tracks: tracksByAlbum.get(album.albumId) ?? []
      };
      if (!albumsByArtist.has(album.artistId)) albumsByArtist.set(album.artistId, []);
      albumsByArtist.get(album.artistId).push(record);
    }
    artists = artistRows.map((artist) => ({
      name: artist.name,
      path: artist.path,
      albums: albumsByArtist.get(artist.artistId) ?? []
    }));
  }
  const folderRows = rowsFromAll(database, `
    SELECT path, name, parent_path, track_count, cover_data
    FROM folder_nodes
    ORDER BY path COLLATE NOCASE
  `, (row) => ({
    path: String(row.path),
    name: String(row.name),
    parentPath: row.parent_path ? String(row.parent_path) : null,
    trackCount: Number(row.track_count) || 0,
    coverData: row.cover_data ? String(row.cover_data) : null
  }));
  const folderTrackRows = rowsFromAll(database, `
    SELECT folder_path, track_id
    FROM folder_tracks
    ORDER BY folder_path COLLATE NOCASE
  `, (row) => ({
    folderPath: String(row.folder_path),
    trackId: String(row.track_id)
  }));
  const folderTrackMap = /* @__PURE__ */ new Map();
  for (const row of folderTrackRows) {
    const track = tracksById.get(row.trackId);
    if (!track) continue;
    if (!folderTrackMap.has(row.folderPath)) folderTrackMap.set(row.folderPath, []);
    folderTrackMap.get(row.folderPath).push(track);
  }
  const nodeMap = /* @__PURE__ */ new Map();
  for (const row of folderRows) {
    nodeMap.set(row.path, {
      name: row.name,
      path: row.path,
      children: [],
      tracks: folderTrackMap.get(row.path) ?? [],
      trackCount: row.trackCount,
      coverData: row.coverData
    });
  }
  const folderTree = [];
  for (const row of folderRows) {
    const node = nodeMap.get(row.path);
    if (row.parentPath && nodeMap.has(row.parentPath)) {
      nodeMap.get(row.parentPath).children.push(node);
    } else {
      folderTree.push(node);
    }
  }
  const allTracks = trackRows.map((row) => tracksById.get(row.id)).filter(Boolean);
  const fileCount = Number(getSingleMeta(database, "fileCount") || allTracks.length);
  const scannedAt = Number(getSingleMeta(database, "scannedAt") || 0);
  let parsedFolderPaths;
  try {
    parsedFolderPaths = JSON.parse(folderPathsRaw);
  } catch {
    return null;
  }
  return {
    folderPaths: parsedFolderPaths,
    artists,
    folderTree,
    allTracks,
    fileCount,
    scannedAt
  };
}
function loadTrackListSnapshot(filePath) {
  const snapshot = loadLibrarySnapshot(filePath, { lean: true });
  if (!snapshot) return null;
  for (const track of snapshot.allTracks) {
    track.coverData = null;
    track.albumCoverData = null;
  }
  for (const artist of snapshot.artists) {
    for (const album of artist.albums) {
      album.coverData = null;
      for (const track of album.tracks) {
        track.coverData = null;
        track.albumCoverData = null;
      }
    }
  }
  function stripFolderCovers(nodes) {
    for (const node of nodes) {
      node.coverData = null;
      for (const t of node.tracks) {
        t.coverData = null;
        t.albumCoverData = null;
      }
      stripFolderCovers(node.children);
    }
  }
  stripFolderCovers(snapshot.folderTree);
  return snapshot;
}
function loadTrackMetadataIndex(filePath) {
  if (!import_node_fs3.default.existsSync(filePath)) return /* @__PURE__ */ new Map();
  const database = openDatabase(filePath);
  const rows = rowsFromAll(database, `
    SELECT path, duration, cover_data, meta_title, meta_artist, file_mtime, file_size,
           genre, bitrate, sample_rate
    FROM tracks
  `, (row) => ({
    path: String(row.path),
    duration: Number(row.duration) || 0,
    hasCover: row.cover_data ? true : false,
    title: row.meta_title ? String(row.meta_title) : null,
    artist: row.meta_artist ? String(row.meta_artist) : null,
    fileMtime: Number(row.file_mtime) || 0,
    fileSize: Number(row.file_size) || 0,
    genre: row.genre ? String(row.genre) : null,
    bitrate: row.bitrate ? Number(row.bitrate) : null,
    sampleRate: row.sample_rate ? Number(row.sample_rate) : null
  }));
  const index = /* @__PURE__ */ new Map();
  for (const row of rows) index.set(row.path, row);
  return index;
}
function loadFullMetadataIndex(filePath) {
  if (!import_node_fs3.default.existsSync(filePath)) return /* @__PURE__ */ new Map();
  const database = openDatabase(filePath);
  const index = /* @__PURE__ */ new Map();
  const rows = rowsFromAll(database, `
    SELECT path, duration, meta_title, meta_artist, file_mtime, file_size,
           genre, bitrate, sample_rate
    FROM tracks
  `, (r) => r);
  for (const row of rows) {
    const r = row;
    const normalizedPath = String(r.path).replace(/\\/g, "/");
    index.set(normalizedPath, {
      duration: Number(r.duration) || 0,
      coverData: null,
      // covers loaded separately from filesystem
      title: r.meta_title ? String(r.meta_title) : null,
      artist: r.meta_artist ? String(r.meta_artist) : null,
      genre: r.genre ? String(r.genre) : null,
      bitrate: r.bitrate ? Number(r.bitrate) : null,
      sampleRate: r.sample_rate ? Number(r.sample_rate) : null,
      fileMtime: Number(r.file_mtime) || 0,
      fileSize: Number(r.file_size) || 0
    });
  }
  return index;
}

// KX-Player/electron/memMonitor.ts
var import_node_fs4 = __toESM(require("node:fs"));
var import_node_path4 = __toESM(require("node:path"));
var LOG_FILE_NAME = "memory.log";
var MAX_LOG_BYTES = 2 * 1024 * 1024;
var KEEP_BYTES = 1024 * 1024;
var SAMPLE_INTERVAL_MS = 1e4;
var logPath = null;
var lastSampleAt = { main: 0 };
var intervalHandle = null;
function formatLine(s) {
  const parts = [s.ts, s.source, s.event];
  if (s.pid !== void 0) parts.push(`pid=${s.pid}`);
  if (s.rssMb !== void 0) parts.push(`rss=${s.rssMb.toFixed(1)}MB`);
  if (s.heapUsedMb !== void 0) parts.push(`heap=${s.heapUsedMb.toFixed(1)}/${s.heapTotalMb?.toFixed(1) ?? "?"}MB`);
  if (s.jsHeapUsedMb !== void 0) parts.push(`jsHeap=${s.jsHeapUsedMb.toFixed(1)}/${s.jsHeapTotalMb?.toFixed(1) ?? "?"}MB`);
  if (s.externalMb !== void 0) parts.push(`ext=${s.externalMb.toFixed(1)}MB`);
  if (s.arraysMb !== void 0) parts.push(`buffers=${s.arraysMb.toFixed(1)}MB`);
  if (s.deltaMs !== void 0) parts.push(`dt=${s.deltaMs}ms`);
  if (s.extra) parts.push(s.extra);
  return parts.join(" | ") + "\n";
}
function writeLog(line) {
  if (!logPath) return;
  try {
    if (import_node_fs4.default.existsSync(logPath)) {
      const stat = import_node_fs4.default.statSync(logPath);
      if (stat.size > MAX_LOG_BYTES) {
        const content = import_node_fs4.default.readFileSync(logPath, "utf-8");
        const keepFrom = Math.max(0, content.length - KEEP_BYTES);
        import_node_fs4.default.writeFileSync(logPath, "// --- rotated ---\n" + content.slice(keepFrom) + line, "utf-8");
        return;
      }
    }
    import_node_fs4.default.appendFileSync(logPath, line, "utf-8");
  } catch {
  }
}
function nowIso() {
  return (/* @__PURE__ */ new Date()).toISOString();
}
function mb(bytes) {
  if (bytes == null || !Number.isFinite(bytes)) return void 0;
  return bytes / (1024 * 1024);
}
function initMemoryMonitor(targetDir) {
  if (logPath) return;
  if (!import_node_fs4.default.existsSync(targetDir)) import_node_fs4.default.mkdirSync(targetDir, { recursive: true });
  logPath = import_node_path4.default.join(targetDir, LOG_FILE_NAME);
  try {
    import_node_fs4.default.writeFileSync(logPath, `// memory monitor started at ${nowIso()} (pid=${process.pid})
`, "utf-8");
  } catch {
  }
  startInterval("main");
}
function getMainSample(event, extra, pid) {
  const mu = process.memoryUsage();
  const ts = nowIso();
  const prev = lastSampleAt.main;
  lastSampleAt.main = Date.now();
  return {
    ts,
    source: "main",
    event,
    pid,
    rssMb: mb(mu.rss),
    heapUsedMb: mb(mu.heapUsed),
    heapTotalMb: mb(mu.heapTotal),
    externalMb: mb(mu.external),
    deltaMs: prev ? Date.now() - prev : void 0,
    extra
  };
}
function markMain(event, extra) {
  if (!logPath) return;
  writeLog(formatLine(getMainSample(event, extra, process.pid)));
}
function startInterval(source) {
  if (intervalHandle) return;
  intervalHandle = setInterval(() => {
    try {
      writeLog(formatLine(getMainSample("tick")));
    } catch {
    }
  }, SAMPLE_INTERVAL_MS);
  intervalHandle.unref?.();
}
function reportRendererSample(sample) {
  if (!logPath) return;
  const ts = sample.ts || nowIso();
  const source = "renderer";
  const prev = lastSampleAt[`r-${sample.pid ?? "x"}`] || 0;
  lastSampleAt[`r-${sample.pid ?? "x"}`] = Date.now();
  const line = {
    ts,
    source,
    event: sample.event,
    pid: sample.pid,
    jsHeapUsedMb: mb(sample.jsHeapUsedMb),
    jsHeapTotalMb: mb(sample.jsHeapTotalMb),
    arraysMb: mb(sample.arraysMb),
    heapUsedMb: mb(sample.heapUsedMb),
    heapTotalMb: mb(sample.heapTotalMb),
    deltaMs: prev ? Date.now() - prev : void 0,
    extra: sample.extra,
    rssMb: void 0,
    externalMb: void 0
  };
  writeLog(formatLine(line));
}

// KX-Player/electron/main.ts
var IMG_MIME = { jpg: "jpeg", jpeg: "jpeg", png: "png", bmp: "bmp", webp: "webp", gif: "gif" };
var MAX_LOG_BYTES2 = 1024 * 1024;
function writeLog2(msg) {
  try {
    const logPath2 = import_node_path5.default.join(getUserDataDir(), "kx-player-log.txt");
    const ts = (/* @__PURE__ */ new Date()).toISOString();
    const line = `[${ts}] ${msg}
`;
    if (import_node_fs5.default.existsSync(logPath2)) {
      const stat = import_node_fs5.default.statSync(logPath2);
      if (stat.size > MAX_LOG_BYTES2) {
        const content = import_node_fs5.default.readFileSync(logPath2, "utf-8");
        const keepFrom = Math.max(0, content.length - MAX_LOG_BYTES2 / 2);
        import_node_fs5.default.writeFileSync(logPath2, content.slice(keepFrom) + line, "utf-8");
        return;
      }
    }
    import_node_fs5.default.appendFileSync(logPath2, line, "utf-8");
  } catch {
  }
}
import_electron.app.commandLine.appendSwitch("disable-crashpad");
import_electron.app.commandLine.appendSwitch("disable-breakpad");
import_electron.app.commandLine.appendSwitch("disable-gpu-shader-disk-cache");
import_electron.app.commandLine.appendSwitch("disable-extensions");
var mainWindow = null;
function getUserDataDir() {
  return import_electron.app.getPath("userData");
}
function getSettingsPath() {
  return import_node_path5.default.join(getUserDataDir(), "kx-player-settings.json");
}
function getLibraryDbPath() {
  return import_node_path5.default.join(getUserDataDir(), "kx-player-library.sqlite");
}
function getBgImagePath() {
  return import_node_path5.default.join(getUserDataDir(), "kx-player-bg.png");
}
function getProjectRoot() {
  return import_node_path5.default.resolve(__dirname, "..");
}
function createWindow() {
  mainWindow = new import_electron.BrowserWindow({
    width: 1200,
    height: 800,
    minWidth: 900,
    minHeight: 600,
    frame: false,
    titleBarStyle: "hidden",
    webPreferences: {
      preload: import_node_path5.default.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      webSecurity: !process.env.VITE_DEV_SERVER_URL
    },
    icon: import_node_path5.default.join(__dirname, "../public/favicon.ico"),
    backgroundColor: "#1a1a1e"
  });
  mainWindow.webContents.session.setPermissionRequestHandler((_webContents, permission, callback) => {
    const allowed = ["media"];
    callback(allowed.includes(permission));
  });
  mainWindow.on("maximize", () => {
    mainWindow?.webContents.send("window:maximizeChange", true);
  });
  mainWindow.on("unmaximize", () => {
    mainWindow?.webContents.send("window:maximizeChange", false);
  });
  mainWindow.on("closed", () => {
    mainWindow = null;
  });
  if (process.env.VITE_DEV_SERVER_URL) {
    mainWindow.loadURL(process.env.VITE_DEV_SERVER_URL);
  } else {
    mainWindow.loadFile(import_node_path5.default.join(__dirname, "../dist/index.html"));
  }
}
import_electron.app.whenReady().then(() => {
  import_electron.app.setPath("cache", import_node_path5.default.join(getUserDataDir(), "Cache"));
  initCoverDir(getUserDataDir());
  loadFolderCoverMap(getUserDataDir());
  initMemoryMonitor(getProjectRoot());
  markMain("main:appReady");
  createWindow();
  createTray();
  mainWindow?.on("close", (event) => {
    if (!forceCloseFlag) {
      event.preventDefault();
      mainWindow?.hide();
    }
  });
  if (process.env.VITE_DEV_SERVER_URL) {
    mainWindow?.webContents.session.setCertificateVerifyProc((_request, callback) => {
      callback(0);
    });
  }
  import_electron.app.on("activate", () => {
    if (import_electron.BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});
import_electron.app.on("window-all-closed", () => {
  if (tray && !forceCloseFlag) return;
  if (process.platform !== "darwin") {
    import_electron.app.quit();
  }
});
import_electron.ipcMain.handle("dialog:openFolder", async () => {
  if (!mainWindow) return null;
  const result = await import_electron.dialog.showOpenDialog(mainWindow, {
    properties: ["openDirectory"],
    title: "\u9009\u62E9\u97F3\u4E50\u6587\u4EF6\u5939"
  });
  if (result.canceled || result.filePaths.length === 0) return null;
  return result.filePaths;
});
import_electron.ipcMain.handle("dialog:openImageFile", async () => {
  if (!mainWindow) return null;
  const result = await import_electron.dialog.showOpenDialog(mainWindow, {
    filters: [{ name: "\u56FE\u7247", extensions: ["png", "jpg", "jpeg", "bmp", "webp", "gif"] }],
    properties: ["openFile"]
  });
  return result.canceled ? null : result.filePaths[0];
});
import_electron.ipcMain.handle("dialog:openAudioFiles", async () => {
  if (!mainWindow) return [];
  const result = await import_electron.dialog.showOpenDialog(mainWindow, {
    filters: [{ name: "\u97F3\u9891/\u89C6\u9891", extensions: ["mp3", "flac", "wav", "ogg", "m4a", "aac", "wma", "opus", "ape", "wv", "aiff", "mp4", "mkv", "avi", "mov", "webm", "flv", "wmv"] }],
    properties: ["openFile", "multiSelections"]
  });
  return result.canceled ? [] : result.filePaths;
});
import_electron.ipcMain.on("mem:report", (_event, sample) => {
  try {
    reportRendererSample(sample || {});
  } catch {
  }
});
import_electron.ipcMain.handle("scanner:scanFoldersWithProgress", async (event, folderPaths) => {
  const sender = event.sender;
  const t0 = Date.now();
  markMain("main:scanFolders:start", `paths=${folderPaths.length}`);
  try {
    console.time("[scan] loadMetadataIndex");
    const metadataIndex = await loadTrackMetadataIndex(getLibraryDbPath());
    console.timeEnd("[scan] loadMetadataIndex");
    markMain("main:scanFolders:metaIndexLoaded", `indexSize=${metadataIndex?.size ?? 0}`);
    const result = await scanFoldersWithProgress(
      folderPaths,
      metadataIndex,
      (completed, total) => {
        if (!sender.isDestroyed()) {
          sender.send("scanner:progress", { completed, total, stage: "\u89E3\u6790\u5143\u6570\u636E..." });
          if (total > 0) {
            const pct = completed / total * 100;
            if (pct >= 100 && sender._markScan100 !== true) {
              ;
              sender._markScan100 = true;
              markMain("main:scanFolders:progress=100%", `files=${total}`);
            } else if (pct >= 50 && sender._markScan50 !== true) {
              ;
              sender._markScan50 = true;
              markMain("main:scanFolders:progress=50%", `files=${Math.floor(total / 2)}`);
            }
          }
        }
      },
      (stage) => {
        if (!sender.isDestroyed()) {
          sender.send("scanner:stage", stage);
        }
      }
    );
    sender._markScan50 = false;
    sender._markScan100 = false;
    console.time("[scan] saveLibrary");
    try {
      await saveLibrarySnapshot(getLibraryDbPath(), {
        folderPaths,
        artists: result.artists,
        folderTree: result.folderTree,
        allTracks: result.allTracks,
        fileCount: result.fileCount,
        scannedAt: Date.now()
      });
    } catch {
    }
    console.timeEnd("[scan] saveLibrary");
    console.time("[scan] saveCovers");
    try {
      await saveCoversToFileSystem(result);
      console.timeEnd("[scan] saveCovers");
    } catch (e) {
      console.error("[scan] saveCovers failed:", e);
    }
    markMain("main:scanFolders:done", `tracks=${result.fileCount} dt=${Date.now() - t0}ms`);
    return result;
  } catch (err) {
    console.error("[scan] Fatal error:", err?.message || err);
    markMain("main:scanFolders:error", err?.message || String(err));
    return { artists: [], folderTree: [], allTracks: [], fileCount: 0 };
  }
});
import_electron.ipcMain.handle("library:load", async () => {
  try {
    const t0 = Date.now();
    const snap = await loadLibrarySnapshot(getLibraryDbPath());
    markMain("main:library:load", `dt=${Date.now() - t0}ms tracks=${snap?.fileCount ?? 0}`);
    return snap;
  } catch {
    return null;
  }
});
import_electron.ipcMain.handle("library:loadFast", async () => {
  try {
    const t0 = Date.now();
    const snap = await loadTrackListSnapshot(getLibraryDbPath());
    markMain("main:library:loadFast", `dt=${Date.now() - t0}ms tracks=${snap?.fileCount ?? 0}`);
    return snap;
  } catch {
    return null;
  }
});
import_electron.ipcMain.handle("library:getCovers", async (_event, trackIds) => {
  try {
    if (!trackIds || !trackIds.length) return {};
    const result = {};
    for (let i = 0; i < trackIds.length; i += 100) {
      const batch = trackIds.slice(i, i + 100);
      const covers = await getTrackCoversBatchAsync(batch);
      Object.assign(result, covers);
    }
    return result;
  } catch {
    return {};
  }
});
import_electron.ipcMain.handle("library:getFolderCovers", async (_event, folderPaths) => {
  try {
    if (!folderPaths || !folderPaths.length) return {};
    const result = {};
    for (let i = 0; i < folderPaths.length; i += 100) {
      const batch = folderPaths.slice(i, i + 100);
      const covers = await getFolderCoversBatchAsync(batch);
      Object.assign(result, covers);
    }
    return result;
  } catch {
    return {};
  }
});
import_electron.ipcMain.handle("library:loadFolderCovers", async () => {
  try {
    return await getAllFolderCoversFromMapAsync();
  } catch {
    return {};
  }
});
import_electron.ipcMain.handle("library:scanIncremental", async (event, folderPaths) => {
  const sender = event.sender;
  let fullMeta = null;
  const t0 = Date.now();
  markMain("main:scanIncremental:start", `paths=${folderPaths.length}`);
  try {
    console.time("[scan-incr] loadFullMeta");
    fullMeta = await loadFullMetadataIndex(getLibraryDbPath());
    console.timeEnd("[scan-incr] loadFullMeta");
    const result = await scanFoldersIncremental(
      folderPaths,
      fullMeta,
      (completed, total) => {
        if (!sender.isDestroyed()) sender.send("scanner:progress", { completed, total, stage: "\u89E3\u6790\u5143\u6570\u636E..." });
      },
      (stage) => {
        if (!sender.isDestroyed()) sender.send("scanner:stage", stage);
      }
    );
    console.time("[scan-incr] saveLibrary");
    try {
      await saveLibrarySnapshot(getLibraryDbPath(), {
        folderPaths,
        artists: result.artists,
        folderTree: result.folderTree,
        allTracks: result.allTracks,
        fileCount: result.fileCount,
        scannedAt: Date.now()
      });
    } catch {
    }
    console.timeEnd("[scan-incr] saveLibrary");
    console.time("[scan-incr] saveCovers");
    try {
      await saveCoversIncremental(result, result.changedPaths || /* @__PURE__ */ new Set());
      console.timeEnd("[scan-incr] saveCovers");
    } catch (e) {
      console.error("[scan-incr] saveCovers failed:", e);
    }
    markMain("main:scanIncremental:done", `tracks=${result.fileCount} dt=${Date.now() - t0}ms`);
    return result;
  } catch (err) {
    console.error("[scan-incr] Fatal error:", err?.message || err);
    markMain("main:scanIncremental:error", err?.message || String(err));
    return { artists: [], folderTree: [], allTracks: [], fileCount: 0 };
  } finally {
    fullMeta?.clear();
  }
});
import_electron.ipcMain.handle("library:removeFolder", async (_event, folderPath, remainingPaths) => {
  const t0 = Date.now();
  markMain("main:removeFolder:start", `path=${folderPath}`);
  try {
    let calcCount = function(node) {
      let c = node.tracks.length;
      for (const child of node.children) c += calcCount(child);
      node.trackCount = c;
      return c;
    }, propCovers = function(node) {
      const c = pathToCover.get(node.path);
      if (c && !node.coverData) node.coverData = c;
      for (const child of node.children) {
        propCovers(child);
        if (!node.coverData && child.coverData) node.coverData = child.coverData;
      }
    };
    const dbPath = getLibraryDbPath();
    const snapshot = await loadLibrarySnapshot(dbPath);
    if (!snapshot) return { folderTree: [], allTracks: [], fileCount: 0, folderPaths: remainingPaths };
    const normalizedRemove = folderPath.replace(/\\/g, "/").replace(/\/+$/, "");
    const keptTracks = (snapshot.allTracks || []).filter((t) => {
      const np = (t.path || "").replace(/\\/g, "/").replace(/\/+$/, "");
      return np !== normalizedRemove && !np.startsWith(normalizedRemove + "/");
    });
    const artistMap = /* @__PURE__ */ new Map();
    for (const track of keptTracks) {
      const tp = (track.path || "").replace(/\\/g, "/");
      let matchedRoot = null;
      for (const rp of remainingPaths) {
        const nrp = rp.replace(/\\/g, "/").replace(/\/+$/, "");
        if (tp === nrp || tp.startsWith(nrp + "/")) {
          matchedRoot = nrp;
          break;
        }
      }
      if (!matchedRoot) continue;
      const rel = tp.slice(matchedRoot.length + 1);
      const parts = rel.split("/");
      let artistName = matchedRoot.split("/").pop() || matchedRoot;
      let albumName;
      if (parts.length >= 2) {
        albumName = parts[0];
      } else {
        albumName = artistName;
      }
      if (track.artist && track.artist.trim()) artistName = track.artist.trim();
      if (!artistMap.has(artistName)) artistMap.set(artistName, { path: matchedRoot, albums: /* @__PURE__ */ new Map() });
      const artist = artistMap.get(artistName);
      if (!artist.albums.has(albumName)) {
        artist.albums.set(albumName, {
          name: albumName,
          artist: artistName,
          dirPath: parts.length >= 1 ? matchedRoot + "/" + parts[0] : matchedRoot,
          coverPath: null,
          coverData: null,
          tracks: []
        });
      }
      artist.albums.get(albumName).tracks.push(track);
    }
    const artists = Array.from(artistMap.entries()).map(([name, data]) => ({
      name,
      path: data.path,
      albums: Array.from(data.albums.values())
    }));
    const folderMap = /* @__PURE__ */ new Map();
    for (const rp of remainingPaths) {
      const nrp = rp.replace(/\\/g, "/").replace(/\/+$/, "");
      if (!folderMap.has(nrp)) folderMap.set(nrp, { node: { path: nrp, name: nrp.split("/").pop() || nrp, children: [], tracks: [], trackCount: 0, coverData: null }, parent: "" });
    }
    for (const track of keptTracks) {
      const dir = (track.path || "").replace(/\\/g, "/").replace(/\/+$/, "");
      const dirParts = dir.split("/");
      for (let i = 1; i <= dirParts.length; i++) {
        const p = dirParts.slice(0, i).join("/");
        if (!folderMap.has(p)) {
          const parent = dirParts.slice(0, i - 1).join("/");
          folderMap.set(p, { node: { path: p, name: dirParts[i - 1] || p, children: [], tracks: [], trackCount: 0, coverData: null }, parent });
        }
      }
      const entry = folderMap.get(dir);
      if (entry) entry.node.tracks.push(track);
    }
    for (const [p, entry] of folderMap) {
      if (entry.parent && folderMap.has(entry.parent)) {
        folderMap.get(entry.parent).node.children.push(entry.node);
      }
    }
    for (const [, entry] of folderMap) {
      entry.node.children.sort((a, b) => a.name.localeCompare(b.name));
    }
    const roots = remainingPaths.map((rp) => {
      const nrp = rp.replace(/\\/g, "/").replace(/\/+$/, "");
      return folderMap.get(nrp)?.node;
    }).filter(Boolean);
    for (const r of roots) calcCount(r);
    const pathToCover = /* @__PURE__ */ new Map();
    for (const t of keptTracks) {
      const cover = t.coverData || t.albumCoverData;
      if (cover) {
        const trackDir = (t.path || "").replace(/\\/g, "/").split("/").slice(0, -1).join("/");
        if (!pathToCover.has(trackDir)) pathToCover.set(trackDir, cover);
      }
    }
    for (const r of roots) propCovers(r);
    await saveLibrarySnapshot(dbPath, {
      folderPaths: remainingPaths,
      artists,
      folderTree: roots,
      allTracks: keptTracks,
      fileCount: keptTracks.length,
      scannedAt: Date.now()
    });
    try {
      const coversDir2 = getCoversDir();
      const removedTracks = (snapshot.allTracks || []).filter((t) => {
        const np = (t.path || "").replace(/\\/g, "/").replace(/\/+$/, "");
        return np === normalizedRemove || np.startsWith(normalizedRemove + "/");
      });
      for (const t of removedTracks) {
        try {
          import_node_fs5.default.unlinkSync(import_node_path5.default.join(coversDir2, `${t.id}.jpg`));
        } catch {
        }
      }
    } catch {
    }
    markMain("main:removeFolder:done", `kept=${keptTracks.length} dt=${Date.now() - t0}ms`);
    return { folderTree: roots, allTracks: keptTracks, fileCount: keptTracks.length, folderPaths: remainingPaths };
  } catch (err) {
    console.error("[removeFolder] Error:", err?.message || err);
    markMain("main:removeFolder:error", err?.message || String(err));
    return { folderTree: [], allTracks: [], fileCount: 0, folderPaths: remainingPaths };
  }
});
async function saveCoversToFileSystem(result) {
  let trackSaved = 0, trackTotal = 0;
  const coverDedup = /* @__PURE__ */ new Map();
  const trackPromises = [];
  for (const track of result.allTracks || []) {
    const cd = track.coverData || track.albumCoverData;
    if (cd) {
      trackTotal++;
      if (coverDedup.has(cd)) {
        coverDedup.get(cd).push(track.id);
      } else {
        coverDedup.set(cd, [track.id]);
      }
    }
  }
  for (const [dataUrl, trackIds] of coverDedup) {
    trackPromises.push((async () => {
      const firstId = trackIds[0];
      const saved = await saveTrackCover(firstId, dataUrl);
      if (saved) {
        trackSaved++;
        const covDir = getCoversDir();
        for (let i = 1; i < trackIds.length; i++) {
          try {
            const srcPath = import_node_path5.default.join(covDir, `${firstId}.jpg`);
            const dstPath = import_node_path5.default.join(covDir, `${trackIds[i]}.jpg`);
            import_node_fs5.default.copyFileSync(srcPath, dstPath);
            trackSaved++;
          } catch {
          }
        }
      }
    })());
    if (trackPromises.length > 50) {
      await Promise.all(trackPromises);
      trackPromises.length = 0;
    }
  }
  if (trackPromises.length) await Promise.all(trackPromises);
  console.log(`[cover] saved ${trackSaved}/${trackTotal} track covers`);
  let folderSaved = 0, folderTotal = 0;
  const saveFolderCovers = async (nodes) => {
    for (const node of nodes) {
      if (node.coverData) {
        folderTotal++;
        const saved = await saveFolderCover(node.path, node.coverData);
        if (saved) {
          folderSaved++;
          setFolderCoverMapping(node.path);
        }
      }
      const extCover = findExternalCoverInDir(node.path);
      if (extCover && !node.coverData) {
        folderTotal++;
        const saved = await saveExternalCover(node.path, extCover);
        if (saved) {
          folderSaved++;
          setFolderCoverMapping(node.path);
        }
      }
      if (node.children) await saveFolderCovers(node.children);
    }
  };
  await saveFolderCovers(result.folderTree || []);
  console.log(`[cover] saved ${folderSaved}/${folderTotal} folder covers`);
}
async function saveCoversIncremental(result, changedPaths) {
  let trackSaved = 0, trackTotal = 0;
  const coverDedup = /* @__PURE__ */ new Map();
  for (const track of result.allTracks || []) {
    const normalizedPath = (track.path || "").replace(/\\/g, "/");
    if (!changedPaths.has(normalizedPath)) continue;
    const cd = track.coverData || track.albumCoverData;
    if (cd) {
      trackTotal++;
      if (coverDedup.has(cd)) {
        coverDedup.get(cd).push(track.id);
      } else {
        coverDedup.set(cd, [track.id]);
      }
    }
  }
  const trackPromises = [];
  for (const [dataUrl, trackIds] of coverDedup) {
    trackPromises.push((async () => {
      const firstId = trackIds[0];
      const saved = await saveTrackCover(firstId, dataUrl);
      if (saved) {
        trackSaved++;
        const covDir = getCoversDir();
        for (let i = 1; i < trackIds.length; i++) {
          try {
            const srcPath = import_node_path5.default.join(covDir, `${firstId}.jpg`);
            const dstPath = import_node_path5.default.join(covDir, `${trackIds[i]}.jpg`);
            import_node_fs5.default.copyFileSync(srcPath, dstPath);
            trackSaved++;
          } catch {
          }
        }
      }
    })());
    if (trackPromises.length > 50) {
      await Promise.all(trackPromises);
      trackPromises.length = 0;
    }
  }
  if (trackPromises.length) await Promise.all(trackPromises);
  console.log(`[cover-incr] saved ${trackSaved}/${trackTotal} track covers (incremental)`);
  const affectedFolders = /* @__PURE__ */ new Set();
  for (const cp of changedPaths) {
    let dir = import_node_path5.default.dirname(cp).replace(/\\/g, "/");
    while (dir) {
      affectedFolders.add(dir);
      const parent = import_node_path5.default.dirname(dir).replace(/\\/g, "/");
      if (parent === dir) break;
      dir = parent;
    }
  }
  let folderSaved = 0, folderTotal = 0;
  const saveFolderCoversIncr = async (nodes) => {
    for (const node of nodes) {
      const nodePath = (node.path || "").replace(/\\/g, "/");
      if (affectedFolders.has(nodePath)) {
        if (node.coverData) {
          folderTotal++;
          const saved = await saveFolderCover(node.path, node.coverData);
          if (saved) {
            folderSaved++;
            setFolderCoverMapping(node.path);
          }
        }
        const extCover = findExternalCoverInDir(node.path);
        if (extCover && !node.coverData) {
          folderTotal++;
          const saved = await saveExternalCover(node.path, extCover);
          if (saved) {
            folderSaved++;
            setFolderCoverMapping(node.path);
          }
        }
      }
      if (node.children) await saveFolderCoversIncr(node.children);
    }
  };
  await saveFolderCoversIncr(result.folderTree || []);
  console.log(`[cover-incr] saved ${folderSaved}/${folderTotal} folder covers (incremental)`);
}
import_electron.ipcMain.handle("scanner:startWatching", async (_event, folderPaths) => {
  if (!mainWindow) return;
  markMain("main:watching:start", `paths=${folderPaths.length}`);
  await startWatching(folderPaths, () => {
    mainWindow?.webContents.send("scanner:fsChanged");
  });
});
import_electron.ipcMain.handle("scanner:stopWatching", async () => {
  markMain("main:watching:stop");
  stopWatching();
});
import_electron.ipcMain.handle("media:getAudioDevices", async () => {
  if (!mainWindow) return [];
  try {
    const devices = await mainWindow.webContents.executeJavaScript(`
      (async () => {
        try {
          const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
          stream.getTracks().forEach(track => track.stop())
          const devices = await navigator.mediaDevices.enumerateDevices()
          return devices
            .filter(d => d.kind === 'audiooutput' && d.deviceId && d.deviceId !== 'communications' && d.deviceId !== 'default')
            .map(d => ({ deviceId: d.deviceId, label: d.label || (d.deviceId === 'default' ? '\u7CFB\u7EDF\u9ED8\u8BA4\u8F93\u51FA' : '\u97F3\u9891\u8BBE\u5907'), kind: d.kind }))
        } catch {
          try {
            const devices = await navigator.mediaDevices.enumerateDevices()
            return devices
              .filter(d => d.kind === 'audiooutput')
              .map(d => ({ deviceId: d.deviceId, label: d.label || (d.deviceId === 'default' ? '\u7CFB\u7EDF\u9ED8\u8BA4\u8F93\u51FA' : '\u97F3\u9891\u8BBE\u5907'), kind: d.kind }))
          } catch { return [] }
        }
      })()
    `);
    const seen = /* @__PURE__ */ new Set();
    const unique = devices.filter((d) => {
      if (seen.has(d.deviceId)) return false;
      seen.add(d.deviceId);
      return true;
    });
    if (!unique.some((d) => d.deviceId === "default")) {
      unique.unshift({ deviceId: "default", label: "\u7CFB\u7EDF\u9ED8\u8BA4\u8F93\u51FA", kind: "audiooutput" });
    }
    return unique;
  } catch {
    return [{ deviceId: "default", label: "\u7CFB\u7EDF\u9ED8\u8BA4\u8F93\u51FA", kind: "audiooutput" }];
  }
});
import_electron.ipcMain.handle("media:setAudioDevice", async (_event, deviceId) => {
  if (!mainWindow) return false;
  try {
    await mainWindow.webContents.executeJavaScript(`
      (async () => {
        const audioElements = document.querySelectorAll('audio');
        const deviceId = ${JSON.stringify(deviceId)};
        for (const audio of audioElements) {
          if (typeof audio.setSinkId === 'function') {
            try { await audio.setSinkId(deviceId) } catch {}
          }
        }
        return true;
      })()
    `);
    return true;
  } catch {
    return false;
  }
});
import_electron.ipcMain.handle("file:readAsDataURL", async (_event, filePath) => {
  try {
    const buffer = await import_node_fs5.default.promises.readFile(filePath);
    const ext = import_node_path5.default.extname(filePath).toLowerCase().replace(".", "");
    const mime = IMG_MIME[ext] || ext;
    return `data:image/${mime};base64,${buffer.toString("base64")}`;
  } catch {
    return null;
  }
});
import_electron.ipcMain.handle("file:exists", async (_event, filePath) => {
  return import_node_fs5.default.existsSync(filePath);
});
import_electron.ipcMain.handle("file:readTextFile", async (_event, filePath) => {
  try {
    const buffer = await import_promises2.default.readFile(filePath);
    try {
      const utf8 = new TextDecoder("utf-8", { fatal: true }).decode(buffer);
      return utf8;
    } catch {
      try {
        const gbk = new TextDecoder("gbk").decode(buffer);
        return gbk;
      } catch {
        return buffer.toString("utf-8");
      }
    }
  } catch {
    return null;
  }
});
import_electron.ipcMain.handle("file:listDir", async (_event, dirPath) => {
  try {
    const entries = await import_promises2.default.readdir(dirPath, { withFileTypes: false });
    return entries;
  } catch {
    return [];
  }
});
import_electron.ipcMain.handle("dialog:selectBgImage", async () => {
  if (!mainWindow) return null;
  const result = await import_electron.dialog.showOpenDialog(mainWindow, {
    properties: ["openFile"],
    title: "\u9009\u62E9\u80CC\u666F\u56FE\u7247",
    filters: [{ name: "\u56FE\u7247\u6587\u4EF6", extensions: ["jpg", "jpeg", "png", "bmp", "webp", "gif"] }]
  });
  if (result.canceled || result.filePaths.length === 0) return null;
  const filePath = result.filePaths[0];
  try {
    const buffer = await import_node_fs5.default.promises.readFile(filePath);
    const ext = import_node_path5.default.extname(filePath).toLowerCase().replace(".", "");
    const mime = IMG_MIME[ext] || ext;
    return { dataUrl: `data:image/${mime};base64,${buffer.toString("base64")}`, path: filePath };
  } catch {
    return null;
  }
});
import_electron.ipcMain.handle("settings:load", async () => {
  try {
    const settingsPath = getSettingsPath();
    if (import_node_fs5.default.existsSync(settingsPath)) {
      const data = await import_promises2.default.readFile(settingsPath, "utf-8");
      return JSON.parse(data);
    }
  } catch {
  }
  return {};
});
import_electron.ipcMain.handle("settings:save", async (_event, settings) => {
  try {
    const settingsPath = getSettingsPath();
    const dir = import_node_path5.default.dirname(settingsPath);
    if (!import_node_fs5.default.existsSync(dir)) {
      import_node_fs5.default.mkdirSync(dir, { recursive: true });
    }
    import_node_fs5.default.writeFileSync(settingsPath, JSON.stringify(settings, null, 2), "utf-8");
    return true;
  } catch {
    return false;
  }
});
import_electron.ipcMain.on("settings:syncSave", (_event, settings) => {
  try {
    const settingsPath = getSettingsPath();
    const dir = import_node_path5.default.dirname(settingsPath);
    if (!import_node_fs5.default.existsSync(dir)) {
      import_node_fs5.default.mkdirSync(dir, { recursive: true });
    }
    import_node_fs5.default.writeFileSync(settingsPath, JSON.stringify(settings, null, 2), "utf-8");
  } catch {
  }
});
import_electron.ipcMain.handle("bgImage:load", async () => {
  try {
    const bgPath = getBgImagePath();
    if (import_node_fs5.default.existsSync(bgPath)) {
      const buffer = await import_node_fs5.default.promises.readFile(bgPath);
      const ext = import_node_path5.default.extname(bgPath).toLowerCase().replace(".", "");
      const mime = IMG_MIME[ext] || ext;
      return { dataUrl: `data:image/${mime};base64,${buffer.toString("base64")}`, path: bgPath };
    }
  } catch {
  }
  return null;
});
import_electron.ipcMain.handle("bgImage:save", async (_event, dataUrl) => {
  try {
    const bgPath = getBgImagePath();
    const dir = import_node_path5.default.dirname(bgPath);
    if (!import_node_fs5.default.existsSync(dir)) {
      import_node_fs5.default.mkdirSync(dir, { recursive: true });
    }
    const base64 = dataUrl.replace(/^data:image\/\w+;base64,/, "");
    const buffer = Buffer.from(base64, "base64");
    import_node_fs5.default.writeFileSync(bgPath, buffer);
    return true;
  } catch {
    return false;
  }
});
import_electron.ipcMain.handle("bgImage:remove", async () => {
  try {
    const bgPath = getBgImagePath();
    if (import_node_fs5.default.existsSync(bgPath)) {
      import_node_fs5.default.unlinkSync(bgPath);
    }
    return true;
  } catch {
    return false;
  }
});
import_electron.ipcMain.handle("clipboard:writeText", async (_event, text) => {
  try {
    import_electron.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
});
import_electron.ipcMain.handle("shell:showItemInFolder", async (_event, filePath) => {
  try {
    import_electron.shell.showItemInFolder(import_node_path5.default.normalize(filePath));
    return true;
  } catch {
    return false;
  }
});
function getFfmpegExe() {
  const isDev = !!process.env.VITE_DEV_SERVER_URL;
  if (isDev) {
    const devPath = import_node_path5.default.join(__dirname, "../ffmpeg.exe");
    if (import_node_fs5.default.existsSync(devPath)) return devPath;
  }
  const prodPath = import_node_path5.default.join(process.resourcesPath, "ffmpeg", "ffmpeg.exe");
  if (import_node_fs5.default.existsSync(prodPath)) return prodPath;
  const exeDir = import_node_path5.default.dirname(import_electron.app.getPath("exe"));
  const fallback = import_node_path5.default.join(exeDir, "ffmpeg.exe");
  if (import_node_fs5.default.existsSync(fallback)) return fallback;
  return "";
}
import_electron.ipcMain.handle("ffmpeg:exec", async (_event, args) => {
  const exe = getFfmpegExe();
  if (!exe) return { code: -1, error: "ffmpeg.exe \u672A\u627E\u5230" };
  return new Promise((resolve) => {
    const child = (0, import_node_child_process.execFile)(exe, args, { timeout: 6e5, maxBuffer: 100 * 1024 * 1024 }, (error, stdout, stderr) => {
      resolve({ code: error ? error.code ?? 1 : 0, stdout, stderr });
    });
  });
});
import_electron.ipcMain.handle("tools:saveFile", async (_event, filePath, base64Data) => {
  try {
    const buffer = Buffer.from(base64Data, "base64");
    import_node_fs5.default.writeFileSync(filePath, buffer);
    return true;
  } catch (e) {
    writeLog2(`[tools:saveFile] error: ${e.message}`);
    return false;
  }
});
import_electron.ipcMain.handle("window:minimize", () => {
  mainWindow?.minimize();
});
import_electron.ipcMain.handle("window:maximize", () => {
  if (mainWindow?.isMaximized()) mainWindow.unmaximize();
  else mainWindow?.maximize();
});
var forceCloseFlag = false;
var tray = null;
function createTray() {
  if (tray) return;
  let icon = null;
  const iconCandidates = [
    import_node_path5.default.join(__dirname, "../public/icon-256.png"),
    import_node_path5.default.join(__dirname, "../public/favicon.ico"),
    import_node_path5.default.join(__dirname, "../../public/icon-256.png"),
    import_node_path5.default.join(__dirname, "../../public/favicon.ico"),
    import_node_path5.default.join(process.resourcesPath || "", "public", "icon-256.png"),
    import_node_path5.default.join(process.resourcesPath || "", "public", "favicon.ico"),
    import_node_path5.default.join(process.resourcesPath || "", "icon-256.png"),
    import_node_path5.default.join(process.resourcesPath || "", "favicon.ico")
  ].filter(Boolean);
  for (const p of iconCandidates) {
    try {
      if (import_node_fs5.default.existsSync(p)) {
        const buf = import_node_fs5.default.readFileSync(p);
        icon = import_electron.nativeImage.createFromBuffer(buf);
        if (icon && !icon.isEmpty()) {
          icon = icon.resize({ width: 32, height: 32 });
          console.log("[tray] icon loaded from:", p);
          break;
        }
      }
    } catch {
    }
  }
  if (!icon || icon.isEmpty()) {
    const size = 16;
    const rgba = Buffer.alloc(size * size * 4, 0);
    const cx = Math.floor(size / 2);
    for (let y = cx - 1; y <= cx; y++) {
      for (let x = cx - 1; x <= cx; x++) {
        const i = (y * size + x) * 4;
        rgba[i] = 64;
        rgba[i + 1] = 128;
        rgba[i + 2] = 255;
        rgba[i + 3] = 255;
      }
    }
    icon = import_electron.nativeImage.createFromBuffer(rgba, { width: size, height: size });
  }
  tray = new import_electron.Tray(icon);
  tray.setToolTip("KX \u97F3\u4E50\u64AD\u653E\u5668");
  const contextMenu = import_electron.Menu.buildFromTemplate([
    {
      label: "\u663E\u793A\u7A97\u53E3",
      click: () => {
        if (mainWindow) {
          mainWindow.show();
          mainWindow.focus();
        }
      }
    },
    { type: "separator" },
    {
      label: "\u9000\u51FA",
      click: () => {
        forceCloseFlag = true;
        import_electron.app.quit();
      }
    }
  ]);
  tray.setContextMenu(contextMenu);
  tray.on("click", () => {
    if (!mainWindow) return;
    if (mainWindow.isVisible()) {
      mainWindow.focus();
    } else {
      mainWindow.show();
      mainWindow.focus();
    }
  });
}
import_electron.ipcMain.handle("window:close", () => {
  if (!mainWindow) return;
  mainWindow.hide();
});
import_electron.ipcMain.handle("window:forceClose", async () => {
  if (!mainWindow) return;
  forceCloseFlag = true;
  try {
    mainWindow.webContents.send("window:beforeClose");
    await new Promise((r) => setTimeout(r, 200));
  } catch {
  }
  mainWindow.close();
});
import_electron.ipcMain.handle("window:isMaximized", () => mainWindow?.isMaximized() ?? false);
import_electron.app.on("before-quit", async () => {
  terminateWorkerPool();
  if (mainWindow && !mainWindow.isDestroyed()) {
    try {
      mainWindow.webContents.send("window:beforeClose");
      await new Promise((r) => setTimeout(r, 200));
    } catch {
    }
  }
});
