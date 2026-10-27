(() => {
  'use strict';
  const $ = (id) => document.getElementById(id);
  const ui = Object.fromEntries(['connection', 'device-name', 'text-form', 'text-input', 'text-count', 'send-text', 'send-message', 'photo-input', 'queue-section', 'upload-queue'].map((id) => [id, $(id)]));
  const uploads = [];
  let textAttempt = null;
  let textSending = false;
  let photoRunning = false;

  function node(tag, className, text) {
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text !== undefined) element.textContent = String(text);
    return element;
  }
  function message(text, error = false) {
    ui['send-message'].textContent = text;
    ui['send-message'].hidden = !text;
    ui['send-message'].classList.toggle('error', error);
  }
  function uuid() {
    if (globalThis.crypto?.randomUUID) return crypto.randomUUID();
    const bytes = crypto.getRandomValues(new Uint8Array(16));
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    return [...bytes].map((byte, index) => ([4, 6, 8, 10].includes(index) ? '-' : '') + byte.toString(16).padStart(2, '0')).join('');
  }
  async function api(path, options = {}) {
    const response = await fetch(path, { cache: 'no-store', ...options });
    const data = await response.json().catch(() => null);
    if (!response.ok) throw new Error(data?.error || `请求失败（${response.status}）`);
    if (data === null) throw new Error('电脑返回了无法读取的响应');
    return data;
  }
  async function waitTask(id) {
    for (let attempt = 0; attempt < 120; attempt++) {
      const result = await api('/api/tasks');
      const task = (result.tasks || []).find((item) => item.id === id);
      if (task?.state === 'done') return task;
      if (task?.state === 'error') throw new Error(task.message || '电脑未能接收内容');
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    throw new Error('等待电脑确认超时，请重试');
  }
  async function finish(result) {
    if (!result.pending) return result;
    const task = await waitTask(result.task_id);
    return { ...task, task_id: task.id, clipboard_written: task.clipboard_written };
  }
  function updateTextCount() {
    ui['text-count'].textContent = ui['text-input'].value.length ? `${ui['text-input'].value.length.toLocaleString()} 个字符` : '文字会写入电脑剪贴板';
  }
  ui['text-input'].addEventListener('input', updateTextCount);
  ui['text-form'].addEventListener('submit', async (event) => {
    event.preventDefault();
    const text = ui['text-input'].value;
    if (!text.trim() || textSending) return;
    textSending = true;
    ui['send-text'].disabled = true;
    ui['send-text'].textContent = '正在发送…';
    message('');
    if (!textAttempt || textAttempt.text !== text) textAttempt = { text, request_id: uuid() };
    try {
      const response = await api('/api/text', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(textAttempt) });
      const result = await finish(response);
      message(result.message || '文字已接收');
      if (ui['text-input'].value === text) ui['text-input'].value = '';
      textAttempt = null;
      updateTextCount();
    } catch (error) { message(`发送失败：${error.message}`, true); }
    finally {
      textSending = false;
      ui['send-text'].disabled = false;
      ui['send-text'].textContent = '发送文字';
    }
  });
  function renderUploads() {
    ui['queue-section'].hidden = uploads.length === 0;
    ui['upload-queue'].replaceChildren(...uploads.map((upload) => {
      const row = node('li', 'upload-task');
      const info = node('div', 'task-info');
      info.append(node('div', 'task-name', upload.name));
      const labels = { queued: '等待发送', sending: '正在上传', processing: '等待电脑确认', done: '已完成', error: '发送失败' };
      info.append(node('p', `task-message${upload.state === 'error' ? ' error' : ''}`, upload.message || labels[upload.state]));
      if (upload.state === 'sending' || upload.state === 'processing') {
        const progress = node('progress');
        progress.max = 100;
        if (upload.state === 'sending') progress.value = upload.progress;
        info.append(progress);
      }
      row.append(info);
      if (upload.state === 'error') {
        const retry = node('button', 'text-button', '重试');
        retry.type = 'button';
        retry.addEventListener('click', () => { upload.state = 'queued'; upload.message = ''; renderUploads(); void runPhotos(); });
        row.append(retry);
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
      xhr.addEventListener('error', () => reject(new Error('连接中断，请检查网络后重试')));
      xhr.addEventListener('abort', () => reject(new Error('上传已中断，可重试')));
      xhr.addEventListener('timeout', () => reject(new Error('上传超时，请检查网络后重试')));
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
        upload.state = 'sending';
        upload.progress = 0;
        upload.message = '';
        renderUploads();
        try {
          const response = await uploadPhoto(upload);
          const result = await finish(response);
          upload.state = 'done';
          upload.message = result.message || '照片已接收';
          upload.file = null;
        } catch (error) { upload.state = 'error'; upload.message = error.message; }
        renderUploads();
      }
    } finally { photoRunning = false; }
  }
  $('choose-photos').addEventListener('click', () => ui['photo-input'].click());
  ui['photo-input'].addEventListener('change', () => {
    [...ui['photo-input'].files].forEach((file) => uploads.push({ name: file.name, file, requestId: uuid(), state: 'queued', progress: 0, message: '' }));
    ui['photo-input'].value = '';
    renderUploads();
    void runPhotos();
  });
  $('clear-completed').addEventListener('click', () => {
    for (let index = uploads.length - 1; index >= 0; index--) if (uploads[index].state === 'done') uploads.splice(index, 1);
    renderUploads();
  });
  async function refreshState() {
    try {
      const state = await api('/api/state');
      ui.connection.textContent = '已连接电脑';
      ui.connection.className = 'connection online';
      ui['device-name'].textContent = state.name ? `连接到 ${state.name}，照片与文字随手传。` : '照片与文字，在同一局域网轻松接力。';
    } catch (_) {
      ui.connection.textContent = '连接中断，请检查电脑与网络';
      ui.connection.className = 'connection offline';
    }
  }
  document.addEventListener('visibilitychange', () => { if (!document.hidden) void refreshState(); });
  window.addEventListener('online', () => void refreshState());
  window.addEventListener('offline', () => { ui.connection.textContent = '当前网络已断开'; ui.connection.className = 'connection offline'; });
  window.addEventListener('beforeunload', (event) => { if (photoRunning || textSending) { event.preventDefault(); event.returnValue = ''; } });
  void refreshState();
  setInterval(refreshState, 3000);
})();
