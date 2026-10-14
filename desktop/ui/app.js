import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

const $ = (id) => document.getElementById(id);
const statusBox = $('status');
const summaryBox = $('summary');
const groupsBox = $('groups');
const scanBtn = $('scanBtn');
const singleMode = $('singleMode');
const compareMode = $('compareMode');
const singleFields = $('singleFields');
const compareFields = $('compareFields');
const warning = $('warning');
let activeMode = 'single';

function bytes(n) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = Number(n || 0);
  for (const unit of units) {
    if (value < 1024 || unit === 'TB') return unit === 'B' ? `${value} B` : `${value.toFixed(1)} ${unit}`;
    value /= 1024;
  }
}

function normalizePath(value) {
  return String(value || '').trim().replace(/\/$/, '');
}

function setMode(mode) {
  activeMode = mode;
  singleMode.classList.toggle('active', mode === 'single');
  compareMode.classList.toggle('active', mode === 'compare');
  singleFields.classList.toggle('hidden', mode !== 'single');
  compareFields.classList.toggle('hidden', mode !== 'compare');
  statusBox.textContent = mode === 'single'
    ? 'Single-folder scan: DupeLens finds duplicate groups inside this folder.'
    : 'Compare mode: primary is protected; backup/import side is where duplicate copies are candidates.';
  validateInputs();
}

function setStatus(text, detail = '') {
  statusBox.innerHTML = `<strong>${escapeHtml(text)}</strong>${detail ? `<pre>${escapeHtml(detail)}</pre>` : ''}`;
}

function setScanning(isScanning) {
  document.body.classList.toggle('isScanning', isScanning);
  scanBtn.disabled = isScanning;
  scanBtn.textContent = isScanning ? 'Scanning…' : 'Scan';
}

function nextPaint() {
  return new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
}

function metric(label, value) {
  return `<div class="metric"><strong>${value}</strong><span>${label}</span></div>`;
}

function emptyMessage(result) {
  if (result.mode === 'compare') {
    return 'No duplicates were found across those two folders. If you used the same path twice, switch to “Scan one folder”.';
  }
  return 'No duplicate groups found in this folder with the current depth setting.';
}

function roleText(file, result) {
  if (file.role === 'keep') return result.mode === 'compare' ? 'KEEP — primary/protected copy' : 'KEEP — one copy retained';
  return result.mode === 'compare' ? 'TRASH CANDIDATE — backup/import copy' : 'TRASH CANDIDATE — extra copy';
}

function renderResult(result) {
  const s = result.summary || {};
  summaryBox.classList.remove('hidden');
  summaryBox.innerHTML = [
    metric('duplicate groups', s.groups || 0),
    metric('files in groups', s.files || 0),
    metric('reclaimable', bytes(s.reclaimable_bytes || 0)),
    metric('elapsed', `${Number(s.elapsed_seconds || 0).toFixed(2)}s`),
  ].join('');

  const groups = result.groups || [];
  if (!groups.length) {
    groupsBox.innerHTML = `<section class="card muted">${escapeHtml(emptyMessage(result))}</section>`;
    return;
  }
  groupsBox.innerHTML = groups.map((group, index) => `
    <article class="group">
      <div class="groupHead">
        <div>
          <strong>Group ${index + 1}</strong>
          <div class="meta">${group.files.length} files · ${bytes(group.size)} each · ${bytes(group.reclaimable_bytes)} reclaimable</div>
        </div>
        <span class="badge">${escapeHtml(group.id)}</span>
      </div>
      ${group.files.map((file) => `
        <div class="file">
          <div class="role ${file.role}">${escapeHtml(roleText(file, result))}</div>
          <div class="path">${escapeHtml(file.path)}</div>
          <div class="meta">${bytes(file.size)} · id ${escapeHtml(file.id)}</div>
        </div>
      `).join('')}
    </article>
  `).join('');
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (ch) => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#039;'}[ch]));
}

function validateInputs() {
  warning.classList.add('hidden');
  warning.textContent = '';
  scanBtn.disabled = false;
  const primary = activeMode === 'single' ? normalizePath($('singlePath').value) : normalizePath($('primary').value);
  if (!primary) {
    scanBtn.disabled = true;
    return false;
  }
  if (activeMode === 'compare') {
    const backup = normalizePath($('backup').value);
    if (!backup) {
      scanBtn.disabled = true;
      return false;
    }
    if (primary === backup) {
      warning.textContent = 'You selected the same folder twice. Use “Scan one folder” for this, or choose a different backup/import folder.';
      warning.classList.remove('hidden');
      scanBtn.disabled = true;
      return false;
    }
  }
  return true;
}

scanBtn.addEventListener('click', async () => {
  summaryBox.classList.add('hidden');
  groupsBox.innerHTML = '';
  if (!validateInputs()) return;
  setScanning(true);
  setStatus('Scanning…', 'Rust core is checking exact duplicates. Nothing will be deleted.');
  try {
    const payload = {
      mode: activeMode,
      primary: activeMode === 'single' ? $('singlePath').value : $('primary').value,
      backup: $('backup').value || null,
      scan_mode: $('scanMode').value,
      recursive: $('recursive').checked,
      include_hidden: $('includeHidden').checked,
    };
    await nextPaint();
    const result = await invoke('scan_duplicates', { request: payload });
    const modeText = result.mode === 'compare'
      ? 'Compare scan complete. Primary folder stayed protected.'
      : 'Folder scan complete. Showing duplicate groups inside this folder.';
    setStatus(modeText);
    renderResult(result);
  } catch (err) {
    setStatus('Scan failed', err.message || String(err));
  } finally {
    setScanning(false);
    validateInputs();
  }
});

async function pickFolder(targetId) {
  setStatus('Choose a folder…', 'A native folder picker should open.');
  try {
    const selected = await open({ directory: true, multiple: false });
    if (!selected) {
      setStatus('Picker cancelled');
      return;
    }
    $(targetId).value = selected;
    setStatus('Folder selected', selected);
    validateInputs();
  } catch (err) {
    setStatus('Picker failed', err.message || String(err));
  }
}

singleMode.addEventListener('click', () => setMode('single'));
compareMode.addEventListener('click', () => setMode('compare'));
['singlePath', 'primary', 'backup'].forEach((id) => $(id).addEventListener('input', validateInputs));
document.querySelectorAll('[data-pick]').forEach((button) => {
  button.addEventListener('click', () => pickFolder(button.dataset.pick));
});

setMode('single');
