var __create = Object.create;
var __defProp = Object.defineProperty;
var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
var __getOwnPropNames = Object.getOwnPropertyNames;
var __getProtoOf = Object.getPrototypeOf;
var __hasOwnProp = Object.prototype.hasOwnProperty;
var __export = (target, all) => {
  for (var name in all)
    __defProp(target, name, { get: all[name], enumerable: true });
};
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
var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);
var stdin_exports = {};
__export(stdin_exports, {
  initMemoryMonitor: () => initMemoryMonitor,
  markMain: () => markMain,
  reportRendererSample: () => reportRendererSample,
  stopMemoryMonitor: () => stopMemoryMonitor
});
module.exports = __toCommonJS(stdin_exports);
var import_node_fs = __toESM(require("node:fs"));
var import_node_path = __toESM(require("node:path"));
const LOG_FILE_NAME = "memory.log";
const MAX_LOG_BYTES = 2 * 1024 * 1024;
const KEEP_BYTES = 1024 * 1024;
const SAMPLE_INTERVAL_MS = 1e4;
let logPath = null;
let lastSampleAt = { main: 0 };
let intervalHandle = null;
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
    if (import_node_fs.default.existsSync(logPath)) {
      const stat = import_node_fs.default.statSync(logPath);
      if (stat.size > MAX_LOG_BYTES) {
        const content = import_node_fs.default.readFileSync(logPath, "utf-8");
        const keepFrom = Math.max(0, content.length - KEEP_BYTES);
        import_node_fs.default.writeFileSync(logPath, "// --- rotated ---\n" + content.slice(keepFrom) + line, "utf-8");
        return;
      }
    }
    import_node_fs.default.appendFileSync(logPath, line, "utf-8");
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
  if (!import_node_fs.default.existsSync(targetDir)) import_node_fs.default.mkdirSync(targetDir, { recursive: true });
  logPath = import_node_path.default.join(targetDir, LOG_FILE_NAME);
  try {
    import_node_fs.default.writeFileSync(logPath, `// memory monitor started at ${nowIso()} (pid=${process.pid})
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
function stopMemoryMonitor() {
  if (intervalHandle) {
    clearInterval(intervalHandle);
    intervalHandle = null;
  }
}
// Annotate the CommonJS export names for ESM import in node:
0 && (module.exports = {
  initMemoryMonitor,
  markMain,
  reportRendererSample,
  stopMemoryMonitor
});
