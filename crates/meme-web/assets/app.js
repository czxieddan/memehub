import { attribute, clearText, hasMessage, t, text } from './i18n.js';

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
  constructor(key, status = 0) { super(key); this.key = key; this.status = status; }
}

function element(tag, className = '', content = '') {
  const result = document.createElement(tag);
  result.className = className;
  result.textContent = content;
  return result;
}

function message(tag, className, key, values = {}) {
  return text(element(tag, className), key, values);
}

function errorKey(error) {
  return error instanceof ApiError ? error.key : 'error.network';
}

const HTTP_ERRORS = {
  400: 'bad_request', 401: 'unauthorized', 403: 'forbidden', 404: 'not_found',
  408: 'request_timeout', 409: 'conflict', 413: 'request_too_large',
  422: 'unprocessable', 429: 'too_many_requests', 503: 'upstream_unavailable',
  504: 'upstream_timeout',
};

function serviceErrorKey(code, fallback = 'error.service') {
  const key = `error.${code}`;
  return hasMessage(key) ? key : fallback;
}

async function responseError(response) {
  let body;
  try { body = await response.json(); } catch { body = null; }
  const fallback = serviceErrorKey(HTTP_ERRORS[response.status]);
  const code = body?.error?.code ?? body?.code;
  return new ApiError(serviceErrorKey(code, fallback), response.status);
}

async function api(path, options = {}) {
  const timeout = AbortSignal.timeout(35000);
  const signal = options.signal ? AbortSignal.any([options.signal, timeout]) : timeout;
  const response = await fetch(path, {
    credentials: 'same-origin', cache: 'no-store', redirect: 'error', ...options, signal,
  });
  if (!response.ok) throw await responseError(response);
  try { return await response.json(); }
  catch { throw new ApiError('error.invalidData'); }
}

function actionControl(action) {
  if (action.href) {
    const link = message('a', 'button primary', action.label);
    link.href = action.href;
    return link;
  }
  const button = message('button', 'button secondary', action.label);
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
  box.replaceChildren(symbol, message('h3', '', content.title), message('p', '', content.text));
  if (content.action) box.append(actionControl(content.action));
}

function notice(key, isError = false) {
  const box = $('global-notice');
  if (key) text(box, key); else clearText(box);
  box.classList.toggle('error', isError);
  box.hidden = !key;
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
  text($('load-more'), 'gallery.loadingMore');
  if (append) return controller;
  state.hashes.clear();
  state.cursor = null;
  $('gallery-grid').replaceChildren();
  clearText($('gallery-count'));
  $('load-more').hidden = true;
  text($('gallery-heading'), state.query ? 'gallery.results' : 'gallery.all');
  stateBox('gallery-status', { symbol: '···', title: 'gallery.loadingTitle', text: 'gallery.loadingText' });
  return controller;
}

async function loadGallery(append = false) {
  const controller = beginGallery(append);
  const previousCursor = state.cursor;
  const params = new URLSearchParams({ query: state.query, limit: '24' });
  if (append) params.set('cursor', previousCursor);
  try {
    const data = await api(`/api/gallery?${params}`, { signal: controller.signal });
    if (!Array.isArray(data.items)) throw new ApiError('error.galleryData');
    renderGallery(data, previousCursor);
  } catch (error) {
    if (controller.signal.aborted) return;
    galleryFailure(error, append);
  } finally {
    if (state.galleryController === controller) {
      $('gallery-grid').setAttribute('aria-busy', 'false');
      $('load-more').disabled = false;
      text($('load-more'), 'gallery.more');
    }
  }
}

function validGalleryItem(item) {
  if (!item) return false;
  const fieldsValid = typeof item.name === 'string' && typeof item.display_name === 'string';
  return fieldsValid && /^[a-f0-9]{64}$/.test(item.manifest_hash) && Number.isInteger(item.uploader_id);
}

function renderGallery(data, previousCursor) {
  if (!data.items.every(validGalleryItem)) throw new ApiError('error.galleryItem');
  $('gallery-status').hidden = true;
  for (const item of data.items) {
    if (state.hashes.has(item.manifest_hash)) continue;
    state.hashes.add(item.manifest_hash);
    $('gallery-grid').append(galleryCard(item));
  }
  const next = typeof data.next_cursor === 'string' ? data.next_cursor : null;
  state.cursor = next !== previousCursor ? next : null;
  $('load-more').hidden = !state.cursor;
  text($('gallery-count'), 'gallery.count', { count: state.hashes.size });
  if (!state.hashes.size) emptyGallery();
}

function emptyGallery() {
  const searching = Boolean(state.query);
  const action = searching
    ? { label: 'gallery.viewAll', onClick: clearSearch }
    : { label: 'gallery.shareFirst', onClick: openUpload };
  stateBox('gallery-status', {
    symbol: searching ? '⌕' : '☺', title: searching ? 'gallery.noMatches' : 'gallery.empty',
    text: searching ? 'gallery.noMatchesText' : 'gallery.emptyText', action,
  });
}

function galleryFailure(error, append) {
  stateBox('gallery-status', {
    symbol: '!', title: append ? 'gallery.moreFailed' : 'gallery.failed',
    text: errorKey(error), error: true,
    action: { label: 'gallery.reload', onClick: () => loadGallery(append) },
  });
  $('load-more').hidden = true;
}

function galleryCard(item) {
  const card = element('button', 'meme-card');
  card.type = 'button';
  attribute(card, 'aria-label', 'gallery.cardLabel', { name: item.display_name, id: String(item.uploader_id) });
  const frame = element('span', 'card-image');
  const image = element('img');
  image.alt = '';
  image.loading = 'lazy';
  image.decoding = 'async';
  image.src = `/api/images/${item.manifest_hash}`;
  image.addEventListener('error', () => frame.replaceChildren(message('span', 'card-image-error', 'gallery.imageFailed')));
  frame.append(image);
  const meta = element('span', 'card-meta');
  const uploader = element('span', 'card-uploader');
  const arrow = element('span', 'card-arrow', '↗');
  arrow.setAttribute('aria-hidden', 'true');
  uploader.append(message('span', '', 'gallery.uploader', { id: String(item.uploader_id) }), arrow);
  meta.append(element('span', 'card-title', item.display_name), element('span', 'card-name mono', item.name), uploader);
  card.append(frame, meta);
  card.addEventListener('click', () => openImage(item));
  return card;
}

function openImage(item) {
  $('image-title').textContent = item.display_name;
  text($('image-name'), 'image.name', { name: item.name });
  text($('image-uploader'), 'gallery.uploader', { id: String(item.uploader_id) });
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
  text($('session-loading'), state.session === 'error' ? 'session.unavailable' : 'session.loading');
  $('login-link').hidden = state.session !== 'anonymous';
  $('account-name').hidden = state.session !== 'authenticated';
  $('logout').hidden = state.session !== 'authenticated';
  $('session-retry').hidden = state.session !== 'error';
  if (state.me) text($('account-name'), 'session.account', { login: state.me.login, id: String(state.me.id) });
  else clearText($('account-name'));
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
    if (!authenticatedMe(me)) throw new ApiError('error.sessionData');
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

function accessMessage(scope) {
  const descriptions = {
    loading: { symbol: '···', title: 'session.checkTitle', text: 'session.checkText' },
    anonymous: { symbol: '↗', title: `${scope}.loginTitle`, text: `${scope}.loginText`, action: { label: 'common.login', href: '/auth/github' } },
    error: { symbol: '!', title: 'session.errorTitle', text: 'session.errorText', error: true, action: { label: 'common.retry', onClick: loadSession } },
  };
  return descriptions[state.session];
}

function mineAccess() {
  stateBox('mine-status', accessMessage('mine'));
  $('refresh-mine').disabled = true;
}

function mineNeedsFetch(force) {
  if (state.session !== 'authenticated') { mineAccess(); return false; }
  return !state.mineBusy && (!state.mineLoaded || force);
}

function receiveMine(data) {
  if (!Array.isArray(data.items)) throw new ApiError('error.submissionData');
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
  stateBox('mine-status', { symbol: '···', title: 'mine.loadingTitle', text: 'mine.loadingText' });
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
  stateBox('mine-status', { symbol: '!', title: 'mine.failed', text: errorKey(error), error: true, action: { label: 'common.retry', onClick: () => loadMine(true) } });
}

function renderSubmissions(items) {
  $('submission-list').replaceChildren(...items.map(submissionCard));
  $('mine-status').hidden = items.length > 0;
  if (items.length) return;
  stateBox('mine-status', { title: 'mine.empty', text: 'mine.emptyText', action: { label: 'common.share', onClick: openUpload } });
}

const STATUS_STYLES = {
  queued: 'pending', pending: 'pending', processing: 'pending', submitted: '',
  pr_open: 'pending', open: 'pending', merged: '', approved: '', failed: 'failed',
  rejected: 'failed', closed: 'unknown', retrying: 'pending', awaiting_publish: 'pending', published: '',
};

function submissionCard(item) {
  const card = element('article', 'submission');
  const info = element('div');
  info.append(element('h3', '', item.display_name), element('p', 'mono muted', item.name));
  const details = element('div', 'submission-details');
  details.append(message('span', '', 'submission.id', { id: String(item.id) }), element('span', '', item.filename), submissionTime(item.created_at));
  info.append(details);
  const end = element('div', 'submission-end');
  const status = Object.hasOwn(STATUS_STYLES, item.status) ? item.status : 'unknown';
  end.append(message('span', `badge ${STATUS_STYLES[status] ?? 'unknown'}`, `status.${status}`));
  const pr = safePrLink(item.pr_url);
  if (pr) end.append(pr);
  card.append(info, end);
  if (item.error) card.append(message('p', 'notice error', submissionErrorKey(item)));
  return card;
}

function submissionErrorKey(item) {
  const code = item.error_code ?? item.error?.code ?? item.error;
  return serviceErrorKey(code, 'error.submission');
}

function submissionTime(value) {
  const date = new Date(typeof value === 'number' ? value * 1000 : value);
  if (Number.isNaN(date.getTime())) return message('span', '', 'submission.unknownTime');
  const time = message('time', '', 'submission.time', { date: { date: date.toISOString() } });
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
  const link = message('a', '', 'submission.pr');
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
    if (email) text($('contribution-email'), 'upload.email', { email });
    else clearText($('contribution-email'));
    return;
  }
  stateBox('upload-gate', accessMessage('upload'));
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
  text($('file-summary'), 'upload.noFile');
}

function fileMetadataProblem(file) {
  if (!IMAGE_TYPES.has(file.type)) return 'validation.fileType';
  if (file.size === 0) return 'validation.fileEmpty';
  if (file.size > MAX_FILE) return 'validation.fileSize';
  return '';
}

function uploadError(key) {
  if (key) text($('upload-error'), key); else clearText($('upload-error'));
  $('upload-error').hidden = !key;
  if (key) $('upload-error').focus();
}

function selectFile(files) {
  if (!files.length) return;
  const chosen = files[0];
  const problem = files.length !== 1 ? 'validation.oneFile' : fileMetadataProblem(chosen);
  resetFile();
  if (problem) { uploadError(problem); return; }
  uploadError('');
  state.file = chosen;
  state.previewUrl = URL.createObjectURL(chosen);
  $('file-preview').src = state.previewUrl;
  $('file-preview').hidden = false;
  $('drop-placeholder').hidden = true;
  $('replace-file').hidden = false;
  text($('file-summary'), 'upload.fileSummary', {
    name: chosen.name,
    size: { number: chosen.size / 1024 / 1024, options: { minimumFractionDigits: 2, maximumFractionDigits: 2 } },
  });
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
  text($('submit-upload'), busy ? 'upload.submitting' : 'upload.submit');
  $('logout').disabled = busy;
  $('upload-form').setAttribute('aria-busy', String(busy));
}

function validUpload() {
  if (!state.file) { uploadError('validation.fileRequired'); return false; }
  const namesPresent = $('meme-name').value.trim() && $('display-name').value.trim();
  if (!namesPresent) { uploadError('validation.namesRequired'); return false; }
  return $('upload-form').reportValidity();
}

function validationKey(input) {
  const required = {
    'meme-name': 'validation.nameRequired', 'display-name': 'validation.displayRequired',
    consent: 'validation.consent',
  };
  const rules = [
    ['valueMissing', required[input.id]], ['patternMismatch', 'validation.namePattern'],
    ['tooLong', 'validation.tooLong'],
  ];
  return rules.find(([flag]) => input.validity[flag])?.[1] || input.dataset.validationKey || 'error.bad_request';
}

function invalidField(event) {
  const input = event.target;
  input.dataset.validationKey = validationKey(input);
  input.setCustomValidity(t(input.dataset.validationKey));
  input.setAttribute('aria-invalid', 'true');
}

function clearFieldError(input) {
  input.setCustomValidity('');
  input.removeAttribute('aria-invalid');
  delete input.dataset.validationKey;
}

function editedField(event) {
  clearFieldError(event.target);
  uploadError('');
}

function refreshValidation() {
  $('upload-form').querySelectorAll('[data-validation-key]').forEach((input) => {
    input.setCustomValidity(t(input.dataset.validationKey));
  });
}

function uploadReady() {
  if (state.uploadBusy) return false;
  if (state.session !== 'authenticated') { renderUploadAccess(); return false; }
  return validUpload();
}

function uploadFailure(error) {
  if (error.status === 401) expireSession();
  uploadError(errorKey(error));
  notice(errorKey(error), true);
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
  $('upload-form').querySelectorAll('[aria-invalid]').forEach(clearFieldError);
  renderUploadAccess();
  notice('upload.successNotice');
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
    notice('session.signedOut');
  } catch (error) { notice(errorKey(error), true); }
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
  $('file-preview').addEventListener('error', () => { if (state.file) { resetFile(); uploadError('validation.preview'); } });
  $('detail-image').addEventListener('error', () => { $('detail-image').hidden = true; $('detail-image-error').hidden = false; });
  $('upload-form').addEventListener('invalid', invalidField, true);
  $('upload-form').addEventListener('input', editedField);
  $('upload-form').addEventListener('submit', submitUpload);
  $('view-submission').addEventListener('click', () => { $('upload-dialog').close(); showTab('mine', true); });
  window.addEventListener('pagehide', () => { if (state.previewUrl) URL.revokeObjectURL(state.previewUrl); });
  window.addEventListener('memehub:localechange', refreshValidation);
}

initEvents();
loadSession();
loadGallery();
