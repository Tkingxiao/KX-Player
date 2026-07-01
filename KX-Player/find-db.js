const fs = require('fs');
const path = require('path');

const searchPaths = [
  'C:/Users/12997/AppData/Roaming',
  'C:/Users/12997/AppData/Local',
  process.cwd()
];

for (const base of searchPaths) {
  walk(base, 0);
}

function walk(dir, depth) {
  if (depth > 5) return;
  let entries;
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const e of entries) {
    if (e.isDirectory() && !e.name.startsWith('.')) {
      walk(path.join(dir, e.name), depth + 1);
    } else if (e.name.endsWith('.sqlite') || (e.name.includes('kx-player') && e.name.endsWith('.json'))) {
      const fp = path.join(dir, e.name);
      const stat = fs.statSync(fp);
      if (stat.size > 100) console.log(fp, stat.size, 'bytes');
    }
  }
}
