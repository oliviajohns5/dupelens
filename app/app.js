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

function eventLine(event) {
  if (!event) return 'Starting...';
  if (event.event === 'progress') {
    const total = event.total ? `/${event.total}` : '';
    const done = Number.isFinite(event.done) ? `${event.done}${total}` : '';
    return `${event.phase || 'Scanning'} ${done}`.trim();
  }
  if (event.event === 'phase_done') return event.message || 'Phase complete';
  if (event.event === 'done') return `${event.groups || 0} groups · ${bytes(event.reclaimable_bytes || 0)} reclaimable`;
  return event.message || event.event || '';
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
  if (activeMode === 'compare') {
    const primary = normalizePath($('primary').value);
    const backup = normalizePath($('backup').value);
    if (primary && backup && primary === backup) {
      warning.textContent = 'You selected the same folder twice. Use “Scan one folder” for this, or choose a different backup/import folder.';
      warning.classList.remove('hidden');
      scanBtn.disabled = true;
      return false;
    }
  }
  return true;
}

async function poll(jobId) {
  while (true) {
    const res = await fetch(`/api/jobs/${jobId}`);
    const job = await res.json();
    if (!res.ok) throw new Error(job.error || 'job not found');
    const events = job.events || [];
    const last = events[events.length - 1];
    if (job.status === 'running') {
      setStatus('Scanning…', eventLine(last));
      await new Promise((resolve) => setTimeout(resolve, 500));
      continue;
    }
    if (job.status === 'failed') {
      setStatus('Scan failed', job.error || 'Unknown error');
      scanBtn.disabled = false;
      validateInputs();
      return;
    }
    const modeText = job.result.mode === 'compare'
      ? 'Compare scan complete. Primary folder stayed protected.'
      : 'Folder scan complete. Showing duplicate groups inside this folder.';
    setStatus(modeText);
    renderResult(job.result);
    scanBtn.disabled = false;
    validateInputs();
    return;
  }
}

scanBtn.addEventListener('click', async () => {
  summaryBox.classList.add('hidden');
  groupsBox.innerHTML = '';
  if (!validateInputs()) return;
  scanBtn.disabled = true;
  setStatus('Starting scan...');
  try {
    const payload = {
      mode: activeMode,
      primary: activeMode === 'single' ? $('singlePath').value : $('primary').value,
      backup: $('backup').value,
      scan_mode: $('scanMode').value,
      recursive: $('recursive').checked,
      include_hidden: $('includeHidden').checked,
    };
    const res = await fetch('/api/scan', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify(payload),
    });
    const data = await res.json();
    if (!res.ok) throw new Error(data.error || 'Could not start scan');
    await poll(data.job_id);
  } catch (err) {
    setStatus('Error', err.message || String(err));
    scanBtn.disabled = false;
    validateInputs();
  }
});

singleMode.addEventListener('click', () => setMode('single'));
compareMode.addEventListener('click', () => setMode('compare'));
['singlePath', 'primary', 'backup'].forEach((id) => $(id).addEventListener('input', validateInputs));

fetch('/api/health')
  .then((r) => r.json())
  .then((h) => {
    if (!h.core_exists) setStatus('Core binary missing', `Run: cd core && cargo build --release\nExpected: ${h.core}`);
  })
  .catch(() => {});

setMode('single');
