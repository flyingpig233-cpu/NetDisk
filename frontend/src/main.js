import './style.css';
import { api, ROOT_ID, setToken } from './api';
import { icon } from './icons';
import { initShares, isShareRoute, renderSharePage, shareHistory, createShare, resetShareReceiver } from './shares';

const app = document.querySelector('#app');
const dialog = document.querySelector('#dialog');
const input = document.querySelector('#file-input');
const escape = value => String(value).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);
function readPreference(key, fallback) { try { return JSON.parse(localStorage.getItem(key)) ?? fallback; } catch { return fallback; } }
let favorites = new Set(readPreference('netdisk-favorites', []));
let state = { users: [], managedUser: null, user: null, files: [], section: 'all', folder: ROOT_ID, query: '', view: readPreference('netdisk-view', 'list'), sort: 'updated', selected: new Set(), transfers: [], busy: false, error: '', menu: null, mobile: false };
let noticeTimer;
let refreshId = 0;
let authVersion = 0;
const sections = { admin: '用户管理', all: '我的文件', recent: '最近使用', starred: '星标文件', shared: '我的分享', trash: '回收站', image: '图片', doc: '文档', video: '视频', music: '音频', archive: '压缩包' };
const kind = file => file.is_directory ? 'folder' : (/\.(png|jpe?g|webp|gif|svg|heic)$/i.test(file.file_name) ? 'image' : /\.(mp4|mov|mkv|webm)$/i.test(file.file_name) ? 'video' : /\.(mp3|wav|flac|ogg)$/i.test(file.file_name) ? 'music' : /\.(zip|rar|7z|tar|gz)$/i.test(file.file_name) ? 'archive' : 'doc');
const bytes = n => n === 0 ? '0 B' : n < 1024 ? `${n} B` : n < 1024 ** 2 ? `${(n / 1024).toFixed(1)} KB` : n < 1024 ** 3 ? `${(n / 1024 ** 2).toFixed(1)} MB` : `${(n / 1024 ** 3).toFixed(2)} GB`;
const date = n => new Intl.DateTimeFormat('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false }).format(new Date(n * 1000));
const workspaceOwner = () => state.managedUser?.user_id || state.user?.user_id;
const fileById = id => state.files.find(f => f.file_id === id);
const actionButton = (action, label, symbol, cls = '', data = '') => `<button class="${cls}" data-action="${action}" ${data}>${symbol ? icon(symbol) : ''}<span>${label}</span></button>`;

function toast(message) {
  const el = document.querySelector('#toast');
  el.textContent = message; el.classList.add('visible');
  clearTimeout(noticeTimer); noticeTimer = setTimeout(() => el.classList.remove('visible'), 4000);
}
function saveFavorites() { try { localStorage.setItem('netdisk-favorites', JSON.stringify([...favorites])); } catch { /* preferences are optional */ } }
function visibleFiles() {
  let files = state.files.filter(f => state.section === 'trash' ? f.trashed : !f.trashed);
  if (state.section === 'all') files = files.filter(f => f.parent_id === state.folder);
  else if (state.section === 'starred') files = files.filter(f => favorites.has(f.file_id));
  else if (['image', 'doc', 'video', 'music', 'archive'].includes(state.section)) files = files.filter(f => kind(f) === state.section);
  else if (state.section === 'recent') files = files.filter(f => !f.is_directory);
  if (state.query) files = files.filter(f => f.file_name.toLocaleLowerCase().includes(state.query.toLocaleLowerCase()));
  return files.sort((a, b) => Number(b.is_directory) - Number(a.is_directory) || (state.sort === 'name' ? a.file_name.localeCompare(b.file_name, 'zh') : state.sort === 'size' ? b.file_size - a.file_size : b.file_updated_at - a.file_updated_at));
}
function navItem(key, symbol) { return `<button class="nav-item ${state.section === key ? 'active' : ''}" data-action="nav" data-section="${key}" ${state.section === key ? 'aria-current="page"' : ''}>${icon(symbol)}<span>${sections[key]}</span>${key === 'all' ? '<span class="nav-dot"></span>' : ''}</button>`; }
function sidebar() {
  const used = state.files.filter(f => !f.trashed).reduce((n, f) => n + f.file_size, 0);
  return `<aside class="sidebar ${state.mobile ? 'open' : ''}"><a class="brand" href="#" data-action="home"><span class="brand-symbol">${icon('cloud')}</span>NetDisk<span class="brand-point">.</span></a>
    ${actionButton('upload', '上传文件', 'plus', 'upload-main')}
    <nav aria-label="文件导航">${navItem('all', 'folder')}${navItem('recent', 'clock')}${navItem('starred', 'star')}<div class="nav-separator"></div>${navItem('shared', 'share')}${navItem('trash', 'trash')}${state.user?.is_admin ? navItem('admin', 'settings') : ''}<p class="nav-label">文件分类</p>${navItem('image', 'image')}${navItem('doc', 'doc')}${navItem('video', 'video')}${navItem('music', 'music')}${navItem('archive', 'archive')}</nav>
    <div class="storage"><div class="storage-title">${icon('cloud')}<span>存储空间</span><span class="storage-mode">已用</span></div><p><strong>${bytes(used)}</strong> · 文件总大小</p><small>容量配额待接入</small></div>
    <div class="sidebar-footer"><span class="status-dot"></span>已连接个人空间${actionButton('account', '账户', 'settings', 'icon-button', 'aria-label="账户"')}</div></aside>`;
}
function previewArt(file) {
  return `<div class="art generic-art ${kind(file)}">${icon(kind(file))}<span>${escape(file.file_name.split('.').pop().toUpperCase())}</span></div>`;
}
function recommendations() {
  const recent = state.files.filter(f => !f.trashed && !f.is_directory).sort((a, b) => b.file_updated_at - a.file_updated_at).slice(0, 3);
  if (!recent.length || state.section !== 'all' || state.folder !== ROOT_ID || state.query) return '';
  return `<section class="recent-section"><div class="section-heading"><h2>最近修改</h2><button class="text-button" data-action="nav" data-section="recent">查看全部 ${icon('arrow')}</button></div><div class="recent-grid">${recent.map(f => `<button class="recent-card" data-action="open" data-id="${escape(f.file_id)}">${previewArt(f)}<div class="recent-caption"><span class="file-icon ${kind(f)}">${icon(kind(f))}</span><div><strong>${escape(f.file_name)}</strong><small><span class="recent-size">${bytes(f.file_size)}</span><span class="recent-separator">·</span><span class="recent-date">${date(f.file_updated_at)}</span></small></div>${icon('arrow', 'card-arrow')}</div></button>`).join('')}</div></section>`;
}
function row(file) {
  const id = escape(file.file_id); const starred = favorites.has(file.file_id);
  return `<tr class="${state.selected.has(file.file_id) ? 'selected' : ''}"><td class="check-cell"><input type="checkbox" data-select="${id}" aria-label="选择 ${escape(file.file_name)}" ${state.selected.has(file.file_id) ? 'checked' : ''}></td><td><div class="file-cell"><button class="file-open" data-action="open" data-id="${id}"><span class="file-icon ${kind(file)}">${icon(kind(file))}</span><span>${escape(file.file_name)}</span></button>${starred ? icon('star', 'inline-star') : ''}</div></td><td class="type-cell">${file.is_directory ? '文件夹' : file.file_name.includes('.') ? escape(file.file_name.split('.').pop().toUpperCase()) : '文件'}</td><td class="size-cell">${file.is_directory ? '—' : bytes(file.file_size)}</td><td class="date-cell">${date(file.file_updated_at)}</td><td class="row-actions"><button class="icon-button star-button ${starred ? 'is-starred' : ''}" data-action="star" data-id="${id}" aria-label="${starred ? '取消星标' : '添加星标'}">${icon('star')}</button><button class="icon-button" data-action="more" data-id="${id}" aria-label="${escape(file.file_name)} 的更多操作">${icon('more')}</button></td></tr>`;
}
function fileGrid(files) { return `<div class="file-grid">${files.map(f => `<article class="file-tile ${state.selected.has(f.file_id) ? 'selected' : ''}"><div class="tile-top"><input type="checkbox" data-select="${escape(f.file_id)}" aria-label="选择 ${escape(f.file_name)}" ${state.selected.has(f.file_id) ? 'checked' : ''}><button class="icon-button" data-action="more" data-id="${escape(f.file_id)}" aria-label="更多操作">${icon('more')}</button></div><button class="tile-open" data-action="open" data-id="${escape(f.file_id)}"><span class="file-icon ${kind(f)}">${icon(kind(f))}</span><strong>${escape(f.file_name)}</strong><small>${f.is_directory ? `${state.files.filter(x => x.parent_id === f.file_id && !x.trashed).length} 个项目` : bytes(f.file_size)}</small></button></article>`).join('')}</div>`; }
function emptyState() {
  const message = state.query ? '没有找到匹配的文件' : state.section === 'trash' ? '回收站是空的' : state.section === 'starred' ? '把常用文件加上星标' : '这里还没有文件';
  return `<div class="empty-state"><span>${icon(state.section === 'trash' ? 'trash' : state.section === 'starred' ? 'star' : 'folder')}</span><h3>${message}</h3><p>${state.query ? '试试其他关键词，或清空搜索。' : state.section === 'trash' ? '回收站接口尚未开放。' : state.section === 'starred' ? '点击文件旁的星标，方便下次快速找到。' : '上传一份文件，开始整理你的空间。'}</p>${state.section === 'all' && !state.query ? actionButton('upload', '上传文件', 'upload', 'primary-button') : ''}</div>`;
}
function adminUsers() {
  const users = state.users.filter(user => `${user.username} ${user.user_id}`.toLocaleLowerCase().includes(state.query.toLocaleLowerCase()));
  return `<section class="files-section"><div class="files-heading"><h2>全部用户</h2><span class="count">${users.length} 个账户</span></div>${state.error ? `<div class="error-state" role="alert">${escape(state.error)}${actionButton('refresh', '重试', null, 'text-button')}</div>` : ''}${state.busy ? '<div class="loading-state"><span class="spinner"></span>正在读取用户…</div>' : `<div class="admin-user-list">${users.map(user => `<article class="admin-user-row"><div class="admin-user-info"><strong>${escape(user.username)} <span class="share-status">${user.is_admin ? '管理员' : '普通用户'}</span></strong><small>${escape(user.user_id)}</small><small>创建：${date(user.created_at)} · 更新：${date(user.updated_at)}</small></div><div class="admin-user-actions">${actionButton('admin-files', '管理文件', 'folder', 'secondary-button', `data-id="${escape(user.user_id)}"`)}${actionButton('admin-edit-user', '修改账户', 'edit', 'secondary-button', `data-id="${escape(user.user_id)}"`)}${user.user_id !== state.user.user_id ? actionButton('admin-delete-user', '删除用户', 'trash', 'secondary-button danger', `data-id="${escape(user.user_id)}"`) : ''}</div></article>`).join('') || '<div class="empty-state"><h3>没有匹配的用户</h3></div>'}</div>`}</section>`;
}
function deleteUser(user) {
  if (!state.user?.is_admin || !user || user.user_id === state.user.user_id) return;
  const session = authVersion;
  modal(`删除用户：${user.username}`, `<p class="dialog-description">将永久删除账户 <strong>${escape(user.username)}</strong>、其全部文件和文件夹，并清理相应分享。其他用户仍在使用的共享文件内容会保留。此操作无法撤销。</p><label class="form-label">输入用户名确认删除<input name="confirmation" required autocomplete="off" placeholder="${escape(user.username)}"></label>`, async form => {
    if (form.get('confirmation') !== user.username) throw new Error('输入的用户名不匹配');
    await api.deleteUser(user.user_id);
    if (authVersion !== session) return;
    ++refreshId;
    state.users = state.users.filter(u => u.user_id !== user.user_id);
    if (state.managedUser?.user_id === user.user_id) {
      state.managedUser = null; state.files = []; state.folder = ROOT_ID; state.selected.clear(); state.transfers = [];
    }
    state.busy = false; render(); toast('用户及其文件已删除');
  }, '永久删除用户');
}
function editUser(user) {
  if (!state.user?.is_admin || !user) return;
  const session = authVersion;
  modal(`修改账户：${user.username}`, `<label class="form-label">用户名<input name="username" value="${escape(user.username)}" maxlength="64" required></label><label class="form-label">重置密码<input name="password" type="password" autocomplete="new-password" minlength="8" placeholder="留空保留原密码"></label><label class="form-label">权限<select name="role"><option value="user" ${!user.is_admin ? 'selected' : ''}>普通用户</option><option value="admin" ${user.is_admin ? 'selected' : ''}>管理员</option></select></label><p class="dialog-description">重置密码后，该账户已有登录会失效。系统至少保留一位管理员。</p>`, async form => {
    const password = form.get('password');
    const updated = await api.updateUser(user.user_id, { username: form.get('username').trim(), is_admin: form.get('role') === 'admin', ...(password ? { password } : {}) });
    if (authVersion !== session) return;
    if (user.user_id === state.user.user_id) {
      if (password) { logout(); toast('密码已更新，请重新登录'); return; }
      state.user = updated;
      if (!updated.is_admin) { state.managedUser = null; state.users = []; state.files = []; state.section = 'all'; state.folder = ROOT_ID; await refresh(); }
    }
    state.users = state.users.map(u => u.user_id === updated.user_id ? updated : u);
    if (state.managedUser?.user_id === updated.user_id) state.managedUser = updated;
    render(); toast('账户信息已更新');
  }, '保存');
}
function fileArea() {
  if (state.section === 'admin') return adminUsers();
  if (state.section === 'shared') return shareHistory(state.query);
  const files = visibleFiles();
  const folder = fileById(state.folder);
  const title = state.query ? '搜索结果' : state.section === 'all' ? folder?.file_name || '全部文件' : sections[state.section];
  return `<section class="files-section"><div class="files-heading"><div class="heading-left"><h2>${escape(title)}</h2><span class="count">${files.length} 个项目</span></div><div class="file-tools"><label class="sort-control">${icon('sort')}<select id="sort" aria-label="文件排序"><option value="updated" ${state.sort === 'updated' ? 'selected' : ''}>最近修改</option><option value="name" ${state.sort === 'name' ? 'selected' : ''}>名称排序</option><option value="size" ${state.sort === 'size' ? 'selected' : ''}>文件大小</option></select></label><div class="view-toggle">${actionButton('list-view', '', 'list', `icon-button ${state.view === 'list' ? 'active' : ''}`, 'aria-label="列表视图"')}${actionButton('grid-view', '', 'grid', `icon-button ${state.view === 'grid' ? 'active' : ''}`, 'aria-label="网格视图"')}</div></div></div>
    ${state.selected.size ? `<div class="selection-bar"><span>已选择 ${state.selected.size} 项</span>${state.section !== 'trash' && state.section !== 'shared' ? actionButton('share-selected', '分享', 'share', 'text-button') + actionButton('move-selected', '移动', 'move', 'text-button') : ''}${actionButton('delete-selected', state.section === 'trash' ? '彻底删除' : '删除', 'trash', 'text-button')}${state.section === 'trash' ? actionButton('restore-selected', '恢复', 'clock', 'text-button') : ''}${actionButton('clear-selection', '取消选择', 'close', 'text-button')}</div>` : ''}
    ${state.error ? `<div class="error-state">${icon('info')}<span>${escape(state.error)}</span>${actionButton('refresh', '重试', null, 'text-button')}</div>` : ''}
    ${state.busy ? '<div class="loading-state"><span class="spinner"></span>正在读取文件…</div>' : files.length ? state.view === 'grid' ? fileGrid(files) : `<div class="table-wrap"><table><thead><tr><th class="check-cell"><input id="select-all" type="checkbox" aria-label="选择当前全部文件" ${files.every(f => state.selected.has(f.file_id)) ? 'checked' : ''}></th><th>文件名称</th><th class="type-cell">类型</th><th class="size-cell">大小</th><th class="date-cell">修改时间</th><th class="row-actions"></th></tr></thead><tbody>${files.map(row).join('')}</tbody></table></div>` : emptyState()}
    <div class="files-footer"><span>个人文件空间</span><span>有序收藏，自在随行。</span></div></section>`;
}
function transfers() {
  if (!state.transfers.length) return '';
  return `<aside class="transfer-panel" aria-label="上传任务"><div class="transfer-heading"><strong>${icon('upload')}上传任务</strong><button class="icon-button" data-action="clear-transfers" aria-label="收起已完成的上传">${icon('close')}</button></div>${state.transfers.map(t => `<div class="transfer-item"><span class="file-icon doc">${icon('doc')}</span><div><strong>${escape(t.name)}</strong><small>${t.error ? escape(t.error) : t.done ? '上传完成' : `正在上传 ${t.progress}%`}</small><div class="transfer-track"><i style="width:${t.progress}%"></i></div></div>${t.done ? icon('check', 'success-icon') : t.error ? icon('info') : ''}</div>`).join('')}</aside>`;
}
function render() {
  if (isShareRoute()) { app.innerHTML = renderSharePage(); return; }
  if (!state.user) {
    document.title = 'NetDisk · 登录';
    app.innerHTML = `<div class="share-page"><header class="share-topbar"><a class="brand" href="#">${icon('cloud')}NetDisk<span class="brand-point">.</span></a>${actionButton('extract-share', '提取分享', 'download', 'secondary-button')}</header><main class="share-main"><div class="share-intro"><span class="share-emblem">${icon('cloud')}</span><p class="eyebrow">YOUR PERSONAL SPACE</p><h1>你的文件，井然有序<span class="heading-dot">.</span></h1><p>登录你的账户，开始管理文件。</p></div><section class="share-receiver"><div class="share-gate">${state.busy ? '<div class="loading-state" role="status"><span class="spinner"></span>正在恢复登录…</div>' : `${icon('folder')}<h2>欢迎使用 NetDisk</h2><p>登录或注册后，即可上传、下载和分享文件。</p>${actionButton('account', '登录', 'arrow', 'primary-button')}${actionButton('register', '注册账户', 'plus', 'secondary-button')}`}</div></section></main></div>`;
    return;
  }
  const name = state.user.username;
  document.title = `NetDisk · ${sections[state.section]}`;
  app.innerHTML = `${sidebar()}${state.mobile ? '<div class="sidebar-scrim" data-action="mobile-close"></div>' : ''}<div class="workspace"><header class="topbar"><button class="icon-button mobile-menu" data-action="mobile" aria-label="展开导航">${icon('menu')}</button><div class="breadcrumb"><span>${state.managedUser ? escape(state.managedUser.username) + '的文件' : '个人空间'}</span>${icon('arrow')}<strong>${sections[state.section]}</strong></div><div class="top-actions"><button class="mode-pill" data-action="account"><span class="status-dot"></span>${state.user.is_admin ? '管理员' : '已连接'}</button><button class="avatar" data-action="account" aria-label="账户：${escape(name)}">${escape(name.slice(0, 1).toUpperCase())}</button></div></header>
    <main>${state.managedUser ? `<div class="admin-workspace-banner"><span>正在管理 <strong>${escape(state.managedUser.username)}</strong> 的文件</span>${actionButton('personal-space', '返回我的文件', 'folder', 'text-button')}</div>` : ''}<div class="page-heading"><div><p class="eyebrow">YOUR PERSONAL SPACE</p><h1>${sections[state.section]}<span class="heading-dot">.</span></h1><p class="page-description">${state.section === 'admin' ? '管理用户账户、权限与文件。' : state.section === 'all' ? '每一份文件，都有它的位置。' : state.section === 'recent' ? '接着上次的灵感，继续出发。' : state.section === 'starred' ? '重要的文件，一眼就能找到。' : state.section === 'trash' ? '留一点时间，重新做个决定。' : state.section === 'shared' ? '让好内容，遇见更多人。' : '将同一类收藏，放在一起。'}</p></div><div class="heading-actions">${state.section === 'admin' ? '' : state.section === 'shared' ? actionButton('extract-share', '提取分享', 'download', 'primary-button') : actionButton('create-folder', '新建文件夹', 'plus', 'secondary-button') + actionButton('upload', '上传文件', 'upload', 'primary-button')}</div></div>
    <div class="search-row"><label class="search-box">${icon('search')}<input id="search" type="search" placeholder="${state.section === 'admin' ? '搜索用户名或用户 ID' : state.section === 'shared' ? '搜索分享码或文件名' : '搜索你的文件'}" aria-label="搜索" value="${escape(state.query)}"><kbd>/</kbd></label>${actionButton('refresh', '刷新', 'clock', 'refresh-button')}</div>
    ${state.folder !== ROOT_ID && state.section === 'all' ? `<div class="folder-breadcrumb"><button class="text-button" data-action="home">全部文件</button>${folderTrail()}</div>` : ''}
    ${recommendations()}${fileArea()}</main><footer class="workspace-footer"><span>NetDisk</span><small>属于你的，始终在这里。</small><span>简而有序</span></footer></div>${transfers()}`;
}
function folderTrail() {
  const parents = []; let file = fileById(state.folder); const seen = new Set();
  while (file && !seen.has(file.file_id)) { seen.add(file.file_id); parents.unshift(file); file = fileById(file.parent_id); }
  return parents.map(f => `${icon('arrow')}<button class="text-button" data-action="open" data-id="${escape(f.file_id)}">${escape(f.file_name)}</button>`).join('');
}
function modal(title, content, onSubmit, submitLabel = '确认') {
  if (dialog.open) dialog.close();
  dialog.innerHTML = `<form id="modal-form"><div class="dialog-heading"><h2>${escape(title)}</h2><button type="button" class="icon-button" id="dialog-close" aria-label="关闭">${icon('close')}</button></div>${content}<p id="form-error" class="form-error" role="alert"></p><div class="dialog-actions"><button type="button" class="secondary-button" id="dialog-cancel">取消</button><button class="primary-button" type="submit">${escape(submitLabel)}</button></div></form>`;
  dialog.querySelector('#dialog-close').onclick = () => dialog.close();
  dialog.querySelector('#dialog-cancel').onclick = () => dialog.close();
  dialog.querySelector('form').onsubmit = async event => {
    event.preventDefault(); const button = event.submitter || event.target.querySelector('button[type="submit"]'); button.disabled = true;
    try { const close = await onSubmit(new FormData(event.target)); if (close !== false && dialog.open) dialog.close(); }
    catch (error) { dialog.querySelector('#form-error').textContent = error.message; }
    finally { button.disabled = false; }
  };
  dialog.showModal();
}
function validName(value) {
  const name = String(value || '').trim();
  if (!name || name === '.' || name === '..' || /[\\/\u0000-\u001f]/.test(name)) throw new Error('请输入有效名称，不能包含斜杠或控制字符');
  if (name.length > 255) throw new Error('名称不能超过 255 个字符');
  return name;
}
async function refresh() {
  if (state.section === 'shared') { ++refreshId; state.busy = false; state.error = ''; render(); return; }
  if (!state.user) { render(); return; }
  const requestId = ++refreshId;
  const userId = workspaceOwner(); const folder = state.folder; const section = state.section;
  const isCurrent = () => requestId === refreshId && workspaceOwner() === userId && state.folder === folder && state.section === section;
  state.busy = true; state.error = ''; render();
  try {
    if (section === 'admin') {
      const users = await api.adminUsers();
      if (!isCurrent()) return;
      state.users = users;
    } else if (section === 'all') {
      const files = await api.list(userId, folder);
      if (!isCurrent()) return;
      const removed = descendants(state.files.filter(f => f.parent_id === folder && !files.some(next => next.file_id === f.file_id)).map(f => f.file_id));
      state.files = [...state.files.filter(f => f.parent_id !== folder && !removed.has(f.file_id)), ...files];
    } else {
      const files = []; const pending = [ROOT_ID]; const seen = new Set();
      for (const parent of pending) {
        if (seen.has(parent)) continue;
        seen.add(parent);
        const children = await api.list(userId, parent);
        if (!isCurrent()) return;
        files.push(...children);
        pending.push(...children.filter(f => f.is_directory).map(f => f.file_id));
      }
      state.files = files;
    }
  }
  catch (error) { if (isCurrent()) state.error = error.message; }
  finally { if (isCurrent()) { state.busy = false; state.selected.clear(); render(); } }
}
function logout() {
  setToken('');
  ++refreshId;
  ++authVersion;
  resetShareReceiver();
  state = { ...state, users: [], managedUser: null, user: null, files: [], folder: ROOT_ID, section: 'all', selected: new Set(), query: '', error: '', transfers: [], busy: false, mobile: false };
  document.querySelector('.context-menu')?.remove();
  render();
}
function account() {
  if (state.user) {
    modal('我的账户', `<div class="account-summary"><span class="avatar">${escape(state.user.username.slice(0, 1))}</span><div><strong>${escape(state.user.username)}</strong><p>已连接个人文件空间</p></div></div><p class="dialog-description">退出后需要重新登录才能管理文件。</p>`, async () => { logout(); toast('已退出登录'); }, '退出登录');
    return;
  }
  authDialog(false);
}
function authDialog(register) {
  modal(register ? '创建你的 NetDisk 账户' : '连接你的个人空间', `<p class="dialog-description">${register ? '注册后，使用新账户登录。' : '登录后即可管理后端中的真实文件。'}</p><label class="form-label">用户名<input name="username" autocomplete="username" required maxlength="64" placeholder="输入用户名"></label><label class="form-label">密码<input name="password" type="password" autocomplete="${register ? 'new-password' : 'current-password'}" required placeholder="输入密码"></label><button type="button" class="text-button auth-switch">${register ? '已有账户？返回登录' : '还没有账户？创建账户'}</button>`, async form => {
    const username = form.get('username').trim(); const password = form.get('password');
    if (!username) throw new Error('请输入用户名');
    if (register) { await api.register(username, password); authDialog(false); toast('注册成功，请登录'); return false; }
    const token = await api.login(username, password); setToken(token);
    let user;
    try { user = await api.user(); } catch (error) { setToken(''); throw error; }
    ++authVersion;
    state = { ...state, users: [], managedUser: null, user, files: [], section: 'all', folder: ROOT_ID, query: '', selected: new Set(), transfers: [], error: '', busy: false };
    resetShareReceiver();
    await refresh(); toast('已连接个人空间');
  }, register ? '注册账户' : '登录');
  dialog.querySelector('.auth-switch').onclick = () => authDialog(!register);
}
async function openFile(file) {
  if (!file) return;
  if (file.trashed) { toast('请先恢复文件，再打开'); return; }
  if (file.is_directory) { state.folder = file.file_id; state.section = 'all'; state.query = ''; state.selected.clear(); await refresh(); return; }
  const preview = `<div class="preview-placeholder"><span class="file-icon ${kind(file)}">${icon(kind(file))}</span><p>文件内容预览接口尚未接入</p></div>`;
  modal(file.file_name, `${preview}<dl class="file-details"><div><dt>文件大小</dt><dd>${bytes(file.file_size)}</dd></div><div><dt>修改时间</dt><dd>${date(file.file_updated_at)}</dd></div><div><dt>所在空间</dt><dd>个人空间</dd></div></dl>`, async () => download(file), '下载文件');
}

const activeDownloads = new Set();
async function download(file) {
  if (!state.user) { authDialog(false); return; }
  if (activeDownloads.has(file.file_id)) throw new Error('此文件正在下载，请稍候');
  activeDownloads.add(file.file_id);
  try {
    toast(file.is_directory ? '正在打包文件夹，请稍候…' : '正在准备下载…');
    const blob = await api.download(file.file_id);
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = file.is_directory ? `${file.file_name}.zip` : file.file_name;
    document.body.append(a);
    try { a.click(); }
    finally { a.remove(); setTimeout(() => URL.revokeObjectURL(url), 60000); }
    toast('已开始下载');
  } finally { activeDownloads.delete(file.file_id); }
}
function createFolder() {
  if (!state.user) { authDialog(false); return; }
  if (state.section === 'trash' || state.section === 'shared' || state.section === 'admin') { toast('请在我的文件中新建文件夹'); return; }
  modal('新建文件夹', '<label class="form-label">文件夹名称<input name="name" required maxlength="255" value="新建文件夹" autofocus></label>', async data => {
    const name = validName(data.get('name')); const parent = state.section === 'all' ? state.folder : ROOT_ID;
    if (state.files.some(f => !f.trashed && f.parent_id === parent && f.file_name === name)) throw new Error('此位置已存在同名文件');
    state.files.push(await api.createFolder(name, workspaceOwner(), parent));
    state.section = 'all'; state.folder = parent; state.query = ''; render(); toast('文件夹已创建');
  }, '创建');
  dialog.querySelector('input').select();
}
function rename(file) {
  modal('重命名', `<label class="form-label">新名称<input name="name" required maxlength="255" value="${escape(file.file_name)}" autofocus></label>`, async data => {
    const name = validName(data.get('name'));
    if (state.files.some(f => f.file_id !== file.file_id && !f.trashed && f.parent_id === file.parent_id && f.file_name === name)) throw new Error('此位置已存在同名文件');
    await api.rename(file.file_id, name);
    file.file_name = name; file.file_updated_at = Math.floor(Date.now() / 1000); render(); toast('名称已更新');
  }, '保存');
}
function descendants(ids) {
  const result = new Set(ids); let changed = true;
  while (changed) { changed = false; for (const file of state.files) if (result.has(file.parent_id) && !result.has(file.file_id)) { result.add(file.file_id); changed = true; } }
  return result;
}
function removeFiles(ids) {
  modal('确认彻底删除？', `<p class="dialog-description">将永久删除 ${ids.length} 个项目及其包含的文件。此操作无法撤销。</p>`, async () => {
    // Remove each successful item immediately so partial failures never resurrect deleted rows.
    for (const id of ids) {
      if (!fileById(id)) continue;
      await api.delete(id);
      const removed = descendants([id]);
      state.files = state.files.filter(f => !removed.has(f.file_id));
      for (const removedId of removed) state.selected.delete(removedId);
      render();
    }
    state.selected.clear(); render(); toast('文件已删除');
  }, '彻底删除');
}
function restore() { toast('恢复文件接口尚未接入'); }
async function allFolders() {
  const folders = []; const pending = [ROOT_ID]; const seen = new Set();
  while (pending.length) {
    const parent = pending.shift();
    if (seen.has(parent)) continue;
    seen.add(parent);
    const children = await api.list(workspaceOwner(), parent);
    for (const child of children) if (child.is_directory) { folders.push(child); pending.push(child.file_id); }
  }
  return folders;
}
async function moveFiles(ids) {
  const moving = ids.map(fileById).filter(f => f && !f.trashed);
  if (!moving.length) return;
  let folders;
  try { folders = await allFolders(); }
  catch (error) { toast(error.message); return; }
  // Never allow moving a folder into itself or one of its own descendants.
  const blocked = new Set(ids); let changed = true;
  while (changed) { changed = false; for (const f of folders) if (blocked.has(f.parent_id) && !blocked.has(f.file_id)) { blocked.add(f.file_id); changed = true; } }
  const byId = new Map(folders.map(f => [f.file_id, f]));
  const depthOf = folder => { let depth = 0; let current = folder; while (current && current.parent_id !== ROOT_ID && byId.has(current.parent_id)) { current = byId.get(current.parent_id); depth++; } return depth; };
  const choices = folders.filter(f => !blocked.has(f.file_id));
  const currentParent = moving.length === 1 ? moving[0].parent_id : null;
  const options = `<option value="${ROOT_ID}" ${currentParent === ROOT_ID ? 'selected' : ''}>全部文件（根目录）</option>`
    + choices.map(f => `<option value="${escape(f.file_id)}" ${currentParent === f.file_id ? 'selected' : ''}>${'\u00a0\u00a0\u00a0'.repeat(depthOf(f))}${escape(f.file_name)}</option>`).join('');
  const title = moving.length === 1 ? `移动“${moving[0].file_name}”` : `移动 ${moving.length} 个项目`;
  modal(title, `<p class="dialog-description">选择目标文件夹，文件将被移动到所选位置。</p><label class="form-label">目标文件夹<select name="folder" class="folder-select" size="${Math.min(11, choices.length + 1)}">${options}</select></label>`, async form => {
    const targetId = form.get('folder') || ROOT_ID;
    if (moving.every(f => f.parent_id === targetId)) throw new Error('文件已在该位置');
    for (const f of moving) await api.move(f.file_id, targetId);
    const now = Math.floor(Date.now() / 1000);
    for (const f of moving) { f.parent_id = targetId; f.file_updated_at = now; }
    state.selected.clear(); render(); toast('文件已移动');
  }, '移动');
}
function menu(file, button) {
  document.querySelector('.context-menu')?.remove();
  const menu = document.createElement('div'); menu.className = 'context-menu'; menu.setAttribute('role', 'group'); menu.setAttribute('aria-label', '文件操作');
  const data = `data-id="${escape(file.file_id)}"`;
  menu.innerHTML = file.trashed ? `${actionButton('restore', '恢复文件', 'clock', '', data)}${actionButton('delete', '彻底删除', 'trash', 'danger', data)}` : `${actionButton('open', '打开', 'folder', '', data)}${actionButton('download', file.is_directory ? '下载 ZIP' : '下载', 'download', '', data)}${actionButton('rename', '重命名', 'edit', '', data)}${actionButton('move', '移动到', 'move', '', data)}${actionButton('star', favorites.has(file.file_id) ? '取消星标' : '添加星标', 'star', '', data)}${actionButton('share', '分享', 'share', '', data)}<hr>${actionButton('delete', '删除文件', 'trash', 'danger', data)}`;
  document.body.append(menu);
  const rect = button.getBoundingClientRect(); const height = menu.offsetHeight;
  menu.style.left = `${Math.max(8, Math.min(rect.right - 180, innerWidth - 190))}px`;
  menu.style.top = `${rect.bottom + height + 8 > innerHeight ? Math.max(8, rect.top - height - 4) : rect.bottom + 4}px`;
  menu.querySelector('button')?.focus();
}
async function upload(files) {
  if (!files.length) return;
  if (!state.user) { authDialog(false); return; }
  if (state.section === 'trash' || state.section === 'shared' || state.section === 'admin') { state.section = 'all'; state.folder = ROOT_ID; }
  const owner = workspaceOwner(); const actorId = state.user.user_id; const parent = state.folder; const session = authVersion;
  for (const file of files) {
    if (state.user?.user_id !== actorId || workspaceOwner() !== owner || authVersion !== session) break;
    const transfer = { name: file.name, progress: 0, done: false }; state.transfers.push(transfer); render();
    try {
      const meta = await api.upload(file, progress => { transfer.progress = progress; render(); }, parent, owner);
      if (state.user?.user_id !== actorId || workspaceOwner() !== owner || authVersion !== session) break;
      state.files.push(meta); transfer.progress = 100; transfer.done = true;
    } catch (error) { transfer.error = error.message; }
    render();
  }
}
document.addEventListener('click', async event => {
  const button = event.target.closest('[data-action]');
  if (!event.target.closest('.context-menu') && button?.dataset.action !== 'more') document.querySelector('.context-menu')?.remove();
  if (!button) return;
  const action = button.dataset.action; const file = fileById(button.dataset.id);
  if (action !== 'more') document.querySelector('.context-menu')?.remove();
  try {
    if (action === 'admin-delete-user') { deleteUser(state.users.find(u => u.user_id === button.dataset.id)); }
    else if (action === 'admin-edit-user') { editUser(state.users.find(u => u.user_id === button.dataset.id)); }
    else if (action === 'admin-files') { const user = state.users.find(u => u.user_id === button.dataset.id); if (!state.user?.is_admin || !user) return; state.managedUser = user; state.files = []; state.section = 'all'; state.folder = ROOT_ID; state.query = ''; state.selected.clear(); await refresh(); }
    else if (action === 'personal-space') { state.managedUser = null; state.files = []; state.section = 'all'; state.folder = ROOT_ID; state.query = ''; state.selected.clear(); await refresh(); }
    else if (action === 'nav') { state.section = button.dataset.section; state.folder = ROOT_ID; state.query = ''; state.selected.clear(); state.mobile = false; await refresh(); }
    else if (action === 'home') { event.preventDefault(); state.section = 'all'; state.folder = ROOT_ID; state.query = ''; state.selected.clear(); await refresh(); }
    else if (action === 'upload') { if (state.user) input.click(); else authDialog(false); }
    else if (action === 'account') account();
    else if (action === 'register') authDialog(true);
    else if (action === 'extract-share') location.hash = '/share';
    else if (action === 'refresh') await refresh();
    else if (action === 'open') await openFile(file);
    else if (action === 'create-folder') createFolder();
    else if (action === 'rename' && file) rename(file);
    else if (action === 'move' && file) await moveFiles([file.file_id]);
    else if (action === 'move-selected') await moveFiles([...state.selected]);
    else if (action === 'delete' && file) removeFiles([file.file_id]);
    else if (action === 'delete-selected') removeFiles([...state.selected]);
    else if (action === 'restore' && file) restore([file.file_id]);
    else if (action === 'restore-selected') restore([...state.selected]);
    else if (action === 'star' && file) { favorites.has(file.file_id) ? favorites.delete(file.file_id) : favorites.add(file.file_id); saveFavorites(); render(); toast(favorites.has(file.file_id) ? '已添加星标（保存在此浏览器）' : '已取消星标'); }
    else if (action === 'clear-selection') { state.selected.clear(); render(); }
    else if (action === 'more' && file) menu(file, button);
    else if (action === 'download' && file) await download(file);
    else if (action === 'share' && file) createShare([file]);
    else if (action === 'share-selected') createShare([...state.selected].map(fileById).filter(Boolean));
    else if (action === 'list-view' || action === 'grid-view') { state.view = action === 'list-view' ? 'list' : 'grid'; try { localStorage.setItem('netdisk-view', JSON.stringify(state.view)); } catch { /* optional */ } render(); }
    else if (action === 'mobile' || action === 'mobile-close') { state.mobile = action === 'mobile'; render(); }
    else if (action === 'clear-transfers') { state.transfers = state.transfers.filter(t => !t.done && !t.error); render(); }
  } catch (error) { toast(error.message); }
});
app.addEventListener('input', event => {
  if (event.target.id !== 'search') return;
  const position = event.target.selectionStart; state.query = event.target.value; state.selected.clear(); render();
  const search = document.querySelector('#search'); search.focus(); try { search.setSelectionRange(position, position); } catch { /* search selection is browser dependent */ }
});
app.addEventListener('change', event => {
  if (event.target.id === 'sort') { state.sort = event.target.value; render(); }
  else if (event.target.dataset.select) { const id = event.target.dataset.select; event.target.checked ? state.selected.add(id) : state.selected.delete(id); render(); }
  else if (event.target.id === 'select-all') { const checked = event.target.checked; for (const f of visibleFiles()) checked ? state.selected.add(f.file_id) : state.selected.delete(f.file_id); render(); }
});
input.onchange = () => { const files = [...input.files]; input.value = ''; upload(files); };
let dragDepth = 0;
document.addEventListener('dragenter', event => { if ([...event.dataTransfer.types].includes('Files')) { event.preventDefault(); dragDepth++; document.body.classList.add('dragging'); } });
document.addEventListener('dragover', event => { if ([...event.dataTransfer.types].includes('Files')) event.preventDefault(); });
document.addEventListener('dragleave', () => { if (--dragDepth <= 0) { dragDepth = 0; document.body.classList.remove('dragging'); } });
document.addEventListener('drop', event => { if (![...event.dataTransfer.types].includes('Files')) return; event.preventDefault(); dragDepth = 0; document.body.classList.remove('dragging'); if (!dialog.open && !isShareRoute()) upload([...event.dataTransfer.files]); });
document.addEventListener('keydown', event => {
  if (event.key === 'Escape') { document.querySelector('.context-menu')?.remove(); if (state.mobile) { state.mobile = false; render(); } }
  if (event.key === '/' && !dialog.open && !['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement.tagName)) { event.preventDefault(); document.querySelector('#search')?.focus(); }
});
window.addEventListener('resize', () => document.querySelector('.context-menu')?.remove());
window.addEventListener('scroll', () => document.querySelector('.context-menu')?.remove(), true);
initShares({
  user: () => state.user, modal, toast, render,
  login: () => authDialog(false), account,
  home: () => { state.section = 'all'; state.folder = ROOT_ID; state.query = ''; state.selected.clear(); void refresh(); },
});
state.busy = Boolean(sessionStorage.getItem('netdisk-token'));
render();
if (state.busy) {
  api.user().then(async user => { state.user = user; state.files = []; state.busy = false; await refresh(); }).catch(error => { logout(); toast(error.message); });
}
