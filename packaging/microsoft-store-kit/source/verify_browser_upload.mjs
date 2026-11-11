// Verify folder-relative CSV references against the files selected in a disposable
// Chrome directory picker. This does not connect to or import into Partner Center.
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const kit = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const profile = await mkdtemp(path.join(tmpdir(), 'electrocraft-store-chrome-'));
const chrome = spawn('google-chrome', ['--headless=new', '--no-sandbox', '--disable-gpu',
  '--disable-dev-shm-usage', '--no-first-run', '--remote-debugging-port=0',
  `--user-data-dir=${profile}`, 'about:blank'], { stdio: 'ignore' });
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
let ws;
try {
  let port;
  for (let i = 0; i < 100; i++) {
    try { port = Number((await readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]); break; }
    catch { await pause(100); }
  }
  if (!port) throw new Error('Chrome did not open its debugging endpoint');
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  ws = new WebSocket(targets.find(t => t.type === 'page').webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let id = 0;
  const pending = new Map();
  ws.onmessage = ({ data }) => {
    const message = JSON.parse(data);
    const request = pending.get(message.id);
    if (!request) return;
    clearTimeout(request.timer);
    pending.delete(message.id);
    if (message.error) request.reject(new Error(JSON.stringify(message.error)));
    else request.resolve(message.result);
  };
  const call = (method, params = {}) => new Promise((resolve, reject) => {
    const key = ++id;
    const timer = setTimeout(() => { pending.delete(key); reject(new Error(`${method} timed out`)); }, 15000);
    pending.set(key, { resolve, reject, timer });
    ws.send(JSON.stringify({ id: key, method, params }));
  });
  const reports = [];
  for (const name of ['microsoft-store', 'microsoft-store-10']) {
    await call('Runtime.evaluate', { expression: 'document.body.innerHTML = `<input id="folder" type="file" webkitdirectory multiple>`' });
    const { root } = await call('DOM.getDocument');
    const { nodeId } = await call('DOM.querySelector', { nodeId: root.nodeId, selector: '#folder' });
    await call('DOM.setFileInputFiles', { nodeId, files: [path.join(kit, '..', name)] });
    for (let i = 0; i < 100; i++) {
      const state = await call('Runtime.evaluate', { expression: 'document.querySelector("#folder").files.length', returnByValue: true });
      if (state.result.value > 0) break;
      await pause(100);
    }
    const result = await call('Runtime.evaluate', { awaitPromise: true, returnByValue: true,
      expression: ` (async () => {
        const files = Array.from(document.querySelector('#folder').files);
        const csvs = files.filter(f => f.name.endsWith('.csv'));
        if (csvs.length !== 1) throw new Error('Expected one uploaded CSV, got ' + csvs.length);
        const csv = (await csvs[0].text()).replace(/^\uFEFF/, '');
        const rows = []; let row = [], cell = '', quoted = false;
        for (let i = 0; i < csv.length; i++) {
          const c = csv[i];
          if (c === '"') {
            if (quoted && csv[i + 1] === '"') { cell += '"'; i++; } else quoted = !quoted;
          } else if (c === ',' && !quoted) { row.push(cell); cell = ''; }
          else if ((c === '\\r' || c === '\\n') && !quoted) {
            row.push(cell); rows.push(row); row = []; cell = '';
            if (c === '\\r' && csv[i+1] === '\\n') i++;
          } else cell += c;
        }
        if (cell || row.length) { row.push(cell); rows.push(row); }
        const byPath = new Map(files.map(file => [file.webkitRelativePath, file]));
        const refs = new Set();
        for (const row of rows.slice(1)) {
          if (!row[2]?.startsWith('Relative path')) continue;
          for (const value of row.slice(3).filter(Boolean)) {
            if (!byPath.has(value)) throw new Error('CSV asset not in browser FileList: ' + value);
            refs.add(value);
          }
        }
        if (refs.size !== files.length - 1) throw new Error('Unexpected unreferenced files');
        for (const ref of refs) {
          const bytes = new Uint8Array(await byPath.get(ref).slice(0,8).arrayBuffer());
          if (Array.from(bytes).join(',') !== '137,80,78,71,13,10,26,10') throw new Error('PNG cannot be read: '+ref);
        }
        return {status:'passed', files:files.length, csvs:csvs.length, rows:rows.length,
          imageReferences:refs.size, relativePaths:Array.from(byPath.keys()).sort()};
      })()` });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    reports.push({ folder: name, ...result.result.value });
    console.log(`${name}: ${result.result.value.files} browser files, ${result.result.value.imageReferences} readable image references; passed`);
  }
  await writeFile(path.join(kit, 'browser-upload-validation.json'), JSON.stringify({
    scope: 'Actual Chrome directory selection and file reads; Partner Center acceptance has not been tested', reports
  }, null, 2) + '\n');
} finally {
  ws?.close();
  const stopped = new Promise(resolve => chrome.once('exit', resolve));
  chrome.kill('SIGTERM');
  await Promise.race([stopped, pause(3000)]);
  await rm(profile, { recursive: true, force: true });
}
