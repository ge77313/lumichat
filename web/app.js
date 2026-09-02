const $ = (s) => document.querySelector(s);
const state = { token: localStorage.getItem('lumichat_token'), user: null, channels: [], users: [], context: null, messages: [], upload: null, reply: null, ws: null, unread: JSON.parse(localStorage.getItem('lumichat_unread') || '{}'), call: null, audit: null };
const emojis = ['😀','😂','🥰','😍','🤔','😎','😭','😡','👍','👏','🙏','💪','🎉','❤️','🔥','✨','✅','👀'];

function toast(message) { const el = $('#toast'); el.textContent = message; el.classList.add('show'); clearTimeout(toast.timer); toast.timer = setTimeout(() => el.classList.remove('show'), 2400); }
async function api(path, options = {}) {
  const headers = new Headers(options.headers || {});
  if (state.token) headers.set('Authorization', `Bearer ${state.token}`);
  if (options.body && !(options.body instanceof FormData)) headers.set('Content-Type', 'application/json');
  const response = await fetch(`/api${path}`, { ...options, headers });
  if (response.status === 204) return null;
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || '请求失败');
  return data;
}
function initials(name = '?') { return name.trim().slice(0, 2).toUpperCase(); }
function escapeHtml(value = '') { const d = document.createElement('div'); d.textContent = value; return d.innerHTML; }
function formatTime(ts) { return new Date(ts * 1000).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' }); }
function contextKey(context = state.context) { return context ? `${context.type}:${context.id}` : ''; }
function setConnection(online) { $('#connection-dot').classList.toggle('online', online); $('#connection-label').textContent = online ? '实时连接正常' : '正在重新连接…'; }
function saveUnread() { localStorage.setItem('lumichat_unread', JSON.stringify(state.unread)); }

async function boot() {
  if (!state.token) return showAuth();
  try {
    state.user = await api('/me'); $('#auth').hidden = true; $('#app').hidden = false; updateProfileUi();
    await Promise.all([loadChannels(), loadUsers()]);
    const saved = localStorage.getItem('lumichat_context'); const [type, id] = saved ? saved.split(':') : [];
    const target = type === 'channel' ? state.channels.find(x => x.id == id) : state.users.find(x => x.id == id);
    if (target) await openContext({ type, ...target }); else if (state.channels[0]) await openContext({ type: 'channel', ...state.channels[0] });
    connectSocket();
  } catch { clearSession(); showAuth(); }
}
function updateProfileUi() { const avatar = initials(state.user.display_name); $('#profile-button').textContent = avatar; $('#sidebar-avatar').textContent = avatar; $('#sidebar-name').textContent = state.user.display_name; $('#manage-button').hidden = state.user.role !== 'admin'; $('#audit-button').hidden = state.user.role !== 'admin'; }
function showAuth() { $('#auth').hidden = false; $('#app').hidden = true; }
function clearSession() { endCall(false); state.token = null; state.user = null; localStorage.removeItem('lumichat_token'); if (state.ws) state.ws.close(); }

let registering = false;
$('#auth-toggle').addEventListener('click', () => {
  registering = !registering; $('#auth-title').textContent = registering ? '创建你的空间' : '登录 LumiChat';
  $('#auth-subtitle').textContent = registering ? '第一个注册用户将成为管理员' : '继续你的团队对话'; $('#display-field').hidden = !registering;
  $('#auth-form .primary').textContent = registering ? '创建账号' : '登录'; $('#auth-toggle').textContent = registering ? '已有账号？返回登录' : '没有账号？创建一个';
  $('#auth-form [name=password]').autocomplete = registering ? 'new-password' : 'current-password';
});
$('#auth-form').addEventListener('submit', async (event) => {
  event.preventDefault(); $('#auth-error').textContent = ''; const values = Object.fromEntries(new FormData(event.currentTarget));
  try { const result = await api(registering ? '/register' : '/login', { method: 'POST', body: JSON.stringify(values) }); state.token = result.token; localStorage.setItem('lumichat_token', result.token); await boot(); }
  catch (error) { $('#auth-error').textContent = error.message; }
});

async function loadChannels() { state.channels = await api('/channels'); $('#channel-list').innerHTML = state.channels.map(c => navItem('channel', c, '<b>#</b>', c.name)).join(''); bindNavigation(); }
async function loadUsers() {
  state.users = await api('/users'); const users = state.users.filter(u => u.id !== state.user.id && u.active);
  $('#user-list').innerHTML = users.map(u => navItem('dm', u, `<span class="mini-avatar">${escapeHtml(initials(u.display_name))}</span>`, u.display_name)).join('') || '<p style="padding:8px 10px;color:var(--muted);font-size:11px">还没有其他成员</p>'; bindNavigation();
}
function navItem(type, item, icon, label) { const key = `${type}:${item.id}`, unread = state.unread[key] || 0; return `<button class="nav-item" data-type="${type}" data-id="${item.id}">${icon}<span>${escapeHtml(label)}</span>${unread ? `<em class="unread">${Math.min(unread,99)}</em>` : ''}</button>`; }
function bindNavigation() { document.querySelectorAll('.nav-item[data-id]').forEach(el => el.onclick = () => { const item = el.dataset.type === 'channel' ? state.channels.find(x => x.id == el.dataset.id) : state.users.find(x => x.id == el.dataset.id); if (item) openContext({ type: el.dataset.type, ...item }); }); }

async function openContext(context) {
  state.context = context; state.upload = null; state.reply = null; state.messages = []; localStorage.setItem('lumichat_context', contextKey());
  state.unread[contextKey()] = 0; saveUnread(); $('#upload-preview').hidden = true; $('#reply-preview').hidden = true; $('#emoji-picker').hidden = true;
  document.querySelectorAll('.nav-item').forEach(el => el.classList.toggle('active', el.dataset.type === context.type && el.dataset.id == context.id));
  $('#context-icon').textContent = context.type === 'channel' ? '#' : initials(context.display_name); $('#context-title').textContent = context.type === 'channel' ? context.name : context.display_name;
  $('#context-subtitle').textContent = context.type === 'channel' ? (context.description || '公开频道') : `@${context.username} · 私密对话`;
  $('#audio-call').hidden = context.type !== 'dm'; $('#video-call').hidden = context.type !== 'dm';
  $('#message-input').placeholder = context.type === 'channel' ? `发送消息到 #${context.name}` : `发送私信给 ${context.display_name}`; closeSidebar();
  $('#message-stream').innerHTML = '<div class="empty-state">正在加载…</div>';
  try { const rows = await api(historyPath()); state.messages = rows; renderMessages(false); $('#load-older').hidden = rows.length < 60; refreshNav(); requestAnimationFrame(() => { $('#message-list').scrollTop = $('#message-list').scrollHeight; }); }
  catch (error) { toast(error.message); }
}
function refreshNav() { const key = contextKey(); document.querySelectorAll('.nav-item').forEach(el => { const elKey = `${el.dataset.type}:${el.dataset.id}`; el.classList.toggle('active', elKey === key); const badge = el.querySelector('.unread'); if (badge && !state.unread[elKey]) badge.remove(); }); }
function historyPath(before) { const base = state.context.type === 'channel' ? `/channels/${state.context.id}/messages` : `/dm/${state.context.id}/messages`; return before ? `${base}?before=${before}` : base; }
function renderMessages(preserve = true) {
  const list = $('#message-list'), oldHeight = list.scrollHeight;
  $('#message-stream').innerHTML = state.messages.length ? `<div class="day-divider">最近消息</div>${state.messages.map(messageHtml).join('')}` : `<div class="empty-state"><div><b>从这里开始</b>${state.context.type === 'channel' ? '发出这个频道的第一条消息。' : '这段私密对话还没有消息。'}</div></div>`;
  bindMessageActions(); if (preserve) list.scrollTop += list.scrollHeight - oldHeight;
}
function messageBodyHtml(body) { const safe = escapeHtml(body); const parts = safe.split('\n\n'); if (parts[0]?.startsWith('&gt; ')) return `<div class="message-quote">${parts.shift().slice(5)}</div><div class="message-body">${parts.join('\n\n')}</div>`; return `<div class="message-body">${safe}</div>`; }
function messageHtml(m) {
  const own = m.sender.id === state.user.id, canDelete = own || state.user.role === 'admin', image = m.file_url && /\.(png|jpe?g|gif|webp|avif)$/i.test(m.file_url);
  const file = m.file_url ? (image ? `<a href="${encodeURI(m.file_url)}" target="_blank" rel="noopener"><img class="file-image" src="${encodeURI(m.file_url)}" alt="图片附件" loading="lazy"></a>` : `<a class="message-file" href="${encodeURI(m.file_url)}" target="_blank" rel="noopener">↗ 打开附件</a>`) : '';
  const actions = `<div class="message-actions"><button data-reply="${m.id}">回复</button><button data-copy="${m.id}">复制</button>${own ? `<button data-edit="${m.id}">编辑</button>` : ''}${canDelete ? `<button data-delete="${m.id}">删除</button>` : ''}</div>`;
  return `<article class="message" data-message="${m.id}"><div class="message-avatar">${escapeHtml(initials(m.sender.display_name))}</div><div><div class="message-meta"><strong>${escapeHtml(m.sender.display_name)}</strong><time>${formatTime(m.created_at)}</time></div>${messageBodyHtml(m.body)}${file}</div>${actions}</article>`;
}
function bindMessageActions() {
  document.querySelectorAll('[data-reply]').forEach(b => b.onclick = () => setReply(+b.dataset.reply));
  document.querySelectorAll('[data-copy]').forEach(b => b.onclick = async () => { const m = state.messages.find(x => x.id == b.dataset.copy); await navigator.clipboard.writeText(m?.body || ''); toast('消息已复制'); });
  document.querySelectorAll('[data-edit]').forEach(b => b.onclick = () => editMessage(+b.dataset.edit)); document.querySelectorAll('[data-delete]').forEach(b => b.onclick = () => deleteMessage(+b.dataset.delete));
}
function setReply(id) { const m = state.messages.find(x => x.id === id); if (!m) return; state.reply = m; $('#reply-preview span').textContent = `回复 ${m.sender.display_name}：${m.body.slice(0, 55) || '附件'}`; $('#reply-preview').hidden = false; $('#message-input').focus(); }
async function editMessage(id) {
  const m = state.messages.find(x => x.id === id); if (!m) return;
  showDialog('编辑消息', `<label>消息内容<textarea id="edit-message" rows="5" maxlength="4000">${escapeHtml(m.body)}</textarea></label><div class="dialog-actions"><button value="cancel">取消</button><button id="save-message" type="button" class="primary">保存</button></div>`, 'MESSAGE', () => {
    $('#save-message').onclick = async () => { try { const body = $('#edit-message').value.trim(); await api(`/messages/${id}`, {method:'PATCH',body:JSON.stringify({body})}); m.body = body; $('#dialog').close(); renderMessages(false); } catch(e) { toast(e.message); } };
  });
}
async function deleteMessage(id) { if (!confirm('删除这条消息？此操作无法撤销。')) return; try { await api(`/messages/${id}`, {method:'DELETE'}); state.messages = state.messages.filter(x => x.id !== id); renderMessages(false); } catch(e) { toast(e.message); } }
$('#load-older').onclick = async () => { const oldest = state.messages[0]?.id; if (!oldest) return; const rows = await api(historyPath(oldest)); state.messages = [...rows, ...state.messages]; renderMessages(true); $('#load-older').hidden = rows.length < 60; };

$('#composer').addEventListener('submit', async (event) => {
  event.preventDefault(); let body = $('#message-input').value.trim(); if (!body && !state.upload) return;
  if (state.reply) body = `> ${state.reply.sender.display_name}: ${(state.reply.body || '附件').replace(/\n/g,' ').slice(0,80)}\n\n${body}`;
  const path = state.context.type === 'channel' ? `/channels/${state.context.id}/messages` : `/dm/${state.context.id}/messages`;
  try { const result = await api(path, { method: 'POST', body: JSON.stringify({ body, file_url: state.upload?.url }) }); $('#message-input').value = ''; $('#message-input').style.height = ''; clearAttachment(); clearReply(); appendIfRelevant(result); }
  catch (error) { toast(error.message); }
});
$('#message-input').addEventListener('input', event => { event.target.style.height = ''; event.target.style.height = `${Math.min(event.target.scrollHeight, 150)}px`; });
$('#message-input').addEventListener('keydown', event => { if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) { event.preventDefault(); $('#composer').requestSubmit(); } });
$('#file-input').addEventListener('change', async (event) => {
  const file = event.target.files[0]; if (!file) return; if (file.size > 10 * 1024 * 1024) { toast('文件不能超过 10 MB'); return; }
  const data = new FormData(); data.append('file', file); $('#upload-preview').hidden = false; $('#upload-preview span').textContent = `正在上传 ${file.name}…`;
  try { state.upload = await api('/upload', { method: 'POST', body: data }); $('#upload-preview span').textContent = `已附加：${state.upload.name}`; } catch (error) { clearAttachment(); toast(error.message); } event.target.value = '';
});
function clearAttachment() { state.upload = null; $('#upload-preview').hidden = true; }
function clearReply() { state.reply = null; $('#reply-preview').hidden = true; }
$('#upload-preview button').onclick = clearAttachment; $('#reply-preview button').onclick = clearReply;

function connectSocket() {
  if (state.ws) state.ws.close(); setConnection(false); const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
  state.ws = new WebSocket(`${protocol}//${location.host}/api/ws?token=${encodeURIComponent(state.token)}`); state.ws.onopen = () => setConnection(true);
  state.ws.onmessage = event => { const data = JSON.parse(event.data); if (data.type === 'message') { appendIfRelevant(data); markUnread(data); } if (data.type === 'message_updated') updateIncoming(data); if (data.type === 'message_deleted') deleteIncoming(data); if (data.type === 'messages_cleared') messagesCleared(); if (data.type === 'channel_created') loadChannels(); if (['call_offer','call_answer','ice_candidate','call_end','call_reject'].includes(data.type)) handleCallSignal(data); };
  state.ws.onclose = () => { setConnection(false); if (state.token) setTimeout(connectSocket, 1800); };
}
function eventKey(event) { if (event.scope === 'channel') return `channel:${event.channel_id}`; const sender = event.message?.sender.id ?? event.sender_id; const other = sender === state.user.id ? event.recipient_id : sender; return `dm:${other}`; }
function markUnread(event) { const key = eventKey(event); if (key === contextKey() || event.message?.sender.id === state.user.id) return; state.unread[key] = (state.unread[key] || 0) + 1; saveUnread(); loadChannels(); loadUsers(); }
function relevant(event) { return eventKey(event) === contextKey(); }
function appendIfRelevant(event) { if (!relevant(event) || state.messages.some(x => x.id === event.message.id)) return; state.messages.push(event.message); renderMessages(false); const list = $('#message-list'); list.scrollTop = list.scrollHeight; }
function updateIncoming(event) { if (!relevant(event)) return; const m = state.messages.find(x => x.id === event.message_id); if (m) { m.body = event.body; renderMessages(false); } }
function deleteIncoming(event) { if (!relevant(event)) return; state.messages = state.messages.filter(x => x.id !== event.message_id); renderMessages(false); }
function messagesCleared() { state.messages = []; state.unread = {}; saveUnread(); if (state.context) renderMessages(false); if ($('#dialog').open && state.audit) { state.audit.rows = []; state.audit.total = 0; state.audit.page = 1; state.audit.totalPages = 1; renderAudit(); } refreshNav(); }

function sendSignal(payload) {
  if (!state.ws || state.ws.readyState !== WebSocket.OPEN) throw new Error('实时连接尚未就绪');
  state.ws.send(JSON.stringify(payload));
}
function callTarget() { return state.call?.target || (state.context?.type === 'dm' ? state.context : null); }
function showCall(target, video, incoming = false) {
  $('#call-layer').hidden = false; $('#call-name').textContent = target.display_name; $('#call-avatar').textContent = initials(target.display_name);
  $('#call-status').textContent = incoming ? `邀请你进行${video ? '视频' : '语音'}通话` : `正在发起${video ? '视频' : '语音'}通话…`;
  $('#incoming-actions').hidden = !incoming; $('#active-call-actions').hidden = incoming; $('#toggle-camera').hidden = !video;
  $('#local-video').hidden = !video; $('#remote-video').hidden = !video; $('#call-placeholder').hidden = false; $('#call-timer').textContent = '00:00';
}
async function getCallMedia(video) {
  if (!window.isSecureContext || !navigator.mediaDevices?.getUserMedia) throw new Error('语音和视频通话需要通过 HTTPS 打开本站');
  return navigator.mediaDevices.getUserMedia({ audio: { echoCancellation:true, noiseSuppression:true, autoGainControl:true }, video: video ? { facingMode:'user', width:{ideal:1280}, height:{ideal:720} } : false });
}
function createPeer(target) {
  const peer = new RTCPeerConnection({ iceServers: [{ urls: 'stun:stun.cloudflare.com:3478' }] });
  peer.onicecandidate = event => { if (event.candidate && state.call) sendSignal({type:'ice_candidate',to_user_id:target.id,candidate:event.candidate}); };
  peer.ontrack = event => {
    if (!state.call || state.call.peer !== peer) return;
    const remote = $('#remote-video');
    if (!state.call.remoteStream) state.call.remoteStream = new MediaStream();
    if (!state.call.remoteStream.getTracks().some(track => track.id === event.track.id)) state.call.remoteStream.addTrack(event.track);
    if (remote.srcObject !== state.call.remoteStream) remote.srcObject = state.call.remoteStream;
    const playRemote = () => remote.play().catch(() => {
      $('#call-status').textContent = '已连接 · 点击画面播放对方视频';
      remote.onclick = () => remote.play().catch(() => {});
    });
    if (event.track.kind === 'video' && state.call.video) {
      remote.hidden = false; $('#call-placeholder').hidden = true;
      event.track.onunmute = () => { $('#call-status').textContent = '已连接'; playRemote(); };
    }
    playRemote();
  };
  peer.onconnectionstatechange = () => {
    if (!state.call || state.call.peer !== peer) return;
    if (peer.connectionState === 'connected') { $('#call-status').textContent = '已连接'; startCallTimer(); }
    if (['failed','disconnected'].includes(peer.connectionState)) $('#call-status').textContent = '连接不稳定，正在尝试恢复…';
    if (peer.connectionState === 'closed') endCall(false);
  };
  return peer;
}
async function startCall(video) {
  const target = state.context?.type === 'dm' ? state.context : null; if (!target || state.call) return;
  try {
    showCall(target, video); const stream = await getCallMedia(video); const peer = createPeer(target);
    state.call = {target,video,peer,stream,remoteStream:new MediaStream(),pendingCandidates:[],timer:null,startedAt:null};
    $('#local-video').srcObject = stream; stream.getTracks().forEach(track => peer.addTrack(track, stream));
    const offer = await peer.createOffer(); await peer.setLocalDescription(offer);
    sendSignal({type:'call_offer',to_user_id:target.id,sdp:peer.localDescription,video});
  } catch (error) { endCall(false); toast(error.message); }
}
async function handleCallSignal(signal) {
  if (signal.from?.id === state.user.id) return;
  if (signal.type === 'call_offer') {
    if (state.call) { try { sendSignal({type:'call_reject',to_user_id:signal.from.id,reason:'busy'}); } catch {} return; }
    state.call = {target:signal.from,video:!!signal.video,peer:null,stream:null,remoteStream:new MediaStream(),offer:signal.sdp,pendingCandidates:[],timer:null,startedAt:null};
    showCall(signal.from, !!signal.video, true); return;
  }
  if (!state.call || signal.from?.id !== state.call.target.id) return;
  if (signal.type === 'call_answer') { await state.call.peer?.setRemoteDescription(signal.sdp); await flushCandidates(); $('#call-status').textContent = '正在建立安全连接…'; }
  if (signal.type === 'ice_candidate') { if (state.call.peer?.remoteDescription) await state.call.peer.addIceCandidate(signal.candidate).catch(()=>{}); else state.call.pendingCandidates.push(signal.candidate); }
  if (signal.type === 'call_reject') { toast(signal.reason === 'busy' ? '对方正在通话中' : '对方已拒绝通话'); endCall(false); }
  if (signal.type === 'call_end') { toast('通话已结束'); endCall(false); }
}
async function acceptCall() {
  if (!state.call?.offer) return;
  try {
    $('#incoming-actions').hidden = true; $('#active-call-actions').hidden = false; $('#call-status').textContent = '正在连接…';
    const stream = await getCallMedia(state.call.video); const peer = createPeer(state.call.target); state.call.stream = stream; state.call.peer = peer;
    $('#local-video').srcObject = stream; stream.getTracks().forEach(track => peer.addTrack(track, stream));
    await peer.setRemoteDescription(state.call.offer); await flushCandidates(); const answer = await peer.createAnswer(); await peer.setLocalDescription(answer);
    sendSignal({type:'call_answer',to_user_id:state.call.target.id,sdp:peer.localDescription});
  } catch (error) { toast(error.message); rejectCall(); }
}
async function flushCandidates() { if (!state.call?.peer) return; for (const c of state.call.pendingCandidates.splice(0)) await state.call.peer.addIceCandidate(c).catch(()=>{}); }
function rejectCall() { if (state.call) { try { sendSignal({type:'call_reject',to_user_id:state.call.target.id}); } catch {} } endCall(false); }
function endCall(notify = true) {
  const call = state.call; if (call && notify) { try { sendSignal({type:'call_end',to_user_id:call.target.id}); } catch {} }
  if (call?.timer) clearInterval(call.timer); call?.stream?.getTracks().forEach(track => track.stop()); call?.peer?.close();
  state.call = null; $('#call-layer').hidden = true; $('#remote-video').onclick = null; $('#remote-video').srcObject = null; $('#local-video').srcObject = null;
}
function startCallTimer() {
  if (!state.call || state.call.timer) return; state.call.startedAt = Date.now();
  state.call.timer = setInterval(() => { if (!state.call) return; const seconds = Math.floor((Date.now()-state.call.startedAt)/1000); $('#call-timer').textContent = `${String(Math.floor(seconds/60)).padStart(2,'0')}:${String(seconds%60).padStart(2,'0')}`; },1000);
}
function toggleTrack(kind, button) { const track = state.call?.stream?.getTracks().find(t => t.kind === kind); if (!track) return; track.enabled = !track.enabled; button.classList.toggle('off', !track.enabled); button.querySelector('small').textContent = track.enabled ? (kind === 'audio' ? '静音' : '摄像头') : (kind === 'audio' ? '取消静音' : '开启视频'); }
$('#audio-call').onclick = () => startCall(false); $('#video-call').onclick = () => startCall(true); $('#accept-call').onclick = acceptCall; $('#reject-call').onclick = rejectCall; $('#hangup-call').onclick = () => endCall(true);
$('#toggle-mic').onclick = () => toggleTrack('audio',$('#toggle-mic')); $('#toggle-camera').onclick = () => toggleTrack('video',$('#toggle-camera'));

$('#add-channel').onclick = () => showDialog('新建频道', `<label>频道名称<input id="new-channel-name" maxlength="32" placeholder="例如：design"></label><label>说明<input id="new-channel-desc" maxlength="120" placeholder="这个频道讨论什么？"></label><div class="dialog-actions"><button value="cancel">取消</button><button id="create-channel" type="button" class="primary">创建频道</button></div>`, 'NEW CHANNEL', () => { $('#create-channel').onclick = async () => { try { const c = await api('/channels', { method:'POST', body:JSON.stringify({ name:$('#new-channel-name').value, description:$('#new-channel-desc').value }) }); $('#dialog').close(); await loadChannels(); openContext({type:'channel',...c}); } catch (e) { toast(e.message); } }; });
function openSearch() { showDialog('搜索消息', `<label>关键词<input id="search-input" placeholder="至少输入 2 个字符"></label><div id="search-results"></div>`, 'SEARCH', () => { let timer; $('#search-input').oninput = event => { clearTimeout(timer); timer = setTimeout(async () => { const q = event.target.value.trim(); if (q.length < 2) { $('#search-results').innerHTML=''; return; } try { const rows = await api(`/search?q=${encodeURIComponent(q)}`); $('#search-results').innerHTML = rows.map(m => `<div class="dialog-row"><div><p>${escapeHtml(m.body || '附件')}</p><small>${escapeHtml(m.sender.display_name)} · ${formatTime(m.created_at)}</small></div></div>`).join('') || '<p style="color:var(--muted)">没有匹配结果</p>'; } catch(e) { toast(e.message); } }, 250); }; $('#search-input').focus(); }); }
$('#search-button').onclick = openSearch; $('#search-head').onclick = openSearch;

$('#manage-button').onclick = () => showUserManager();
async function showUserManager() { await loadUsers(); const rows = state.users.map(u => `<div class="dialog-row"><div><p>${escapeHtml(u.display_name)} ${u.id === state.user.id ? '（你）' : ''}</p><small>@${escapeHtml(u.username)} · ${u.role === 'admin' ? '管理员' : '成员'}</small></div>${u.id === state.user.id ? '' : `<button class="quiet-button" data-manage="${u.id}">${u.active ? '停用' : '启用'}</button>`}</div>`).join(''); showDialog('用户管理', rows, 'ADMIN', () => document.querySelectorAll('[data-manage]').forEach(button => button.onclick = async () => { const u = state.users.find(item => item.id == button.dataset.manage); try { await api(`/users/${u.id}`, { method:'PATCH', body:JSON.stringify({role:u.role,active:!u.active}) }); await showUserManager(); } catch(e) { toast(e.message); } })); }
function auditFile(m) { if (!m.file_url) return ''; const url = encodeURI(m.file_url); return /\.(png|jpe?g|gif|webp|avif)$/i.test(m.file_url) ? `<a href="${url}" target="_blank" rel="noopener"><img class="audit-image" src="${url}" alt="聊天图片" loading="lazy"></a>` : `<a class="message-file" href="${url}" target="_blank" rel="noopener">打开附件</a>`; }
function renderAudit() {
  const box = $('#audit-results'); if (!box || !state.audit) return;
  const rows = state.audit.rows;
  const content = state.audit.view === 'images'
    ? `<div class="image-gallery">${rows.map(m => { const place = m.scope === 'channel' ? `#${m.channel?.name || '频道'}` : `${m.sender.display_name} → ${m.recipient?.display_name || '用户'}`; return `<button class="gallery-card" type="button" data-gallery-id="${m.id}"><img src="${encodeURI(m.file_url)}" alt="${escapeHtml(m.body || '聊天图片')}" loading="lazy"><span><strong>${escapeHtml(place)}</strong><small>${escapeHtml(m.sender.display_name)} · ${new Date(m.created_at*1000).toLocaleString('zh-CN')}</small></span></button>`; }).join('')}</div>`
    : rows.map(m => { const place = m.scope === 'channel' ? `#${m.channel?.name || '频道'}` : `${m.sender.display_name} → ${m.recipient?.display_name || '用户'}`; return `<article class="audit-entry" data-audit-id="${m.id}"><header><strong>${escapeHtml(place)}</strong><time>${new Date(m.created_at*1000).toLocaleString('zh-CN')}</time></header><small>${escapeHtml(m.sender.display_name)} (@${escapeHtml(m.sender.username)})</small>${m.body ? `<p>${escapeHtml(m.body)}</p>` : ''}${auditFile(m)}</article>`; }).join('');
  const pages = `<nav class="audit-pages"><button type="button" data-audit-page="${state.audit.page-1}" ${state.audit.page <= 1 ? 'disabled' : ''}>上一页</button><label><input id="audit-page-input" type="number" min="1" max="${state.audit.totalPages}" value="${state.audit.page}"> / ${state.audit.totalPages} 页</label><button type="button" data-audit-page="${state.audit.page+1}" ${state.audit.page >= state.audit.totalPages ? 'disabled' : ''}>下一页</button><small>共 ${state.audit.total} 条</small></nav>`;
  box.innerHTML = (content || '<div class="empty-audit">没有符合条件的记录</div>') + pages + '<div id="audit-lightbox" class="audit-lightbox" hidden><button id="close-lightbox" type="button" aria-label="关闭">×</button><img id="lightbox-image" alt="图片预览"><div id="lightbox-meta"></div><button id="locate-message" type="button">查看原记录</button></div>';
  document.querySelectorAll('[data-audit-page]').forEach(button => button.onclick = () => { state.audit.page=+button.dataset.auditPage; loadAudit(); });
  $('#audit-page-input').onchange = event => { state.audit.page=Math.max(1,Math.min(state.audit.totalPages,+event.target.value || 1)); loadAudit(); };
  document.querySelectorAll('[data-gallery-id]').forEach(button => button.onclick = () => openAuditImage(+button.dataset.galleryId));
}
function openAuditImage(id) {
  const m = state.audit.rows.find(row => row.id === id); if (!m) return;
  const place = m.scope === 'channel' ? `#${m.channel?.name || '频道'}` : `${m.sender.display_name} → ${m.recipient?.display_name || '用户'}`;
  $('#lightbox-image').src = m.file_url; $('#lightbox-meta').innerHTML = `<strong>${escapeHtml(place)}</strong><small>${escapeHtml(m.sender.display_name)} · ${new Date(m.created_at*1000).toLocaleString('zh-CN')}</small>${m.body ? `<p>${escapeHtml(m.body)}</p>` : ''}`;
  $('#audit-lightbox').hidden = false; $('#close-lightbox').onclick = () => $('#audit-lightbox').hidden = true; $('#audit-lightbox').onclick = e => { if (e.target === $('#audit-lightbox')) $('#audit-lightbox').hidden = true; }; $('#locate-message').onclick = () => focusAuditMessage(id);
}
function updateAuditTabs() { document.querySelectorAll('[data-audit-view]').forEach(button => button.classList.toggle('active',button.dataset.auditView === state.audit.view)); }
async function focusAuditMessage(id) { state.audit.view='records'; state.audit.scope='all'; state.audit.q=''; $('#audit-scope').value='all'; $('#audit-query').value=''; updateAuditTabs(); await loadAudit(id); requestAnimationFrame(() => document.querySelector(`[data-audit-id="${id}"]`)?.scrollIntoView({behavior:'smooth',block:'center'})); }
async function loadAudit(focus = null) {
  const params = new URLSearchParams({scope:state.audit.scope,kind:state.audit.view === 'images' ? 'images' : 'all',page:state.audit.page}); if (state.audit.q) params.set('q',state.audit.q); if (focus) params.set('focus',focus);
  try { const data = await api(`/admin/messages?${params}`); state.audit.rows=data.items; state.audit.page=data.page; state.audit.total=data.total; state.audit.totalPages=data.total_pages; renderAudit(); if (focus) requestAnimationFrame(() => document.querySelector(`[data-audit-id="${focus}"]`)?.classList.add('focused')); } catch(e) { toast(e.message); }
}
async function showAudit() {
  if (state.user.role !== 'admin') return;
  state.audit = {view:'records',scope:'all',q:'',rows:[],page:1,total:0,totalPages:1};
  showDialog('全部聊天记录', `<p class="audit-warning">仅管理员可查看，内容包含所有成员的频道消息、私聊及图片附件。</p><div class="audit-tabs"><button type="button" class="active" data-audit-view="records">聊天记录</button><button type="button" data-audit-view="images">图片集</button></div><div class="audit-tools"><select id="audit-scope"><option value="all">全部</option><option value="channel">频道</option><option value="dm">私聊</option></select><input id="audit-query" maxlength="100" placeholder="搜索文字"><button id="audit-search" type="button">搜索</button></div><div id="audit-results" class="audit-results"><div class="empty-audit">正在加载…</div></div><div class="audit-footer"><button id="clear-all-messages" class="danger-button" type="button">一键清空全部聊天</button></div>`, 'ADMIN AUDIT', () => { document.querySelectorAll('[data-audit-view]').forEach(button => button.onclick = () => { state.audit.view=button.dataset.auditView; state.audit.page=1; updateAuditTabs(); loadAudit(); }); $('#audit-scope').onchange = e => { state.audit.scope=e.target.value; state.audit.page=1; loadAudit(); }; $('#audit-search').onclick = () => { state.audit.q=$('#audit-query').value.trim(); state.audit.page=1; loadAudit(); }; $('#audit-query').onkeydown = e => { if (e.key==='Enter') { e.preventDefault(); $('#audit-search').click(); } }; $('#clear-all-messages').onclick = clearAllMessages; });
  await loadAudit();
}
async function clearAllMessages() {
  if (state.user.role !== 'admin' || !confirm('确定清空所有频道和私聊记录吗？所有聊天图片与附件也会永久删除，此操作无法撤销。')) return;
  const button = $('#clear-all-messages'); button.disabled = true; button.textContent = '正在清空…';
  try { const result = await api('/admin/messages', {method:'DELETE'}); messagesCleared(); toast(`已清空 ${result.deleted_messages} 条消息和 ${result.deleted_files} 个附件`); }
  catch(e) { button.disabled = false; button.textContent = '一键清空全部聊天'; toast(e.message); }
}
$('#audit-button').onclick = showAudit;
function showProfile() { showDialog('我的账号', `<div class="dialog-row"><div><p>${escapeHtml(state.user.display_name)}</p><small>@${escapeHtml(state.user.username)} · ${state.user.role === 'admin' ? '管理员' : '成员'}</small></div></div><label>显示名称<input id="profile-name" maxlength="40" value="${escapeHtml(state.user.display_name)}"></label><div class="dialog-actions">${state.user.role === 'admin' ? '<button id="open-audit" type="button">全部记录</button><button id="open-admin" type="button">用户管理</button>' : ''}<button id="save-profile" type="button" class="primary">保存资料</button></div>`, 'PROFILE', () => { $('#save-profile').onclick = async () => { try { state.user = await api('/me',{method:'PATCH',body:JSON.stringify({display_name:$('#profile-name').value})}); updateProfileUi(); $('#dialog').close(); await loadUsers(); } catch(e) { toast(e.message); } }; if ($('#open-admin')) $('#open-admin').onclick = showUserManager; if ($('#open-audit')) $('#open-audit').onclick = showAudit; }); }
function showDialog(title, body, eyebrow, ready) { $('#dialog-title').textContent = title; $('#dialog-eyebrow').textContent = eyebrow; $('#dialog-body').innerHTML = body; $('#dialog').showModal(); ready?.(); }
function closeSidebar() { $('#sidebar').classList.remove('open'); }
$('#refresh-users').onclick = loadUsers; $('#logout-button').onclick = async () => { try { await api('/logout', { method:'POST' }); } catch {} clearSession(); location.reload(); };
$('#profile-button').onclick = showProfile; $('#mobile-profile').onclick = showProfile; $('#sidebar-open').onclick = () => $('#sidebar').classList.add('open'); $('#sidebar-close').onclick = closeSidebar; $('#sidebar-backdrop').onclick = closeSidebar;
$('#theme-button').onclick = () => { const next = document.documentElement.dataset.theme === 'dark' ? '' : 'dark'; document.documentElement.dataset.theme = next; localStorage.setItem('lumichat_theme', next); };
$('#emoji-picker').innerHTML = emojis.map(e => `<button type="button">${e}</button>`).join(''); $('#emoji-button').onclick = () => $('#emoji-picker').hidden = !$('#emoji-picker').hidden;
$('#emoji-picker').onclick = e => { if (e.target.tagName !== 'BUTTON') return; const input = $('#message-input'); input.setRangeText(e.target.textContent, input.selectionStart, input.selectionEnd, 'end'); input.focus(); };
document.documentElement.dataset.theme = localStorage.getItem('lumichat_theme') || '';
document.addEventListener('keydown', event => { if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); openSearch(); } if (event.key === 'Escape') { closeSidebar(); if ($('#dialog').open) $('#dialog').close(); } });
if ('serviceWorker' in navigator && location.protocol === 'https:') navigator.serviceWorker.register('/sw.js').catch(() => {});
boot();
