const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const getCurrentWindow = () => window.__TAURI__.window.getCurrentWindow();

const POPUP_WIDTH = 360;
const POPUP_MAX_HEIGHT = 640;
const POPUP_MIN_HEIGHT = 120;
const POPUP_MARGIN = 12;
const POPUP_SLACK = 16;
const POPUP_EDGE = 12;

let current = null;
let seqId = 0;
let fitTimer = 0;

function $(id) { return document.getElementById(id); }

function fitWindow() {
  clearTimeout(fitTimer);
  fitTimer = setTimeout(() => { void fitWindowNow(); }, 0);
}

function definitionBlock() {
  const note = $('note');
  const cands = $('cands');
  if (note && note.style.display !== 'none' && note.textContent) return note;
  if (cands && cands.children.length) return cands;
  return null;
}

function clearDefinitionScroll() {
  ['note', 'cands'].forEach(id => {
    const el = $(id);
    if (!el) return;
    el.classList.remove('scroll');
    el.style.maxHeight = '';
  });
}

function workAreaPhysical(monitor, scale) {
  const wa = monitor && monitor.workArea;
  if (wa && wa.position && wa.size) {
    return { x: wa.position.x, y: wa.position.y, w: wa.size.width, h: wa.size.height };
  }
  if (monitor && monitor.position && monitor.size) {
    return { x: monitor.position.x, y: monitor.position.y, w: monitor.size.width, h: monitor.size.height };
  }
  return { x: 0, y: 0, w: 1920 * scale, h: 1080 * scale };
}

async function resolveMonitor(win, cursor) {
  try {
    if (cursor) {
      const m = await win.monitorFromPoint(cursor.x, cursor.y);
      if (m) return m;
    }
  } catch (_) { /* fall through */ }
  try {
    const m = await win.currentMonitor();
    if (m) return m;
  } catch (_) { /* fall through */ }
  try {
    return await win.primaryMonitor();
  } catch (_) {
    return null;
  }
}

function clampToWorkArea(x, y, pw, ph, area, pad) {
  const minX = area.x + pad;
  const minY = area.y + pad;
  const maxX = area.x + area.w - pw - pad;
  const maxY = area.y + area.h - ph - pad;
  return {
    x: Math.min(Math.max(x, minX), Math.max(minX, maxX)),
    y: Math.min(Math.max(y, minY), Math.max(minY, maxY)),
  };
}

async function fitWindowNow() {
  const card = $('card');
  const dpi = window.__TAURI__.dpi;
  if (!card || !dpi || !dpi.LogicalSize) return;
  const win = getCurrentWindow();
  clearDefinitionScroll();
  card.style.maxHeight = 'none';
  const body = definitionBlock();
  const bodyH = body ? body.offsetHeight : 0;
  const chrome = card.offsetHeight - bodyH;
  const natural = Math.ceil(card.offsetHeight + POPUP_MARGIN);
  card.style.maxHeight = '';

  let cursor = null;
  try { cursor = await win.cursorPosition(); } catch (_) { /* no cursor */ }
  const scale = await win.scaleFactor().catch(() => 1);
  const monitor = await resolveMonitor(win, cursor);
  const area = workAreaPhysical(monitor, scale);
  const maxH = Math.max(
    POPUP_MIN_HEIGHT,
    Math.min(POPUP_MAX_HEIGHT, Math.floor((area.h / scale) * 2 / 3))
  );
  const height = Math.max(POPUP_MIN_HEIGHT, Math.min(natural + POPUP_SLACK, maxH));
  let applied = height;
  try {
    applied = await invoke('place_overlay', { width: POPUP_WIDTH, height });
  } catch (_) {
    try {
      await win.setSize(new dpi.LogicalSize(POPUP_WIDTH, height));
    } catch (_) {
      return;
    }
    if (dpi.PhysicalPosition) {
      try {
        const [pos, size] = await Promise.all([win.outerPosition(), win.outerSize()]);
        const pad = POPUP_EDGE * scale;
        let x = pos.x;
        let y = pos.y;
        if (cursor && y + size.height > area.y + area.h - pad) {
          const above = cursor.y - size.height - pad;
          if (above >= area.y + pad) y = above;
        }
        if (cursor && x + size.width > area.x + area.w - pad) {
          const left = cursor.x - size.width - pad;
          if (left >= area.x + pad) x = left;
        }
        const next = clampToWorkArea(x, y, size.width, size.height, area, pad);
        if (next.x !== pos.x || next.y !== pos.y) {
          await win.setPosition(new dpi.PhysicalPosition(next.x, next.y));
        }
      } catch (_) { /* keep current position */ }
    }
  }
  if (body && natural + POPUP_SLACK > applied) {
    body.style.maxHeight = Math.max(48, applied - POPUP_MARGIN - chrome) + 'px';
    body.classList.add('scroll');
  }
}

function resetFix() {
  const f = $('fix'); if (!f) return;
  f.classList.remove('on');
  $('fixInput').value = '';
}

function hideHint() {
  const h = $('hint');
  if (h) h.classList.add('off');
}

function showSinglePane() {
  $('zh').style.display = '';
  $('note').style.display = '';
}

function hideSinglePane() {
  $('zh').textContent = '';
  $('note').textContent = '';
  $('zh').style.display = 'none';
  $('note').style.display = 'none';
}

function layerLabel(t) {
  if (t.status === 'pending') return '待确认';
  if (t.layer === 'personal') return '个人';
  if (t.layer === 'ai') return 'AI';
  if (t.layer === 'ms') return '经典';
  return t.layer || '';
}

function setHeadMeta(t) {
  const chip = $('layer');
  if (!t) {
    chip.textContent = '多义';
    chip.className = 'chip';
    $('domain').textContent = '';
    return;
  }
  chip.textContent = layerLabel(t);
  chip.className = 'chip ' + (t.status === 'pending' ? 'pending' : t.layer || '');
  $('domain').textContent = t.domain && t.domain !== 'general' ? '[' + t.domain + ']' : '';
}

function renderHit(t) {
  hideHint();
  showSinglePane();
  $('term').textContent = t.en;
  $('zh').textContent = t.zh || '';
  $('note').textContent = t.note || '';
  setHeadMeta(t);
  $('cands').innerHTML = '';
  $('loading').textContent = '';
  $('acts').style.display = 'flex';
  fitWindow();
}

function sameSense(a, b) {
  return a && b && a.en === b.en && a.zh === b.zh && a.domain === b.domain && a.layer === b.layer;
}

function renderSenseList(cands, preferred) {
  hideHint();
  hideSinglePane();
  setHeadMeta(preferred || null);
  const box = $('cands');
  box.innerHTML = '';
  cands.forEach(c => {
    const row = document.createElement('div');
    const preferredRow = sameSense(c, preferred);
    row.className = 'sense' + (preferredRow ? ' preferred' : '');
    const zh = document.createElement('div');
    zh.className = 'sense-zh';
    zh.textContent = c.zh || '';
    row.appendChild(zh);
    const bits = [];
    if (c.domain && c.domain !== 'general') bits.push(c.domain);
    const layer = layerLabel(c);
    if (layer) bits.push(layer);
    if (preferredRow) bits.push('当前语境');
    if (bits.length) {
      const meta = document.createElement('div');
      meta.className = 'sense-meta';
      meta.textContent = bits.join(' · ');
      row.appendChild(meta);
    }
    if (c.note) {
      const note = document.createElement('div');
      note.className = 'sense-note';
      note.textContent = c.note;
      row.appendChild(note);
    }
    box.appendChild(row);
  });
  $('loading').textContent = '';
  $('acts').style.display = 'flex';
  fitWindow();
}

async function showTerms(terms) {
  const id = ++seqId;
  resetFix();
  hideHint();
  if (!terms || terms.length === 0) {
    showSinglePane();
    $('term').textContent = '(未提取到英文术语)';
    $('zh').textContent = ''; $('note').textContent = '';
    $('acts').style.display = 'none'; $('cands').innerHTML = '';
    $('loading').textContent = '';
    fitWindow();
    return;
  }
  current = { terms, idx: 0 };
  await renderTerm(0, id);
}

function showSelectionFailed(msg) {
  const id = ++seqId;
  resetFix();
  hideHint();
  current = null;
  showSinglePane();
  $('term').textContent = '未取到选区';
  $('zh').textContent = '';
  $('note').textContent = msg || '未能读取当前选区。请先划选再取词；终端等应用暂不支持。';
  $('acts').style.display = 'none';
  $('cands').innerHTML = '';
  $('loading').textContent = '';
  $('offline').style.display = 'none';
  $('layer').textContent = '';
  $('domain').textContent = '';
  fitWindow();
  return id;
}

async function renderTerm(i, incomingId) {
  const id = incomingId === undefined ? ++seqId : incomingId;
  if (id !== seqId) return;
  current.idx = i;
  resetFix();
  hideHint();
  const en = current.terms[i];
  showSinglePane();
  $('term').textContent = en;
  $('zh').textContent = ''; $('note').textContent = ''; $('cands').innerHTML = '';
  $('domain').textContent = '';
  $('offline').style.display = 'none';
  $('acts').style.display = 'none';
  $('loading').textContent = '查询中…';
  fitWindow();

  let res;
  try {
    res = await invoke('lookup', { en });
  } catch (e) {
    if (id !== seqId) return;
    $('loading').textContent = '本地查询失败: ' + e;
    fitWindow();
    return;
  }
  if (id !== seqId) return;

  if (res.candidates && res.candidates.length > 1) {
    current.displayed = res.hit || null;
    renderSenseList(res.candidates, res.hit || null);
    return;
  }

  if (res.hit) {
    current.displayed = res.hit;
    renderHit(res.hit);
    return;
  }

  $('loading').textContent = '本地未命中，云端兜底…';
  try {
    const cloud = await invoke('fallback', { en });
    if (id !== seqId) return;
    $('loading').textContent = '';
    if (cloud.offline) $('offline').style.display = 'inline';
    if (cloud.result) {
      current.displayed = cloud.result;
      if (cloud.candidates && cloud.candidates.length > 1) {
        renderSenseList(cloud.candidates, cloud.result);
      } else {
        renderHit(cloud.result);
      }
    } else {
      $('zh').textContent = cloud.error || '无结果';
      fitWindow();
    }
  } catch (e) {
    if (id !== seqId) return;
    $('loading').textContent = '兜底失败: ' + e;
    fitWindow();
  }
}

$('bAdopt').onclick = async () => {
  if (!current || !current.displayed) return;
  const d = current.displayed;
  try {
    await invoke('adopt', { en: d.en, zh: d.zh, domain: d.domain || 'general', note: d.note || '' });
    flash('已沉淀到个人层');
  } catch (e) {
    flash('采纳失败: ' + e);
  }
};
$('bFix').onclick = () => {
  const f = $('fix');
  const on = f.classList.toggle('on');
  if (on) { $('fixInput').focus(); } else { $('fixInput').value = ''; }
  fitWindow();
};
$('bFixOk').onclick = async () => {
  const zh = $('fixInput').value.trim();
  if (!zh || !current) return;
  const d = current.displayed || { en: $('term').textContent };
  try {
    await invoke('fix', { en: d.en, zh });
    flash('已按你的译法沉淀');
    resetFix();
    fitWindow();
  } catch (e) {
    flash('修改失败: ' + e);
  }
};
$('bReject').onclick = async () => {
  if (!current || !current.displayed) return;
  try {
    await invoke('reject', { en: current.displayed.en });
    flash('已否决');
  } catch (e) {
    flash('否决失败: ' + e);
  }
};

function flash(msg) {
  $('loading').textContent = msg;
  setTimeout(() => { if ($('loading').textContent === msg) $('loading').textContent = ''; }, 1500);
}

$('term').style.cursor = 'pointer';
$('term').onclick = () => {
  if (current && current.terms && current.terms.length > 1) {
    renderTerm((current.idx + 1) % current.terms.length);
  }
};

window.addEventListener('keydown', e => {
  if (e.key === 'Escape') {
    if ($('fix').classList.contains('on')) { resetFix(); fitWindow(); return; }
    getCurrentWindow().hide();
    return;
  }
  if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && $('fix').classList.contains('on')) {
    e.preventDefault();
    $('bFixOk').click();
  }
});

listen('terms-requested', event => {
  showTerms(event.payload);
});

listen('selection-failed', event => {
  showSelectionFailed(event.payload);
});

getCurrentWindow().onFocusChanged(({ payload: focused }) => {
  if (!focused) getCurrentWindow().hide();
});
