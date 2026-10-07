// All backend access lives here. Routes match src/api.rs; no backend changes required.
export const ROOT_ID = '00000000-0000-0000-0000-000000000000';
let token = sessionStorage.getItem('netdisk-token') || '';

export function setToken(value) {
  token = value;
  if (value) sessionStorage.setItem('netdisk-token', value);
  else sessionStorage.removeItem('netdisk-token');
}

async function request(path, options = {}, responseType = 'auto') {
  const headers = new Headers(options.headers);
  if (token) headers.set('Authorization', `Bearer ${token}`);
  if (options.body && !(options.body instanceof FormData)) headers.set('Content-Type', 'application/json');
  const response = await fetch(`/api${path}`, { ...options, headers });
  if (!response.ok) {
    const message = await response.text();
    const error = new Error(response.status === 401 ? '登录已过期，请重新登录' : message || `请求失败 (${response.status})`);
    error.status = response.status;
    throw error;
  }
  // Download responses contain raw bytes, including when Content-Type is absent.
  if (responseType === 'blob') return response.blob();
  if (response.status === 204) return null;
  const text = await response.text();
  if (!text) return null;
  return response.headers.get('content-type')?.includes('application/json') ? JSON.parse(text) : text;
}

function pending(feature) { throw new Error(`${feature}接口尚未接入`); }

export const api = {
  login: (username, password) => request('/login', { method: 'POST', body: JSON.stringify({ username, password }) }),
  register: (username, password) => request('/register', { method: 'POST', body: JSON.stringify({ username, password }) }),
  adminUsers: () => request('/admin/users'),
  deleteUser: id => request(`/admin/users/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  adminUser: id => request(`/admin/users/${encodeURIComponent(id)}`),
  updateUser: (id, data) => request(`/admin/users/${encodeURIComponent(id)}`, { method: 'PATCH', body: JSON.stringify(data) }),
  user: () => request('/user_info'),
  list: (userId, parentId = ROOT_ID) => request(`/files?user_id=${encodeURIComponent(userId)}&parent_id=${encodeURIComponent(parentId || ROOT_ID)}`),
  metadata: id => request(`/files/${encodeURIComponent(id)}`),
  rename: (id, name) => request(`/files/${encodeURIComponent(id)}`, { method: 'PATCH', body: JSON.stringify({ file_name: name }) }),
  move: (id, newParentId = ROOT_ID) => request('/move', { method: 'POST', body: JSON.stringify({ file_id: id, new_parent_id: newParentId || ROOT_ID }) }),
  delete: id => request(`/files/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  createFolder: (name, owner, parent = ROOT_ID) => request('/files', { method: 'POST', body: JSON.stringify({ file_name: name, file_hash: '', file_size: 0, file_owner: owner, parent_id: parent, is_directory: true }) }),
  upload(file, onProgress, parentId = '', userId = '') {
    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      const query = new URLSearchParams();
      if (parentId) query.set('parent_id', parentId);
      if (userId) query.set('user_id', userId);
      xhr.open('POST', `/api/upload?${query}`);
      if (token) xhr.setRequestHeader('Authorization', `Bearer ${token}`);
      xhr.upload.onprogress = e => { if (e.lengthComputable) onProgress(Math.round(e.loaded / e.total * 100)); };
      xhr.onload = () => {
        if (xhr.status >= 200 && xhr.status < 300) {
          try { resolve(JSON.parse(xhr.responseText)); } catch { reject(new Error('上传接口返回格式有误')); }
        } else reject(new Error(xhr.status === 401 ? '登录已过期，请重新登录' : xhr.responseText || '上传失败'));
      };
      xhr.onerror = () => reject(new Error('无法连接服务器，请检查网络与代理配置'));
      const form = new FormData();
      form.append('file', file);
      xhr.send(form);
    });
  },
  download: id => request(`/download/${encodeURIComponent(id)}`, {}, 'blob'),
  share: (fileIds, expiredAt = null) => request('/create_share', { method: 'POST', body: JSON.stringify({ file_id_list: fileIds, expired_at: expiredAt }) }),
  sharedFiles: code => request(`/shares/${encodeURIComponent(code)}`),
  sharedDownload: (code, id) => request(`/shares/${encodeURIComponent(code)}/download/${encodeURIComponent(id)}`, {}, 'blob'),
  // TODO: implement these after adding the corresponding Rust routes.
  trash: async () => pending('回收站'),
  restore: async () => pending('恢复文件'),
  quota: async () => pending('存储容量'),
};
