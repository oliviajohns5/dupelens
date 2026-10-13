const $ = (id) => document.getElementById(id);
const statusBox = $('status');
const summaryBox = $('summary');
const groupsBox = $('groups');
const scanBtn = $('scanBtn');

function bytes(n) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let value = Number(n || 0);
  for (const unit of units) {
    if (value < 1024 || unit === 'TB') return unit === 'B' ? `${value} B` : `${value.toFixed(1)} ${unit}`;
    value /= 1024;
  }
}

function setStatus(text, detail = '') {
  statusBox.innerHTML = `<strong>${text}</strong>${detail ? `<pre>${detail}</pre>` : ''}`;
}

function metric(label, value) {
  return `<div class="metric"><strong>${value}</strong><span>${label}</span></div>`;
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
    groupsBox.innerHTML = '<section class="card muted">No cross-root duplicates found. Primary folder remains untouched.</section>';
    return;
  }
  groupsBox.innerHTML = groups.map((group, index) => `
    <article class="group">
      <div class="groupHead">
        <div>
          <strong>Group ${index + 1}</strong>
          <div class="meta">${group.files.length} files · ${bytes(group.size)} each · ${bytes(group.reclaimable_bytes)} reclaimable</div>
        </div>
        <span class="badge">${group.id}</span>
      </div>
      ${group.files.map((file) => `
        <div class="file">
          <div class="role ${file.role}">${file.role === 'keep' ? 'KEEP — protected primary' : 'TRASH CANDIDATE — backup/import copy'}</div>
          <div class="path">${escapeHtml(file.path)}</div>
          <div class="meta">${bytes(file.size)} · id ${file.id}</div>
        </div>
      `).join('')}
    </article>
  `).join('');
}

function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, (ch) => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#039;'}[ch]));
}

async function poll(jobId) {
  while (true) {
    const res = await fetch(`/api/jobs/${jobId}`);
    const job = await res.json();
    if (!res.ok) throw new Error(job.error || 'job not found');
    const events = job.events || [];
    const last = events[events.length - 1];
    if (job.status === 'running') {
      const detail = last ? JSON.stringify(last, null, 2) : 'Starting...';
      setStatus('Scanning...', detail);
      await new Promise((resolve) => setTimeout(resolve, 500));
      continue;
    }
    if (job.status === 'failed') {
      setStatus('Scan failed', job.error || 'Unknown error');
      scanBtn.disabled = false;
      return;
    }
    setStatus('Scan complete', `Primary root protected. Showing KEEP/TRASH recommendations.`);
    renderResult(job.result);
    scanBtn.disabled = false;
    return;
  }
}

scanBtn.addEventListener('click', async () => {
  summaryBox.classList.add('hidden');
  groupsBox.innerHTML = '';
  scanBtn.disabled = true;
  setStatus('Starting scan...');
  try {
    const payload = {
      primary: $('primary').value,
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
  }
});

fetch('/api/health')
  .then((r) => r.json())
  .then((h) => {
    if (!h.core_exists) setStatus('Core binary missing', `Run: cd core && cargo build --release\nExpected: ${h.core}`);
  })
  .catch(() => {});
