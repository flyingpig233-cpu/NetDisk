import { api } from './api';
import { icon } from './icons';

const esc = value => String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
const formatDate = seconds => new Date(seconds * 1000).toLocaleString('zh-CN', { hour12: false });
const size = n => n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(1)} KB` : `${(n / 1048576).toFixed(1)} MB`;
const link = code => `${location.origin}${location.pathname}${location.search}#/share/${code}`;
const button = (action, label, symbol = 'share', extra = '', cls = 'secondary-button') => `<button type="button" class="${cls}" data-share-action="${action}" ${extra}>${icon(symbol)}<span>${esc(label)}</span></button>`;
let hooks;
let receiver = { key: '', code: '', files: [], busy: false, error: '', version: 0 };
const downloading = new Set();

export const isShareRoute = () => location.hash.startsWith('#/share');
export function resetShareReceiver() { receiver.key = ''; receiver.version++; }
async function copy(value) {
  try { await navigator.clipboard.writeText(value); hooks.toast('已复制'); }
  catch {
    hooks.modal('手动复制', `<p class="dialog-description">请选中下方内容复制。</p><label class="form-label">分享内容<input value="${esc(value)}" readonly></label>`, () => {}, '完成');
    document.querySelector('#dialog input').select();
  }
}
function result(record) {
  hooks.modal('分享已创建', `<div class="share-success">${icon('check')}<p>把这份收藏，分享给需要的人。</p></div><p class="share-caption">分享码</p><div class="share-code">${esc(record.code)}</div><p class="share-caption">${record.expires ? `有效至 ${esc(formatDate(record.expires))}` : '永久有效'} · 接收者需要登录</p><label class="form-label">分享链接<input value="${esc(link(record.code))}" readonly></label><div class="share-result-actions">${button('copy-link', '复制链接', 'share', `data-code="${record.code}"`)}${button('copy-code', '复制分享码', 'doc', `data-code="${record.code}"`)}</div>`, () => { location.hash = `/share/${record.code}`; }, '查看分享');
}
export function createShare(files) {
  if (!hooks.user()) { hooks.toast('请先登录个人空间，再分享真实文件'); hooks.login(); return; }
  if (!files.length) { hooks.toast('请先选择要分享的文件'); return; }
  const owner = hooks.user().user_id;
  hooks.modal(`分享 ${files.length} 个项目`, `<p class="dialog-description">生成一个分享码，接收者登录后即可下载。文件夹将作为 ZIP 下载。</p><div class="share-picked">${files.map(f => `<div>${icon(f.is_directory ? 'folder' : 'doc')}<span>${esc(f.file_name)}</span></div>`).join('')}</div><label class="form-label">有效期<select name="duration"><option value="86400">1 天</option><option value="604800" selected>7 天</option><option value="2592000">30 天</option><option value="custom">自定义时间</option><option value="forever">永久有效</option></select></label><label class="form-label" id="share-custom" hidden>到期时间<input name="expiration" type="datetime-local"></label><p class="dialog-description">原文件删除后，分享中的对应文件也将失效。</p>`, async form => {
    let expires = null;
    const duration = form.get('duration');
    if (duration === 'custom') {
      expires = Math.floor(new Date(form.get('expiration')).getTime() / 1000);
      if (!Number.isFinite(expires) || expires <= Date.now() / 1000) throw new Error('请选择未来的到期时间');
    } else if (duration !== 'forever') expires = Math.floor(Date.now() / 1000) + Number(duration);
    const response = await api.share(files.map(f => f.file_id), expires);
    if (!/^\d{6}$/.test(response?.share_code)) throw new Error('分享接口返回了无效的分享码');
    const record = { code: response.share_code, names: files.map(f => f.file_name), created: Math.floor(Date.now() / 1000), expires };
    if (hooks.user()?.user_id !== owner) return false;
    await hooks.refreshShares();
    hooks.render(); result(record); return false;
  }, '创建分享');
  const select = document.querySelector('#dialog select');
  select.onchange = () => {
    const custom = document.querySelector('#share-custom'); custom.hidden = select.value !== 'custom';
    custom.querySelector('input').required = select.value === 'custom';
  };
}

export function shareHistory(query = '') {
  const history = hooks.history();
  const filtered = history.records.filter(r => `${r.code} ${r.names.join(' ')}`.toLowerCase().includes(query.toLowerCase()));
  return `<section class="files-section share-history"><div class="files-heading"><div class="heading-left"><h2>${history.all ? '全部用户分享' : '分享列表'}</h2><span class="count">${filtered.length} 条</span></div>${button('extract', '提取分享', 'download')}</div><p class="share-history-note">分享列表与服务器同步。撤销后链接立即失效，原文件保留。</p>${history.error ? `<div class="error-state" role="alert">${esc(history.error)}${button('refresh-history', '重试', 'clock')}</div>` : ''}${history.busy ? '<div class="loading-state" role="status"><span class="spinner"></span>正在读取分享…</div>' : filtered.length ? `<div class="share-records">${filtered.map(r => {
    const expired = r.expires != null && r.expires <= Date.now() / 1000;
    const unavailable = !r.names.length;
    return `<article class="share-record"><div class="share-record-icon">${icon('share')}</div><div class="share-record-content"><h3>${esc(r.names.join('、') || '原文件已删除')}</h3><p>分享码 <strong>${esc(r.code)}</strong> <span class="share-status ${expired || unavailable ? 'expired' : ''}">${expired ? '已到期' : unavailable ? '无可用文件' : r.expires ? '有效期内' : '永久有效'}</span></p><small>${r.expires ? `到期：${esc(formatDate(r.expires))}` : '无到期时间'} · 创建：${esc(formatDate(r.created))}${history.all ? ` · 创建者：${esc(r.owner_id || '历史分享（创建者未知）')}` : ''}</small></div><div class="share-record-actions">${button('copy-link', '复制链接', 'share', `data-code="${esc(r.code)}"`)}${button('visit', '查看', 'arrow', `data-code="${esc(r.code)}"`)}${button('revoke', '撤销分享', 'trash', `data-id="${esc(r.share_id)}"`, 'secondary-button danger')}</div></article>`;
  }).join('')}</div>` : history.error ? '' : `<div class="empty-state"><span>${icon('share')}</span><h3>${query ? '没有匹配的分享' : '还没有分享'}</h3><p>选择文件或文件夹即可创建分享。</p>${button('home', '前往我的文件', 'folder')}</div>`}</section>`;
}
function revoke(id) {
  const record = hooks.history().records.find(r => r.share_id === id);
  if (!record) return;
  const userId = hooks.user()?.user_id;
  hooks.modal('确认撤销分享？', `<p class="dialog-description">分享码 <strong>${esc(record.code)}</strong> 对应的链接将立即失效。原文件会保留，此操作无法撤销。</p>`, async () => {
    await api.revokeShare(id);
    if (hooks.user()?.user_id !== userId) return;
    resetShareReceiver();
    await hooks.refreshShares();
    hooks.toast('分享已撤销');
  }, '撤销分享');
}

async function load(code, key) {
  const version = ++receiver.version;
  receiver.busy = true; receiver.error = ''; receiver.files = [];
  try {
    const files = await api.sharedFiles(code);
    if (!Array.isArray(files)) throw new Error('分享接口返回格式有误');
    if (receiver.key !== key || receiver.version !== version) return;
    receiver.files = files;
  } catch (error) {
    if (receiver.key !== key || receiver.version !== version) return;
    receiver.error = error.status === 401 ? '当前登录已失效，请重新登录后提取。' : error.status === 404 || /not found/i.test(error.message) ? '分享不存在或已过期，请向分享者确认。' : `无法读取分享：${error.message}`;
  } finally {
    if (receiver.key === key && receiver.version === version) { receiver.busy = false; if (isShareRoute()) hooks.render(); }
  }
}
export function renderSharePage() {
  const code = location.hash.match(/^#\/share\/(\d{6})$/)?.[1] || '';
  const user = hooks.user();
  const key = `${code}:${user?.user_id || ''}`;
  if (receiver.key !== key) {
    receiver = { ...receiver, key, code, files: [], error: '', busy: false };
    if (code && user) void load(code, key);
  }
  document.title = 'NetDisk · 提取分享';
  return `<div class="share-page"><header class="share-topbar"><a class="brand" href="#">${icon('cloud')}NetDisk<span class="brand-point">.</span></a>${button(user ? 'account' : 'login', user ? user.username : '登录 / 注册', 'settings')}</header><main class="share-main"><div class="share-intro"><span class="share-emblem">${icon('share')}</span><p class="eyebrow">SHARE SOMETHING GOOD</p><h1>好内容，一起收藏<span class="heading-dot">.</span></h1><p>输入六位分享码，接住一份新的灵感。</p></div><section class="share-receiver"><form id="share-extract-form" class="share-extract-form"><label for="share-extract-code">六位分享码</label><div><input id="share-extract-code" name="code" inputmode="numeric" autocomplete="off" pattern="[0-9]{6}" maxlength="6" required placeholder="000000" value="${code}"><button class="primary-button" type="submit">${icon('download')}提取文件</button></div></form>${!code ? '<div class="share-hint">输入分享码后查看文件。接收者需要登录 NetDisk。</div>' : !user ? `<div class="share-gate">${icon('info')}<h2>登录后查看分享</h2><p>分享码已保留，登录后将自动读取文件。</p>${button('login', '登录并提取', 'arrow', '', 'primary-button')}</div>` : receiver.busy ? '<div class="loading-state" role="status"><span class="spinner"></span>正在读取分享…</div>' : receiver.error ? `<div class="share-gate" role="alert">${icon('info')}<h2>暂时无法提取</h2><p>${esc(receiver.error)}</p>${button('retry', '重新提取', 'clock')}${button('login', '重新登录', 'settings')}</div>` : `<div class="share-files-heading"><h2>分享给你的文件</h2><span class="count">${receiver.files.length} 个项目</span></div>${receiver.files.length ? `<div class="share-download-list">${receiver.files.map(f => `<article><span class="file-icon ${f.is_directory ? 'folder' : 'doc'}">${icon(f.is_directory ? 'folder' : 'doc')}</span><div><strong>${esc(f.file_name)}</strong><small>${f.is_directory ? '文件夹 · ZIP 打包下载' : size(f.file_size)}</small></div>${button('download', downloading.has(`${code}:${f.file_id}`) ? '准备中…' : f.is_directory ? '下载 ZIP' : '下载', 'download', `data-id="${esc(f.file_id)}" ${downloading.has(`${code}:${f.file_id}`) ? 'disabled' : ''}`)}</article>`).join('')}</div>` : '<div class="share-hint">这个分享已没有可下载的文件，请联系分享者。</div>'}<p class="share-hint">${icon('info')}分享可能到期或被移除，下载时会再次检查访问权限。</p>`}</section><div class="share-bottom">${button('home', '返回我的文件', 'folder', '', 'text-button')}<span>有序收藏，自在分享。</span></div></main></div>`;
}

export function initShares(options) {
  hooks = options;
  document.addEventListener('submit', event => {
    if (event.target.id !== 'share-extract-form') return;
    event.preventDefault();
    const code = new FormData(event.target).get('code').trim();
    if (!/^\d{6}$/.test(code)) { hooks.toast('请输入六位数字分享码'); return; }
    if (location.hash === `#/share/${code}`) { if (hooks.user()) { void load(code, receiver.key); hooks.render(); } }
    else location.hash = `/share/${code}`;
  });
  document.addEventListener('click', async event => {
    const el = event.target.closest('[data-share-action]'); if (!el) return;
    const { shareAction: action, code } = el.dataset;
    try {
      if (action === 'copy-link') await copy(link(code));
      else if (action === 'copy-code') await copy(code);
      else if (action === 'revoke') revoke(el.dataset.id);
      else if (action === 'refresh-history') await hooks.refreshShares();
      else if (action === 'visit') location.hash = `/share/${code}`;
      else if (action === 'extract') location.hash = '/share';
      else if (action === 'home') { location.hash = ''; hooks.home(); }
      else if (action === 'login') hooks.login();
      else if (action === 'account') hooks.account();
      else if (action === 'retry') { void load(receiver.code, receiver.key); hooks.render(); }
      else if (action === 'download') {
        const file = receiver.files.find(f => f.file_id === el.dataset.id); if (!file) return;
        const shareCode = receiver.code; const downloadKey = `${shareCode}:${file.file_id}`;
        if (downloading.has(downloadKey)) return;
        downloading.add(downloadKey); hooks.render();
        try {
          hooks.toast(file.is_directory ? '正在打包文件夹，请稍候…' : '正在准备下载…');
          const blob = await api.sharedDownload(shareCode, file.file_id);
          const url = URL.createObjectURL(blob); const a = document.createElement('a');
          a.href = url; a.download = file.is_directory ? `${file.file_name}.zip` : file.file_name;
          document.body.append(a); a.click(); a.remove(); setTimeout(() => URL.revokeObjectURL(url), 60000);
          hooks.toast('已开始下载');
        } finally { downloading.delete(downloadKey); hooks.render(); }
      }
    } catch (error) { hooks.toast(error.message); }
  });
  window.addEventListener('hashchange', () => { document.querySelector('.context-menu')?.remove(); hooks.render(); });
}
