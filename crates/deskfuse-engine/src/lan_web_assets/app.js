(() => {
  'use strict';
  const $ = (id) => document.getElementById(id);
  const ui = Object.fromEntries(['connection', 'device-name', 'text-form', 'text-input', 'text-count', 'send-text', 'send-message', 'photo-input', 'queue-section', 'upload-queue', 'items', 'item-count', 'list-message', 'load-more', 'retry-list', 'search', 'list-filter', 'source-filter'].map((id) => [id, $(id)]));
  let revision = null;
  let items = [];
  let nextOffset = null;
  let queryGeneration = 0;
  let listController = null;
  let listLoading = false;
  let searchTimer;
  let toastTimer;
  let activeTab = 'transfer';
  let pollTimer;
  let polling = false;
  let photoRunning = false;
  let textSending = false;
  let textAttempt = null;
  let refreshNeeded = false;
  const uploads = [];
  const deviceNames = new Map();

  function node(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined && text !== null) element.textContent = String(text);
    return element;
  }
  function message(element, text, error = false) {
    element.textContent = text;
    element.hidden = !text;
    element.classList.toggle('error', error);
  }
  function toast(text) {
    $('toast').textContent = text;
    $('toast').hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => { $('toast').hidden = true; }, 3200);
  }
  function uuid() {
    if (globalThis.crypto?.randomUUID) return crypto.randomUUID();
    const bytes = crypto.getRandomValues(new Uint8Array(16));
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    return [...bytes].map((byte, index) => ([4, 6, 8, 10].includes(index) ? '-' : '') + byte.toString(16).padStart(2, '0')).join('');
  }
  async function api(path, options = {}) {
    const controller = new AbortController();
    let timedOut = false;
    const abort = () => controller.abort();
    if (options.signal?.aborted) abort();
    options.signal?.addEventListener('abort', abort, { once: true });
    const timeout = setTimeout(() => { timedOut = true; controller.abort(); }, 60000);
    try {
      const response = await fetch(path, { cache: 'no-store', ...options, signal: controller.signal });
      const data = await response.json().catch(() => null);
      if (!response.ok) throw new Error(data?.error || `请求失败（${response.status}）`);
      if (data === null) throw new Error('Mac 返回了无法读取的响应');
      return data;
    } catch (error) {
      if (timedOut) throw new Error('请求超时，请检查 Mac 和网络后重试');
      throw error;
    } finally {
      clearTimeout(timeout);
      options.signal?.removeEventListener('abort', abort);
    }
  }
  function itemPath(id, suffix = '') { return `/api/items/${encodeURIComponent(String(id))}${suffix}`; }
  function bytes(value) {
    const count = Number(value) || 0;
    if (count < 1024) return `${count} B`;
    if (count < 1048576) return `${(count / 1024).toFixed(1)} KB`;
    return `${(count / 1048576).toFixed(1)} MB`;
  }
  function date(value) {
    const timestamp = new Date(Number(value));
    if (!Number.isFinite(timestamp.getTime())) return '';
    return new Intl.DateTimeFormat('zh-CN', { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(timestamp);
  }
  function sourceName(source) { return deviceNames.get(String(source)) || String(source || '未知来源'); }
  function setConnected(online) {
    ui.connection.textContent = online ? '已连接 Mac' : '连接中断，请检查 Mac 与网络';
    ui.connection.className = `connection ${online ? 'online' : 'offline'}`;
  }
  function updateSelect(select, values, defaultLabel) {
    const previous = select.value;
    const options = [{ id: '', name: defaultLabel }, ...values.filter((value) => String(value.id) !== '')];
    const signature = JSON.stringify(options.map(({ id, name }) => [String(id), String(name)]));
    if (select.dataset.signature === signature) return false;
    select.replaceChildren(...options.map(({ id, name }) => {
      const option = node('option', '', name);
      option.value = String(id);
      return option;
    }));
    select.dataset.signature = signature;
    select.value = options.some((value) => String(value.id) === previous) ? previous : '';
    return select.value !== previous;
  }
  async function pollState() {
    if (document.hidden || polling) return;
    polling = true;
    try {
      const state = await api('/api/state');
      if (document.hidden) return;
      setConnected(true);
      ui['device-name'].textContent = state.name ? `连接到 ${state.name}，照片与文字随手传。` : '照片与文字，在同一局域网轻松接力。';
      deviceNames.clear();
      const devices = Array.isArray(state.devices) ? state.devices : [];
      devices.forEach((device) => deviceNames.set(String(device.id), device.name));
      const groupChanged = updateSelect(ui['list-filter'], state.lists || [], '全部分组');
      const sourceChanged = updateSelect(ui['source-filter'], devices, '全部来源');
      if (revision !== state.revision || groupChanged || sourceChanged || refreshNeeded) {
        revision = state.revision;
        if (activeTab !== 'browse') refreshNeeded = true;
        else if (listLoading && !groupChanged && !sourceChanged) refreshNeeded = true;
        else { refreshNeeded = false; await loadItems(false, true); }
      }
    } catch (error) {
      if (!document.hidden) {
        setConnected(false);
        if (revision === null && !items.length) {
          message(ui['list-message'], `连接失败：${error.message}。请保持 Mac 上的局域网传输开启。`, true);
          ui['retry-list'].hidden = false;
          ui.items.setAttribute('aria-busy', 'false');
        }
      }
    }
    finally { polling = false; }
  }
  function schedulePoll() {
    clearInterval(pollTimer);
    if (!document.hidden) { void pollState(); pollTimer = setInterval(pollState, 2000); }
  }
  function itemCard(item) {
    const card = node('article', 'item');
    card.dataset.itemId = String(item.id);
    const body = node('div', 'item-inner');
    if (item.has_image && item.available) {
      const image = node('img', 'item-image');
      image.src = itemPath(item.id, '/image?size=320');
      image.alt = item.title || '图片预览';
      image.loading = 'lazy';
      image.addEventListener('error', () => image.replaceWith(node('div', 'item-file', '暂无预览')), { once: true });
      const imageButton = node('button', 'image-open'); imageButton.type = 'button';
      imageButton.setAttribute('aria-label', '预览图片');
      imageButton.append(image); imageButton.addEventListener('click', () => openPreview(item));
      card.append(imageButton);
    } else if (item.file_count || item.has_image) {
      card.append(node('div', 'item-file', item.has_image ? '照片' : `${item.file_count} 个文件`));
    }
    const open = node('button', 'item-open'); open.type = 'button';
    open.setAttribute('aria-label', item.has_image ? '预览图片' : item.file_count ? '预览文件信息' : '预览文字');
    open.append(node('span', 'item-title', item.title || (item.has_image ? '照片' : item.file_count ? '文件' : '文字')));
    if (item.preview) open.append(node('span', 'item-preview', item.preview));
    open.addEventListener('click', () => openPreview(item)); body.append(open);
    const meta = node('div', 'item-meta');
    meta.append(node('span', 'source', sourceName(item.source)), node('span', '', date(item.copied_at_ms)));
    if (item.length) meta.append(node('span', '', bytes(item.length)));
    if (!item.available) meta.append(node('span', 'unavailable', item.remote ? '需在 Mac 获取' : '文件不可用'));
    body.append(meta);
    const actions = node('div', 'item-actions');
    const feedback = node('p', 'inline-message'); feedback.hidden = true; feedback.setAttribute('role', 'status');
    if (!item.has_image && !item.file_count && item.available) {
      const textarea = node('textarea', 'item-text');
      textarea.readOnly = true; textarea.hidden = true; textarea.setAttribute('aria-label', '完整文字');
      const copy = node('button', 'button secondary copy-action', '复制'); copy.type = 'button';
      let textPromise;
      const read = () => {
        if (!textPromise) textPromise = api(itemPath(item.id)).then((detail) => {
          if (typeof detail.text !== 'string') throw new Error('此内容没有可复制的文字');
          textarea.value = detail.text; return detail.text;
        }).catch((error) => { textPromise = null; throw error; });
        return textPromise;
      };
      copy.addEventListener('click', async () => {
        copy.disabled = true; copy.textContent = '读取中…'; message(feedback, '');
        try { await read(); if (card.isConnected) await copyText(textarea, feedback); }
        catch (error) { message(feedback, `复制失败：${error.message}`, true); }
        finally { copy.disabled = false; copy.textContent = '复制'; }
      });
      actions.append(copy); body.append(textarea);
    }
    if (item.available && (item.file_count || item.has_image)) {
      if (item.has_image) {
        // LAN HTTP cannot invoke a native photo-library writer. Open the full
        // image directly in the user's gesture so mobile long-press save works.
        const save = node('a', 'button secondary', '保存相册');
        save.href = itemPath(item.id, '/image'); save.target = '_blank'; save.rel = 'noopener';
        actions.append(save);
      }
      const download = node('a', 'button secondary', item.has_image ? '下载原文件' : '下载文件');
      download.href = itemPath(item.id, '/download'); download.download = ''; actions.append(download);
    }
    body.append(actions, feedback); card.append(body);
    return card;
  }
  let previewController;
  let previewGeneration = 0;
  async function openPreview(item) {
    const generation = ++previewGeneration;
    previewController?.abort();
    const controller = new AbortController(); previewController = controller;
    const dialog = $('preview-dialog'), content = $('preview-content');
    $('preview-title').textContent = item.title || (item.has_image ? '图片预览' : item.file_count ? '文件信息' : '文字预览');
    $('preview-meta').textContent = `${sourceName(item.source)} · ${date(item.copied_at_ms)}`;
    content.replaceChildren(node('p', 'preview-status', '正在加载…'));
    content.setAttribute('aria-busy', 'true');
    if (!dialog.open) dialog.showModal();
    const current = () => generation === previewGeneration && dialog.open;
    try {
      const detail = await api(itemPath(item.id), { signal: controller.signal });
      if (!current()) return;
      if (detail.has_image && detail.available) {
        const image = node('img', 'preview-image'); image.alt = item.title || '图片预览';
        image.addEventListener('load', () => { if (current()) { content.replaceChildren(image); content.setAttribute('aria-busy', 'false'); } }, { once: true });
        image.addEventListener('error', () => { if (current()) { content.replaceChildren(node('p', 'preview-status', '图片预览不可用，请返回卡片下载原文件。')); content.setAttribute('aria-busy', 'false'); } }, { once: true });
        image.src = itemPath(item.id, '/image');
      } else {
        const text = node('pre', 'preview-text');
        text.textContent = typeof detail.text === 'string' && detail.text.length ? detail.text :
          !detail.available ? '内容尚未保存在此 Mac，或文件已不可用。' :
          (detail.files || []).map((file) => file.name).join('\n') || '没有可预览的内容';
        content.replaceChildren(text); content.setAttribute('aria-busy', 'false');
      }
    } catch (error) {
      if (!current() || error.name === 'AbortError') return;
      content.setAttribute('aria-busy', 'false');
      const retry = node('button', 'button secondary', '重新加载'); retry.type = 'button';
      retry.addEventListener('click', () => openPreview(item));
      const status = node('div', 'preview-status'); status.append(node('p', '', `读取失败：${error.message}`), retry); content.replaceChildren(status);
    }
  }
  $('preview-close').addEventListener('click', () => $('preview-dialog').close());
  $('preview-dialog').addEventListener('close', () => { previewGeneration++; previewController?.abort(); $('preview-content').replaceChildren(); });
  $('preview-dialog').addEventListener('click', (event) => {
    if (event.target !== $('preview-dialog')) return;
    const box = event.target.getBoundingClientRect();
    if (event.clientX < box.left || event.clientX > box.right || event.clientY < box.top || event.clientY > box.bottom) event.target.close();
  });
  async function loadItems(append = false, preserve = false) {
    if (append && (listLoading || nextOffset === null)) return;
    const offset = append ? nextOffset : 0;
    const generation = ++queryGeneration;
    listController?.abort();
    listController = new AbortController();
    listLoading = true;
    ui.items.setAttribute('aria-busy', 'true');
    ui['load-more'].disabled = true;
    ui['retry-list'].hidden = true;
    if (!append && !items.length) message(ui['list-message'], '正在读取剪贴板列表…');
    const wanted = preserve ? Math.max(40, items.length) : 40;
    const params = new URLSearchParams({ query: ui.search.value.trim(), list_id: ui['list-filter'].value, source: ui['source-filter'].value, offset: String(offset), limit: '40' });
    try {
      // Refresh the entire loaded range, not just page one. Commit atomically so
      // a failed later page does not discard the user's current reading position.
      const signal = listController.signal;
      let result;
      const incoming = [];
      do {
        params.set('limit', String(Math.min(100, wanted - incoming.length)));
        result = await api(`/api/items?${params}`, { signal });
        if (generation !== queryGeneration) return;
        const page = Array.isArray(result.items) ? result.items : [];
        incoming.push(...page);
        if (!preserve || incoming.length >= wanted || result.next_offset == null || !page.length) break;
        if (Number(result.next_offset) <= Number(params.get('offset'))) throw new Error('分页游标无效');
        params.set('offset', String(result.next_offset));
      } while (true);
      const known = new Set(append ? items.map((item) => String(item.id)) : []);
      const unique = incoming.filter((item) => { const id = String(item.id); if (known.has(id)) return false; known.add(id); return true; });
      const unchanged = !append && JSON.stringify(items) === JSON.stringify(unique);
      const anchor = preserve ? [...ui.items.children].find((card) => card.getBoundingClientRect().bottom > 0) : null;
      const anchorTop = anchor?.getBoundingClientRect().top;
      items = append ? [...items, ...unique] : unique;
      nextOffset = result.next_offset ?? null;
      if (append) ui.items.append(...unique.map(itemCard));
      else if (!unchanged) {
        ui.items.replaceChildren(...items.map(itemCard));
        const replacement = anchor && [...ui.items.children].find((card) => card.dataset.itemId === anchor.dataset.itemId);
        if (replacement) window.scrollBy(0, replacement.getBoundingClientRect().top - anchorTop);
      }
      ui['item-count'].textContent = `共 ${Number(result.total) || 0} 条内容`;
      message(ui['list-message'], items.length ? '' : (ui.search.value || ui['list-filter'].value || ui['source-filter'].value ? '没有符合条件的内容，试试其他关键词或筛选。' : '列表还是空的。发送一段文字或一张照片，就会出现在这里。'));
      ui['load-more'].hidden = nextOffset === null;
      setConnected(true);
    } catch (error) {
      if (error.name === 'AbortError' || generation !== queryGeneration) return;
      refreshNeeded = true;
      message(ui['list-message'], `读取失败：${error.message}${items.length ? '。下方保留上次读取的内容。' : ''}`, true);
      ui['retry-list'].hidden = false;
      ui['load-more'].hidden = true;
    } finally {
      if (generation === queryGeneration) { listLoading = false; ui.items.setAttribute('aria-busy', 'false'); ui['load-more'].disabled = false; }
    }
  }
  function changeFilter() {
    clearTimeout(searchTimer);
    items = [];
    nextOffset = null;
    ui['load-more'].hidden = true;
    ui.items.replaceChildren();
    void loadItems(false);
  }
  async function copyText(textarea, feedback) {
    if (window.isSecureContext && navigator.clipboard?.writeText) {
      try { await navigator.clipboard.writeText(textarea.value); toast('文字已复制'); return; } catch (_) { /* Use the HTTP fallback. */ }
    }
    const wasHidden = textarea.hidden;
    const x = scrollX, y = scrollY;
    textarea.hidden = false;
    textarea.focus({ preventScroll: true }); textarea.select();
    textarea.setSelectionRange(0, textarea.value.length);
    let copied = false;
    try { copied = document.execCommand('copy'); } catch (_) { /* Leave selectable text inline. */ }
    if (copied) { textarea.hidden = wasHidden; window.scrollTo(x, y); toast('文字已复制'); }
    else message(feedback, '文字已选中，请长按选区选择“复制”，或按 ⌘C / Ctrl+C。也可再次点击复制。');
  }
  function renderUploads() {
    ui['queue-section'].hidden = uploads.length === 0;
    ui['upload-queue'].replaceChildren(...uploads.map((upload) => {
      const row = node('li', 'upload-task');
      const info = node('div', 'task-info');
      info.append(node('div', 'task-name', upload.name));
      const labels = { queued: '等待发送', sending: '正在上传', processing: 'Mac 正在处理', done: '已加入列表', error: '发送失败' };
      info.append(node('p', `task-message${upload.state === 'error' ? ' error' : ''}`, upload.message || labels[upload.state]));
      if (upload.state === 'sending' || upload.state === 'processing') {
        const progress = node('progress'); progress.max = 100;
        if (upload.state === 'sending') progress.value = upload.progress;
        progress.setAttribute('aria-label', `${upload.name} 的上传进度`); info.append(progress);
      }
      row.append(info);
      if (upload.state === 'error') {
        const retry = node('button', 'text-button', '重试'); retry.type = 'button';
        retry.addEventListener('click', () => { upload.state = 'queued'; upload.message = ''; renderUploads(); void runPhotos(); }); row.append(retry);
      } else if (upload.state === 'done') row.append(node('span', 'task-state', '已完成'));
      else if (upload.state === 'sending') row.append(node('span', 'task-state', `${upload.progress}%`));
      return row;
    }));
  }
  function uploadPhoto(upload) {
    return new Promise((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open('POST', '/api/photos');
      xhr.responseType = 'json';
      xhr.timeout = 15 * 60 * 1000;
      xhr.upload.addEventListener('progress', (event) => {
        if (event.lengthComputable) upload.progress = Math.min(100, Math.round(event.loaded / event.total * 100));
        if (event.lengthComputable && event.loaded === event.total) upload.state = 'processing';
        renderUploads();
      });
      xhr.addEventListener('load', () => {
        const result = xhr.response;
        if (xhr.status >= 200 && xhr.status < 300 && result) resolve(result);
        else reject(new Error(result?.error || `上传失败（${xhr.status}）`));
      });
      xhr.addEventListener('error', () => reject(new Error('连接中断。恢复网络后可重试，已接收的照片不会重复加入列表。')));
      xhr.addEventListener('abort', () => reject(new Error('上传已中断，可重试。')));
      xhr.addEventListener('timeout', () => reject(new Error('上传超时，请检查网络后重试。')));
      const form = new FormData();
      form.append('request_id', upload.requestId);
      form.append('photo', upload.file, upload.name);
      xhr.send(form);
    });
  }
  async function runPhotos() {
    if (photoRunning) return;
    photoRunning = true;
    try {
      let upload;
      while ((upload = uploads.find((entry) => entry.state === 'queued'))) {
        upload.state = 'sending'; upload.progress = 0; upload.message = ''; renderUploads();
        try {
          const result = await uploadPhoto(upload);
          upload.state = 'done';
          upload.message = result.message || (result.clipboard_written ? '已加入列表，并写入 Mac 剪贴板。' : '已加入列表，未覆盖 Mac 当前剪贴板。');
          upload.file = null;
          refreshNeeded = true;
          if (!document.hidden) void pollState();
        } catch (error) { upload.state = 'error'; upload.message = error.message; }
        renderUploads();
      }
    } finally { photoRunning = false; }
  }
  ui['text-form'].addEventListener('submit', async (event) => {
    event.preventDefault();
    const text = ui['text-input'].value;
    if (!text.trim() || textSending) return;
    textSending = true; ui['send-text'].disabled = true; ui['send-text'].textContent = '正在发送…';
    message(ui['send-message'], '');
    if (!textAttempt || textAttempt.text !== text) textAttempt = { text, request_id: uuid() };
    try {
      const result = await api('/api/text', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(textAttempt) });
      message(ui['send-message'], result.message || (result.clipboard_written ? '文字已加入列表，并写入 Mac 剪贴板。' : '文字已加入列表，未覆盖 Mac 当前剪贴板。'));
      if (ui['text-input'].value === text) ui['text-input'].value = '';
      textAttempt = null; updateTextCount(); refreshNeeded = true; void pollState();
    } catch (error) { message(ui['send-message'], `发送失败：${error.message}`, true); }
    finally { textSending = false; ui['send-text'].disabled = false; ui['send-text'].textContent = '发送文字'; }
  });
  function updateTextCount() { ui['text-count'].textContent = ui['text-input'].value.length ? `${ui['text-input'].value.length.toLocaleString()} 个字符` : '文字会加入 Mac 的剪贴板列表'; }
  ui['text-input'].addEventListener('input', updateTextCount);
  $('choose-photos').addEventListener('click', () => ui['photo-input'].click());
  ui['photo-input'].addEventListener('change', () => {
    [...ui['photo-input'].files].forEach((file) => uploads.push({ name: file.name, file, requestId: uuid(), state: 'queued', progress: 0, message: '' }));
    ui['photo-input'].value = ''; renderUploads(); void runPhotos();
  });
  $('clear-completed').addEventListener('click', () => { for (let index = uploads.length - 1; index >= 0; index--) if (uploads[index].state === 'done') uploads.splice(index, 1); renderUploads(); });
  ui.search.addEventListener('input', () => {
    clearTimeout(searchTimer);
    queryGeneration++;
    listController?.abort();
    listLoading = false;
    nextOffset = null;
    ui['load-more'].hidden = true;
    searchTimer = setTimeout(changeFilter, 250);
  });
  ui['list-filter'].addEventListener('change', changeFilter);
  ui['source-filter'].addEventListener('change', changeFilter);
  ui['load-more'].addEventListener('click', () => loadItems(true));
  ui['retry-list'].addEventListener('click', () => loadItems(false));
  function selectTab(name, focus = false) {
    activeTab = name;
    for (const tab of ['transfer', 'browse']) {
      const selected = tab === name;
      $(`tab-${tab}`).setAttribute('aria-selected', String(selected));
      $(`tab-${tab}`).tabIndex = selected ? 0 : -1;
      $(`panel-${tab}`).hidden = !selected;
    }
    if (focus) $(`tab-${name}`).focus();
    if (name === 'browse') { refreshNeeded = true; void pollState(); }
  }
  for (const name of ['transfer', 'browse']) {
    $(`tab-${name}`).addEventListener('click', () => selectTab(name));
    $(`tab-${name}`).addEventListener('keydown', (event) => {
      if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      selectTab(event.key === 'Home' ? 'transfer' : event.key === 'End' ? 'browse' : activeTab === 'transfer' ? 'browse' : 'transfer', true);
    });
  }
  function setView(view) {
    const mode = view === 'masonry' ? 'masonry' : 'list';
    ui.items.className = `items ${mode}`;
    $('view-list').setAttribute('aria-pressed', String(mode === 'list'));
    $('view-masonry').setAttribute('aria-pressed', String(mode === 'masonry'));
    try { localStorage.setItem('deskfuse.lan.view', mode); } catch (_) { /* Private browsing may disable storage. */ }
  }
  $('view-list').addEventListener('click', () => setView('list'));
  $('view-masonry').addEventListener('click', () => setView('masonry'));
  try { setView(localStorage.getItem('deskfuse.lan.view')); } catch (_) { setView('list'); }
  document.addEventListener('visibilitychange', () => { if (!document.hidden) refreshNeeded = true; schedulePoll(); });
  window.addEventListener('online', () => { refreshNeeded = true; void pollState(); });
  window.addEventListener('offline', () => setConnected(false));
  window.addEventListener('beforeunload', (event) => { if (photoRunning || textSending || uploads.some((upload) => upload.state === 'error')) { event.preventDefault(); event.returnValue = ''; } });
  schedulePoll();
})();
