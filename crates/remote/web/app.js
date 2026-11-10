// Scrawl tablet client: vanilla JS, no dependencies, no third-party requests.
'use strict';
(() => {
  const $ = (id) => document.getElementById(id);
  const store = { get: (k) => { try { return localStorage.getItem(k); } catch (_) { return null; } },
                  set: (k, v) => { try { localStorage.setItem(k, v); } catch (_) {} },
                  del: (k) => { try { localStorage.removeItem(k); } catch (_) {} } };
  let ws = null, seq = 0, lastAcked = 0, outbox = [], authed = false, strokeId = 0;
  let pen = 'pen', drawing = false, penOnly = false, backoff = 500;

  async function sha256hex(s) {
    const b = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(s));
    return [...new Uint8Array(b)].map((x) => x.toString(16).padStart(2, '0')).join('');
  }
  async function confirmationCode(token, nonce) {
    const h = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode('scrawl-pair|' + token + '|' + nonce)));
    const n = ((h[0] << 24 | h[1] << 16 | h[2] << 8 | h[3]) >>> 0) % 1000000;
    return String(n).padStart(6, '0');
  }
  function randomNonce() {
    const b = new Uint8Array(12); crypto.getRandomValues(b);
    return [...b].map((x) => x.toString(16).padStart(2, '0')).join('');
  }

  // ---- connection -----------------------------------------------------------------
  function send(obj) { if (ws && ws.readyState === 1) ws.send(JSON.stringify(obj)); }
  function sendSeq(obj) {            // reliable messages: remembered until acked, resent after reconnect
    obj.seq = ++seq; outbox.push(obj); send(obj);
  }
  function resend() { outbox = outbox.filter((m) => m.seq > lastAcked); outbox.forEach(send); }

  function connect(first) {
    ws = new WebSocket('wss://' + location.host + '/ws');
    ws.onopen = async () => {
      const dev = store.get('device'), sec = store.get('secret');
      if (dev && sec) { send({ t: 'auth', device: dev, secret: sec }); }
      else {
        const hash = location.hash.slice(1);
        if (!/^[0-9a-f]{32}$/i.test(hash)) { $('pair-msg').textContent = 'Open the pairing link from your computer (scan its QR code).'; return; }
        const nonce = randomNonce();
        history.replaceState(null, '', location.pathname);       // never leave the token in the URL
        $('pair-msg').textContent = 'Waiting for approval on your computer…';
        $('code').textContent = await confirmationCode(hash, nonce); $('code').hidden = false; $('pair-help').hidden = false;
        send({ t: 'pair', token: hash, nonce, name: (navigator.userAgentData?.platform || navigator.platform || 'Tablet').slice(0, 30) });
      }
    };
    ws.onmessage = (ev) => { let m; try { m = JSON.parse(ev.data); } catch (_) { return; } onServer(m); };
    ws.onclose = () => { authed = false; setStatus('reconnecting…');
      if (store.get('device')) setTimeout(() => connect(false), backoff = Math.min(backoff * 2, 5000)); };
  }

  function onServer(m) {
    switch (m.t) {
      case 'paired': store.set('device', m.device); store.set('secret', m.secret); showMain(); send({ t: 'auth', device: m.device, secret: m.secret }); break;
      case 'auth_ok': store.set('secret', m.secret); authed = true; backoff = 500; showMain(); setStatus('connected');
        lastAcked = Math.max(lastAcked, 0); send({ t: 'resume', seq }); resend(); break;
      case 'ack': lastAcked = Math.max(lastAcked, m.seq); outbox = outbox.filter((x) => x.seq > lastAcked); break;
      case 'candidates': renderStrip(m.labels); break;
      case 'state': drawing = m.drawing; $('mode').textContent = drawing ? 'Draw' : 'Math'; $('undo').disabled = !m.can_undo; $('redo').disabled = !m.can_redo; $('expr').textContent = m.expr || ''; break;
      case 'err': setStatus(m.msg); if (/auth|device|pair/i.test(m.msg)) { store.del('device'); store.del('secret'); } break;
      default: break;
    }
  }
  function showMain() { $('pair').hidden = true; $('main').hidden = false; sizeCanvas(); }
  function setStatus(s) { $('status').textContent = s; }

  // ---- UI -------------------------------------------------------------------------
  function renderStrip(labels) {
    const strip = $('strip'); strip.textContent = '';
    labels.slice(0, 5).forEach((l, i) => {
      const b = document.createElement('button'); b.textContent = l;
      const s = document.createElement('small'); s.textContent = String(i + 1); b.appendChild(s);
      b.onclick = () => sendSeq({ t: 'cmd', cmd: { c: 'choose', i } }); strip.appendChild(b);
    });
  }
  const cmd = (c) => sendSeq({ t: 'cmd', cmd: c });
  $('undo').onclick = () => cmd({ c: 'undo' });
  $('redo').onclick = () => cmd({ c: 'redo' });
  $('recog').onclick = () => cmd({ c: 'recognize_now' });
  $('mode').onclick = () => cmd({ c: 'mode', drawing: !drawing });
  function setPen(p, id) { pen = p; ['pen', 'hl', 'er'].forEach((x) => $(x).classList.toggle('on', x === id)); cmd({ c: 'tool', pen: p }); }
  $('pen').onclick = () => setPen('pen', 'pen'); $('hl').onclick = () => setPen('highlighter', 'hl'); $('er').onclick = () => setPen('eraser', 'er');
  $('color').oninput = (e) => cmd({ c: 'color', rgb: parseInt(e.target.value.slice(1), 16) });
  $('penonly').onchange = (e) => { penOnly = e.target.checked; };
  $('stop').onclick = () => { store.del('device'); store.del('secret'); if (ws) ws.close(); location.reload(); };
  document.querySelectorAll('#templates button').forEach((b) => {
    b.onclick = () => { const k = b.dataset.t; cmd({ c: 'template', t: k === 'matrix' ? { k, rows: 2, cols: 2 } : { k } }); };
  });
  $('text').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { const s = $('text').value; if (s) sendSeq({ t: 'text', s }); $('text').value = ''; cmd({ c: 'nav', key: 'enter' }); e.preventDefault(); }
    else if (e.key.length === 1 && !e.ctrlKey && !e.metaKey) { sendSeq({ t: 'text', s: e.key }); e.preventDefault(); }   // live per-key typing mixes with drawing
    else if (e.key === 'Tab') { cmd({ c: 'nav', key: e.shiftKey ? 'shift_tab' : 'tab' }); e.preventDefault(); }
    else if (e.key === 'Backspace') { cmd({ c: 'nav', key: 'backspace' }); e.preventDefault(); }
    else if (e.key === 'ArrowLeft') { cmd({ c: 'nav', key: 'left' }); e.preventDefault(); }
    else if (e.key === 'ArrowRight') { cmd({ c: 'nav', key: 'right' }); e.preventDefault(); }
  });
  addEventListener('keydown', (e) => { if (document.activeElement === $('text')) return; if (/^[1-5]$/.test(e.key)) cmd({ c: 'choose', i: +e.key - 1 }); });

  // ---- ink capture ----------------------------------------------------------------
  const pad = $('pad'), ctx = pad.getContext('2d');
  function sizeCanvas() { const r = pad.getBoundingClientRect(); pad.width = r.width * devicePixelRatio; pad.height = r.height * devicePixelRatio; ctx.scale(devicePixelRatio, devicePixelRatio); }
  addEventListener('resize', sizeCanvas);
  ['gesturestart', 'gesturechange', 'contextmenu', 'selectstart'].forEach((t) => pad.addEventListener(t, (e) => e.preventDefault()));
  let cur = null, batch = [], lastT = 0, flushTimer = 0;
  const colorHex = () => $('color').value;

  function point(e, rect) {
    const t = performance.now(); const dt = lastT ? Math.min(10000, t - lastT) : 0; lastT = t;
    return [e.clientX - rect.left, e.clientY - rect.top, e.pressure > 0 ? e.pressure : 0.5, e.tiltX || 0, e.tiltY || 0, dt];
  }
  function flush(done) {
    if (!cur) return;
    while (batch.length) {                                                  // server limit: 256 points per message
      const part = batch.splice(0, 200);
      sendSeq({ t: 'strokes', id: cur.id, pen, width: cur.width, pts: part, done: done && batch.length === 0 });
    }
  }
  pad.addEventListener('pointerdown', (e) => {
    if (penOnly && e.pointerType === 'touch') return;                      // palm rejection
    e.preventDefault(); pad.setPointerCapture(e.pointerId);
    const rect = pad.getBoundingClientRect(); lastT = 0;
    cur = { id: ++strokeId, width: pen === 'highlighter' ? 6 : 3, pid: e.pointerId, last: null };
    batch = [point(e, rect)]; cur.last = [e.clientX, e.clientY];
    ctx.strokeStyle = colorHex(); ctx.lineWidth = cur.width; ctx.lineCap = 'round'; ctx.lineJoin = 'round';
    ctx.globalAlpha = pen === 'highlighter' ? 0.4 : 1; ctx.beginPath(); ctx.moveTo(e.clientX - rect.left, e.clientY - rect.top);
  });
  pad.addEventListener('pointermove', (e) => {
    if (!cur || e.pointerId !== cur.pid) return; e.preventDefault();
    const rect = pad.getBoundingClientRect();
    const evs = e.getCoalescedEvents ? e.getCoalescedEvents() : [e];
    for (const c of (evs.length ? evs : [e])) { batch.push(point(c, rect)); ctx.lineTo(c.clientX - rect.left, c.clientY - rect.top); }
    if (e.getPredictedEvents) for (const p of e.getPredictedEvents().slice(0, 1)) ctx.lineTo(p.clientX - rect.left, p.clientY - rect.top); // local preview only
    ctx.stroke(); ctx.beginPath(); const l = evs[evs.length - 1] || e; ctx.moveTo(l.clientX - rect.left, l.clientY - rect.top);
    if (batch.length >= 12) flush(false);                                  // ~every few ms of ink: low latency
    else if (!flushTimer) flushTimer = setTimeout(() => { flushTimer = 0; flush(false); }, 8);
  });
  function end(e) {
    if (!cur || (e && e.pointerId !== cur.pid)) return;
    clearTimeout(flushTimer); flushTimer = 0;
    if (batch.length === 0) batch.push([cur.last[0], cur.last[1], 0.5, 0, 0, 0]);
    flush(true); cur = null; ctx.globalAlpha = 1; setTimeout(() => ctx.clearRect(0, 0, pad.width, pad.height), 400); // the PC owns the canonical ink
  }
  pad.addEventListener('pointerup', end); pad.addEventListener('pointercancel', end);

  // ---- keep screen awake ----------------------------------------------------------
  let wake = null;
  async function keepAwake() { try { if ('wakeLock' in navigator) wake = await navigator.wakeLock.request('screen'); } catch (_) {} }
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') keepAwake(); });
  keepAwake();

  connect(true);
})();
