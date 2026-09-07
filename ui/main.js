const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const getCurrentWindow = () => window.__TAURI__.window.getCurrentWindow();

let current = null;      // {en, candidates:[...], displayed:{en,zh,note,layer,...}, offline}
let seqId = 0;

function $(id) { return document.getElementById(id); }

// 重置修改态: 隐藏编辑区 + 清空输入。切词/新选择时调, 避免上一次的修改状态带过来
function resetFix() {
  const f = $('fix'); if (!f) return;
  f.classList.remove('on');
  $('fixInput').value = '';
}

function renderHit(t) {
  $('term').textContent = t.en;
  $('zh').textContent = t.zh || '';
  $('note').textContent = t.note || '';
  const chip = $('layer');
  chip.textContent = t.status === 'pending' ? '待确认' :
    (t.layer === 'personal' ? '个人' : t.layer === 'ai' ? 'AI' : t.layer === 'ms' ? '经典' : '云端');
  chip.className = 'chip ' + (t.status === 'pending' ? 'pending' : t.layer);
  $('domain').textContent = t.domain && t.domain !== 'general' ? '[' + t.domain + ']' : '';
  $('cands').innerHTML = '';
  $('loading').textContent = '';
  $('acts').style.display = 'flex';
}

function renderCandidates(cands) {
  const box = $('cands');
  box.innerHTML = '';
  cands.forEach(c => {
    const el = document.createElement('span');
    el.className = 'cand';
    el.textContent = c.en + ' → ' + c.zh + (c.domain !== 'general' ? ' [' + c.domain + ']' : '');
    el.onclick = () => { current.displayed = c; renderHit(c); };
    box.appendChild(el);
  });
}

async function showTerms(terms) {
  // 新 selection, 一定重置修改态 (即便 terms 为空)
  resetFix();
  if (!terms || terms.length === 0) {
    $('term').textContent = '(未提取到英文术语)';
    $('zh').textContent = ''; $('note').textContent = '';
    $('acts').style.display = 'none'; $('cands').innerHTML = '';
    $('loading').textContent = '';
    return;
  }
  current = { terms, idx: 0 };
  await renderTerm(0);
}

async function renderTerm(i) {
  current.idx = i;
  // 切词时也重置: 换词不带着上一次的修改框
  resetFix();
  const en = current.terms[i];
  $('term').textContent = en;
  $('zh').textContent = ''; $('note').textContent = ''; $('cands').innerHTML = '';
  $('domain').textContent = '';
  $('offline').style.display = 'none';
  $('acts').style.display = 'none';
  $('loading').textContent = '查询中…';

  let res;
  try {
    res = await invoke('lookup', { en });
  } catch (e) {
    $('loading').textContent = '本地查询失败: ' + e;
    return;
  }
  const id = ++seqId;

  if (res.hit) {
    $('loading').textContent = '';
    current.displayed = res.hit;
    renderHit(res.hit);
    if (res.candidates && res.candidates.length > 1) renderCandidates(res.candidates);
    return;
  }

  // 本地未命中 → 云端兜底（仅传术语单词，H1）
  $('loading').textContent = '本地未命中，云端兜底…';
  try {
    const cloud = await invoke('fallback', { en });
    if (id !== seqId) return;
    $('loading').textContent = '';
    if (cloud.offline) $('offline').style.display = 'inline';
    if (cloud.result) {
      current.displayed = cloud.result;
      renderHit(cloud.result);
      if (cloud.candidates && cloud.candidates.length > 1) renderCandidates(cloud.candidates);
    } else {
      $('zh').textContent = cloud.error || '无结果';
    }
  } catch (e) {
    if (id !== seqId) return;
    $('loading').textContent = '兜底失败: ' + e;
  }
}

// 动作按钮
$('bAdopt').onclick = async () => {
  if (!current || !current.displayed) return;
  const d = current.displayed;
  await invoke('adopt', { en: d.en, zh: d.zh, domain: d.domain || 'general', note: d.note || '' });
  flash('已沉淀到个人层');
};
$('bFix').onclick = () => {
  const f = $('fix');
  const on = f.classList.toggle('on');
  if (on) { $('fixInput').focus(); } else { $('fixInput').value = ''; }
};
$('bFixOk').onclick = async () => {
  const zh = $('fixInput').value.trim();
  if (!zh || !current) return;
  const d = current.displayed || { en: $('term').textContent };
  await invoke('fix', { en: d.en, zh });
  flash('已按你的译法沉淀');
  resetFix();
};
$('bReject').onclick = async () => {
  if (!current || !current.displayed) return;
  await invoke('reject', { en: current.displayed.en });
  flash('已否决');
};

function flash(msg) {
  $('loading').textContent = msg;
  setTimeout(() => { if ($('loading').textContent === msg) $('loading').textContent = ''; }, 1500);
}

// 多术语切换（右键区域：chips 之外的简单实现——点击 term 循环）
$('term').style.cursor = 'pointer';
$('term').onclick = () => {
  if (current && current.terms && current.terms.length > 1) {
    renderTerm((current.idx + 1) % current.terms.length);
  }
};

// 键位处理
window.addEventListener('keydown', e => {
  // Esc 优先关修改态, 其次隐藏悬浮窗
  if (e.key === 'Escape') {
    if ($('fix').classList.contains('on')) { resetFix(); return; }
    getCurrentWindow().hide();
    return;
  }
  // Ctrl+Enter 在修改态下直接保存 (Enter 留给 textarea 换行)
  if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && $('fix').classList.contains('on')) {
    e.preventDefault();
    $('bFixOk').click();
  }
});

// 初始化事件监听
listen('terms-requested', event => {
  showTerms(event.payload);
});

getCurrentWindow().onFocusChanged(({ payload: focused }) => {
  if (!focused) getCurrentWindow().hide();
});
