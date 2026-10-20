const $ = (id) => document.getElementById(id);
const MAX_FILE = 5 * 1024 * 1024;
const IMAGE_TYPES = new Set(['image/png', 'image/jpeg', 'image/gif', 'image/webp']);
const state = {
  session: 'loading', me: null, tab: 'gallery', query: '', cursor: null,
  galleryController: null, hashes: new Set(), mineLoaded: false,
  mineBusy: false, mineEpoch: 0, file: null, previewUrl: null,
  uploadBusy: false, uploadSuccess: false,
};

class ApiError extends Error {
  constructor(message, status = 0) { super(message); this.status = status; }
}

function element(tag, className = '', text = '') {
  const result = document.createElement(tag);
  result.className = className;
  result.textContent = text;
  return result;
}

function errorText(error) {
  if (error instanceof ApiError) return error.message;
  return '网络连接失败或请求超时，请稍后重试。';
}

async function responseError(response) {
  let body;
  try { body = await response.json(); } catch { body = null; }
  const defaults = {
    401: '登录已过期，请重新登录。', 403: '操作未获授权，请刷新页面后重试。',
    413: '图片或请求过大，请选择更小的图片。', 429: '操作太频繁，请稍后再试。',
  };
  const fallback = defaults[response.status] || '服务暂时无法处理请求，请稍后重试。';
  const message = typeof body?.error === 'string' ? body.error : body?.error?.message || body?.message;
  return new ApiError(typeof message === 'string' ? message : fallback, response.status);
}

async function api(path, options = {}) {
  const timeout = AbortSignal.timeout(35000);
  const signal = options.signal ? AbortSignal.any([options.signal, timeout]) : timeout;
  const response = await fetch(path, {
    credentials: 'same-origin', cache: 'no-store', redirect: 'error', ...options, signal,
  });
  if (!response.ok) throw await responseError(response);
  try { return await response.json(); }
  catch { throw new ApiError('服务返回的数据无法识别，请稍后重试。'); }
}

function actionControl(action) {
  if (action.href) {
    const link = element('a', 'button primary', action.label);
    link.href = action.href;
    return link;
  }
  const button = element('button', 'button secondary', action.label);
  button.type = 'button';
  button.addEventListener('click', action.onClick);
  return button;
}

function stateBox(id, content) {
  const box = $(id);
  box.hidden = false;
  box.classList.toggle('is-error', Boolean(content.error));
  const symbol = element('span', 'status-symbol', content.symbol || '☺');
  symbol.setAttribute('aria-hidden', 'true');
  box.replaceChildren(symbol, element('h3', '', content.title), element('p', '', content.text));
  if (content.action) box.append(actionControl(content.action));
}

function notice(message, isError = false) {
  const box = $('global-notice');
  box.textContent = message;
  box.classList.toggle('error', isError);
  box.hidden = !message;
}

function showTab(tab, focus = false) {
  state.tab = tab;
  for (const name of ['gallery', 'mine']) {
    const selected = name === tab;
    $(`${name}-tab`).setAttribute('aria-selected', String(selected));
    $(`${name}-tab`).tabIndex = selected ? 0 : -1;
    $(`${name}-panel`).hidden = !selected;
  }
  $('search-form').hidden = tab !== 'gallery';
  $('refresh-mine').hidden = tab !== 'mine';
  if (focus) $(`${tab}-tab`).focus();
  if (tab === 'mine') loadMine();
}

function tabsKeydown(event) {
  const choices = { ArrowLeft: 'gallery', ArrowRight: 'mine', Home: 'gallery', End: 'mine' };
  const next = choices[event.key];
  if (!next) return;
  event.preventDefault();
  showTab(next, true);
}

function beginGallery(append) {
  state.galleryController?.abort();
  const controller = new AbortController();
  state.galleryController = controller;
  $('gallery-grid').setAttribute('aria-busy', 'true');
  $('load-more').disabled = true;
  $('load-more').textContent = '正在加载…';
  if (append) return controller;
  state.hashes.clear();
  state.cursor = null;
  $('gallery-grid').replaceChildren();
  $('gallery-count').textContent = '';
  $('load-more').hidden = true;
  $('gallery-heading').textContent = state.query ? '搜索结果' : '所有表情';
  stateBox('gallery-status', { symbol: '···', title: '正在寻找表情', text: '稍等一下，图库马上就来。' });
  return controller;
}

async function loadGallery(append = false) {
  const controller = beginGallery(append);
  const previousCursor = state.cursor;
  const params = new URLSearchParams({ query: state.query, limit: '24' });
  if (append) params.set('cursor', previousCursor);
  try {
    const data = await api(`/api/gallery?${params}`, { signal: controller.signal });
    if (!Array.isArray(data.items)) throw new ApiError('图库数据无法识别，请稍后重试。');
    renderGallery(data, previousCursor);
  } catch (error) {
    if (controller.signal.aborted) return;
    galleryFailure(error, append);
  } finally {
    if (state.galleryController === controller) {
      $('gallery-grid').setAttribute('aria-busy', 'false');
      $('load-more').disabled = false;
      $('load-more').textContent = '再看一些 ↓';
    }
  }
}

function validGalleryItem(item) {
  if (!item) return false;
  const fieldsValid = typeof item.name === 'string' && typeof item.display_name === 'string';
  return fieldsValid && /^[a-f0-9]{64}$/.test(item.manifest_hash) && Number.isInteger(item.uploader_id);
}

function renderGallery(data, previousCursor) {
  if (!data.items.every(validGalleryItem)) throw new ApiError('图库中有无法识别的记录，请稍后重试。');
  $('gallery-status').hidden = true;
  for (const item of data.items) {
    if (state.hashes.has(item.manifest_hash)) continue;
    state.hashes.add(item.manifest_hash);
    $('gallery-grid').append(galleryCard(item));
  }
  const next = typeof data.next_cursor === 'string' ? data.next_cursor : null;
  state.cursor = next !== previousCursor ? next : null;
  $('load-more').hidden = !state.cursor;
  $('gallery-count').textContent = `已显示 ${state.hashes.size} 张`;
  if (!state.hashes.size) emptyGallery();
}

function emptyGallery() {
  const searching = Boolean(state.query);
  const action = searching
    ? { label: '查看所有表情', onClick: clearSearch }
    : { label: '分享第一张表情', onClick: openUpload };
  stateBox('gallery-status', {
    symbol: searching ? '⌕' : '☺', title: searching ? '没有找到匹配的表情' : '这里还没有表情',
    text: searching ? '换个名称试试，或回到图库随意看看。' : '有趣的图库，从大家分享的第一张开始。', action,
  });
}

function galleryFailure(error, append) {
  stateBox('gallery-status', {
    symbol: '!', title: append ? '后面的表情没能加载' : '图库暂时没能打开',
    text: errorText(error), error: true,
    action: { label: '重新加载', onClick: () => loadGallery(append) },
  });
  $('load-more').hidden = true;
}

function galleryCard(item) {
  const card = element('button', 'meme-card');
  card.type = 'button';
  card.setAttribute('aria-label', `查看表情：${item.display_name}，上传者 #${item.uploader_id}`);
  const frame = element('span', 'card-image');
  const image = element('img');
  image.alt = '';
  image.loading = 'lazy';
  image.decoding = 'async';
  image.src = `/api/images/${item.manifest_hash}`;
  image.addEventListener('error', () => frame.replaceChildren(element('span', 'card-image-error', '图片暂时无法加载')));
  frame.append(image);
  const meta = element('span', 'card-meta');
  const uploader = element('span', 'card-uploader');
  const arrow = element('span', 'card-arrow', '↗');
  arrow.setAttribute('aria-hidden', 'true');
  uploader.append(element('span', '', `上传者 #${item.uploader_id}`), arrow);
  meta.append(element('span', 'card-title', item.display_name), element('span', 'card-name mono', item.name), uploader);
  card.append(frame, meta);
  card.addEventListener('click', () => openImage(item));
  return card;
}

function openImage(item) {
  $('image-title').textContent = item.display_name;
  $('image-name').textContent = `Name · ${item.name}`;
  $('image-uploader').textContent = `上传者 #${item.uploader_id}`;
  $('detail-image-error').hidden = true;
  $('detail-image').hidden = false;
  $('detail-image').alt = item.display_name;
  $('detail-image').src = `/api/images/${item.manifest_hash}`;
  $('open-original').href = `/api/images/${item.manifest_hash}`;
  $('image-dialog').showModal();
}

function clearSearch() {
  $('search').value = '';
  state.query = '';
  loadGallery();
}

function renderSession() {
  $('session-loading').hidden = !['loading', 'error'].includes(state.session);
  $('session-loading').textContent = state.session === 'error' ? '登录状态暂不可用' : '正在确认登录…';
  $('login-link').hidden = state.session !== 'anonymous';
  $('account-name').hidden = state.session !== 'authenticated';
  $('logout').hidden = state.session !== 'authenticated';
  $('session-retry').hidden = state.session !== 'error';
  $('account-name').textContent = state.me ? `${state.me.login} · #${state.me.id}` : '';
  renderUploadAccess();
  if (state.tab === 'mine') loadMine();
}

function authenticatedMe(me) {
  if (!me) return false;
  return typeof me.login === 'string' && typeof me.csrf_token === 'string' && Boolean(me.csrf_token);
}

async function loadSession() {
  state.session = 'loading';
  renderSession();
  try {
    const me = await api('/api/me');
    if (!authenticatedMe(me)) throw new ApiError('登录信息不完整，请刷新页面后重试。');
    state.me = me;
    state.session = 'authenticated';
  } catch (error) {
    state.me = null;
    state.session = error.status === 401 ? 'anonymous' : 'error';
  }
  renderSession();
}

function expireSession() {
  state.me = null;
  state.session = 'anonymous';
  state.mineLoaded = false;
  state.mineEpoch += 1;
  $('submission-list').replaceChildren();
  renderSession();
}

function mineAccess() {
  const descriptions = {
    loading: { symbol: '···', title: '正在确认登录', text: '稍等一下，正在读取你的登录状态。' },
    anonymous: { symbol: '↗', title: '登录后查看你的投稿', text: '使用同一个 GitHub 账号，继续跟踪分享进度。', action: { label: '使用 GitHub 登录', href: '/auth/github' } },
    error: { symbol: '!', title: '暂时无法确认登录', text: '请稍后重试，图库仍可继续浏览。', error: true, action: { label: '重试', onClick: loadSession } },
  };
  stateBox('mine-status', descriptions[state.session]);
  $('refresh-mine').disabled = true;
}

function mineNeedsFetch(force) {
  if (state.session !== 'authenticated') { mineAccess(); return false; }
  return !state.mineBusy && (!state.mineLoaded || force);
}

function receiveMine(data) {
  if (!Array.isArray(data.items)) throw new ApiError('投稿记录无法识别，请稍后重试。');
  renderSubmissions(data.items);
  state.mineLoaded = true;
}

function finishMine(epoch) {
  state.mineBusy = false;
  $('refresh-mine').disabled = state.session !== 'authenticated';
  $('submission-list').setAttribute('aria-busy', 'false');
  const stillViewing = state.session === 'authenticated' && state.tab === 'mine';
  if (epoch !== state.mineEpoch && stillViewing) loadMine(true);
}

async function loadMine(force = false) {
  if (!mineNeedsFetch(force)) return;
  const epoch = state.mineEpoch;
  state.mineBusy = true;
  $('refresh-mine').disabled = true;
  $('submission-list').setAttribute('aria-busy', 'true');
  stateBox('mine-status', { symbol: '···', title: '正在读取投稿', text: '正在更新你的投稿进度。' });
  try {
    const data = await api('/api/submissions');
    if (epoch !== state.mineEpoch) return;
    receiveMine(data);
  } catch (error) {
    if (epoch === state.mineEpoch) mineFailure(error);
  } finally { finishMine(epoch); }
}

function mineFailure(error) {
  if (error.status === 401) { expireSession(); return; }
  stateBox('mine-status', { symbol: '!', title: '投稿进度暂时无法读取', text: errorText(error), error: true, action: { label: '重试', onClick: () => loadMine(true) } });
}

function renderSubmissions(items) {
  $('submission-list').replaceChildren(...items.map(submissionCard));
  $('mine-status').hidden = items.length > 0;
  if (items.length) return;
  stateBox('mine-status', { title: '你的第一张表情，还在路上', text: '选择一张想分享的图片，给它起个名字。', action: { label: '分享一张表情', onClick: openUpload } });
}

const STATUS_LABELS = {
  queued: ['待处理', 'pending'], pending: ['等待处理', 'pending'], processing: ['处理中', 'pending'],
  submitted: ['已创建 PR', ''], pr_open: ['等待合并', 'pending'], open: ['等待合并', 'pending'],
  merged: ['已合并', ''], approved: ['已通过', ''], failed: ['处理失败', 'failed'],
  rejected: ['未通过', 'failed'], closed: ['已关闭', 'unknown'],
  retrying: ['正在重试', 'pending'], awaiting_publish: ['已合并，等待上架', 'pending'], published: ['已上架', ''],
};

function submissionCard(item) {
  const card = element('article', 'submission');
  const info = element('div');
  info.append(element('h3', '', item.display_name), element('p', 'mono muted', item.name));
  const details = element('div', 'submission-details');
  details.append(element('span', '', `投稿 #${item.id}`), element('span', '', item.filename), submissionTime(item.created_at));
  info.append(details);
  const end = element('div', 'submission-end');
  const status = STATUS_LABELS[item.status] || [String(item.status || '状态未知'), 'unknown'];
  end.append(element('span', `badge ${status[1]}`, status[0]));
  const pr = safePrLink(item.pr_url);
  if (pr) end.append(pr);
  card.append(info, end);
  if (item.error) card.append(element('p', 'notice error', String(item.error)));
  return card;
}

function submissionTime(value) {
  const date = new Date(typeof value === 'number' ? value * 1000 : value);
  if (Number.isNaN(date.getTime())) return element('span', '', '时间未知');
  const time = element('time', '', date.toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' }));
  time.dateTime = date.toISOString();
  return time;
}

function validPrUrl(url) {
  const hostAllowed = url.hostname === 'github.com' && url.protocol === 'https:';
  const noCredentials = !url.username && !url.password;
  return hostAllowed && noCredentials && !url.port;
}

function parsePrUrl(value) {
  try {
    const url = new URL(value);
    return validPrUrl(url) ? url : null;
  } catch { return null; }
}

function safePrLink(value) {
  const url = parsePrUrl(value);
  if (!url) return null;
  const link = element('a', '', '查看 PR ↗');
  link.href = url.href;
  link.target = '_blank';
  link.rel = 'noopener noreferrer';
  return link;
}

function renderUploadAccess() {
  const allowed = state.session === 'authenticated';
  $('upload-gate').hidden = allowed;
  $('upload-form').hidden = !allowed || state.uploadSuccess;
  $('upload-success').hidden = !allowed || !state.uploadSuccess;
  if (allowed) {
    const email = state.me.contribution_email;
    $('contribution-email').hidden = !email;
    $('contribution-email').textContent = email ? `投稿署名邮箱：${email}` : '';
    return;
  }
  const content = {
    loading: { symbol: '···', title: '正在确认登录', text: '稍等一下，正在读取你的登录状态。' },
    error: { symbol: '!', title: '暂时无法确认登录', text: '请稍后重试。', error: true, action: { label: '重试', onClick: loadSession } },
    anonymous: { symbol: '↗', title: '先登录，再分享', text: '使用 GitHub 登录后，即可投稿并跟踪进度。', action: { label: '使用 GitHub 登录', href: '/auth/github' } },
  };
  stateBox('upload-gate', content[state.session]);
}

function openUpload() {
  state.uploadSuccess = false;
  renderUploadAccess();
  $('upload-dialog').showModal();
  if (state.session === 'authenticated') $('file').focus();
}

function resetFile() {
  if (state.previewUrl) URL.revokeObjectURL(state.previewUrl);
  state.previewUrl = null;
  state.file = null;
  $('file').value = '';
  $('file-preview').hidden = true;
  $('file-preview').removeAttribute('src');
  $('drop-placeholder').hidden = false;
  $('replace-file').hidden = true;
  $('file-summary').textContent = '尚未选择图片';
}

function fileMetadataProblem(file) {
  if (!IMAGE_TYPES.has(file.type)) return '请选择 PNG、JPEG、GIF 或 WebP 图片。';
  if (file.size === 0) return '这张图片是空文件，请重新选择。';
  if (file.size > MAX_FILE) return '图片超过 5 MiB，请选择更小的图片。';
  return '';
}

function uploadError(message) {
  $('upload-error').textContent = message;
  $('upload-error').hidden = !message;
  if (message) $('upload-error').focus();
}

function selectFile(files) {
  if (!files.length) return;
  const chosen = files[0];
  const problem = files.length !== 1 ? '每次只能投稿一张图片。' : fileMetadataProblem(chosen);
  resetFile();
  if (problem) { uploadError(problem); return; }
  uploadError('');
  state.file = chosen;
  state.previewUrl = URL.createObjectURL(chosen);
  $('file-preview').src = state.previewUrl;
  $('file-preview').hidden = false;
  $('drop-placeholder').hidden = true;
  $('replace-file').hidden = false;
  $('file-summary').textContent = `${chosen.name} · ${(chosen.size / 1024 / 1024).toFixed(2)} MiB`;
}

function uploadBody() {
  const body = new FormData();
  body.set('name', $('meme-name').value.trim());
  body.set('display_name', $('display-name').value.trim());
  body.set('file', state.file, state.file.name);
  body.set('consent', 'true');
  return body;
}

function setUploadBusy(busy) {
  state.uploadBusy = busy;
  $('upload-fields').disabled = busy;
  $('submit-upload').disabled = busy;
  $('submit-upload').textContent = busy ? '正在提交…' : '提交表情 ↑';
  $('logout').disabled = busy;
  $('upload-form').setAttribute('aria-busy', String(busy));
}

function validUpload() {
  if (!state.file) { uploadError('请先选择一张图片。'); return false; }
  const namesPresent = $('meme-name').value.trim() && $('display-name').value.trim();
  if (!namesPresent) { uploadError('请填写 Name 和显示名称。'); return false; }
  return $('upload-form').reportValidity();
}

function uploadReady() {
  if (state.uploadBusy) return false;
  if (state.session !== 'authenticated') { renderUploadAccess(); return false; }
  return validUpload();
}

function uploadFailure(error) {
  if (error.status === 401) expireSession();
  uploadError(errorText(error));
  notice(errorText(error), true);
}

async function submitUpload(event) {
  event.preventDefault();
  if (!uploadReady()) return;
  uploadError('');
  const csrf = state.me.csrf_token;
  const body = uploadBody();
  setUploadBusy(true);
  try {
    await api('/api/submissions', { method: 'POST', headers: { 'X-CSRF-Token': csrf }, body });
    uploadSucceeded();
  } catch (error) { uploadFailure(error); }
  finally { setUploadBusy(false); }
}

function uploadSucceeded() {
  state.uploadSuccess = true;
  state.mineLoaded = false;
  state.mineEpoch += 1;
  resetFile();
  $('upload-form').reset();
  renderUploadAccess();
  notice('投稿已收到，可在「我的投稿」查看后续进度。');
  if ($('upload-dialog').open) $('view-submission').focus();
  if (state.tab === 'mine') loadMine(true);
}

async function logout() {
  if (state.uploadBusy) return;
  $('logout').disabled = true;
  try {
    const response = await fetch('/auth/logout', {
      method: 'POST', credentials: 'same-origin', redirect: 'manual',
      headers: { 'X-CSRF-Token': state.me.csrf_token }, signal: AbortSignal.timeout(35000),
    });
    if (response.type !== 'opaqueredirect' && !response.ok) throw await responseError(response);
    expireSession();
    notice('已退出登录。');
  } catch (error) { notice(errorText(error), true); }
  finally { $('logout').disabled = false; }
}

function droppedFile(event) {
  event.preventDefault();
  $('drop-zone').classList.remove('dragging');
  if (state.uploadBusy) return;
  selectFile(event.dataTransfer.files);
}

function containDialogFocus(event) {
  if (event.key !== 'Tab') return;
  const selector = 'button:not(:disabled), a[href], input:not(:disabled), [tabindex="0"]';
  const controls = [...event.currentTarget.querySelectorAll(selector)].filter((control) => control.getClientRects().length > 0);
  const first = controls[0];
  const last = controls[controls.length - 1];
  const boundary = event.shiftKey ? first : last;
  if (document.activeElement !== boundary) return;
  event.preventDefault();
  (event.shiftKey ? last : first).focus();
}

function initEvents() {
  document.querySelectorAll('[data-upload]').forEach((button) => button.addEventListener('click', openUpload));
  document.querySelectorAll('[data-close]').forEach((button) => button.addEventListener('click', () => $(button.dataset.close).close()));
  document.querySelectorAll('dialog').forEach((dialog) => dialog.addEventListener('keydown', containDialogFocus));
  document.querySelector('.tabs').addEventListener('keydown', tabsKeydown);
  $('gallery-tab').addEventListener('click', () => showTab('gallery'));
  $('mine-tab').addEventListener('click', () => showTab('mine'));
  $('search-form').addEventListener('submit', (event) => {
    event.preventDefault(); state.query = $('search').value.trim(); loadGallery();
  });
  $('search').addEventListener('search', () => { if (!$('search').value) clearSearch(); });
  $('load-more').addEventListener('click', () => loadGallery(true));
  $('refresh-mine').addEventListener('click', () => loadMine(true));
  $('session-retry').addEventListener('click', loadSession);
  $('logout').addEventListener('click', logout);
  $('file').addEventListener('change', (event) => selectFile(event.target.files));
  $('drop-zone').addEventListener('dragover', (event) => { event.preventDefault(); if (!state.uploadBusy) $('drop-zone').classList.add('dragging'); });
  $('drop-zone').addEventListener('dragleave', () => $('drop-zone').classList.remove('dragging'));
  $('drop-zone').addEventListener('drop', droppedFile);
  $('file-preview').addEventListener('error', () => { if (state.file) { resetFile(); uploadError('图片无法预览，请确认文件有效后重新选择。'); } });
  $('detail-image').addEventListener('error', () => { $('detail-image').hidden = true; $('detail-image-error').hidden = false; });
  $('upload-form').addEventListener('submit', submitUpload);
  $('view-submission').addEventListener('click', () => { $('upload-dialog').close(); showTab('mine', true); });
  window.addEventListener('pagehide', () => { if (state.previewUrl) URL.revokeObjectURL(state.previewUrl); });
}

initEvents();
loadSession();
loadGallery();
