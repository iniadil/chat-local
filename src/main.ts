import { confirm as confirmDialog, open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, bytesToBase64, type Message, type NetEvent, type Peer, type Settings, type Snapshot, type TransferDirection } from "./api";
import { applyStatic, lang, setLang, t, type Lang } from "./i18n";
import { Recorder } from "./recorder";

// ---------------------------------------------------------------- state

const ALL = "all";

interface State {
  me: { id: string; name: string };
  settings: Settings;
  port: number | null;
  addresses: string[];
  dataDir: string;
  startError: string | null;
  peers: Map<string, Peer>;
  messages: Message[];
  /** Names of peers we have ever talked to, for history of offline peers. */
  names: Map<string, string>;
  active: string;
  unread: Map<string, number>;
  transfers: Map<string, { direction: TransferDirection; done: number; total: number }>;
}

const state: State = {
  me: { id: "", name: "" },
  settings: { display_name: "", port: 47800, discovery: true, relay: false, manual_peers: [] },
  port: null,
  addresses: [],
  dataDir: "",
  startError: null,
  peers: new Map(),
  messages: [],
  names: new Map(),
  active: ALL,
  unread: new Map(),
  transfers: new Map(),
};

const recorder = new Recorder();
const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

// ---------------------------------------------------------------- helpers

function initials(name: string) {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  const s = parts.length >= 2 ? parts[0][0] + parts[1][0] : name.slice(0, 2);
  return s.toUpperCase() || "?";
}

const AVATAR_COLORS = ["#2f6b4f", "#8a5a2b", "#3c5f8f", "#7a4e8c", "#a0513d", "#4b6b2f", "#3f7f8a", "#8c6a2b"];
function avatarColor(id: string) {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return AVATAR_COLORS[h % AVATAR_COLORS.length];
}

function avatar(id: string, name: string, small = false) {
  const el = document.createElement("div");
  el.className = "avatar" + (small ? " sm" : "");
  el.style.background = avatarColor(id);
  el.textContent = initials(name);
  return el;
}

function fmtTime(ms: number) {
  return new Date(ms).toLocaleTimeString(lang() === "id" ? "id-ID" : undefined, { hour: "2-digit", minute: "2-digit" });
}

function fmtDay(ms: number) {
  const d = new Date(ms);
  const today = new Date();
  const yesterday = new Date();
  yesterday.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return t().today;
  if (d.toDateString() === yesterday.toDateString()) return t().yesterday;
  return d.toLocaleDateString(lang() === "id" ? "id-ID" : undefined, { weekday: "short", day: "numeric", month: "short", year: "numeric" });
}

function fmtSize(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function fmtDuration(ms: number) {
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

function nameOf(id: string) {
  return state.peers.get(id)?.name ?? state.names.get(id) ?? "?";
}

function conversationOf(m: Message): string {
  if (!m.to) return ALL;
  return m.from === state.me.id ? m.to : m.from;
}

function toast(text: string, error = false) {
  const el = document.createElement("div");
  el.className = "toast" + (error ? " error" : "");
  el.textContent = text;
  $("toasts").appendChild(el);
  setTimeout(() => el.remove(), error ? 6000 : 3000);
}

function errText(e: unknown) {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

// ---------------------------------------------------------------- rendering

function renderMe() {
  const av = $("me-avatar");
  av.textContent = initials(state.me.name);
  av.style.background = avatarColor(state.me.id);
  $("me-name").textContent = state.me.name;
  const status = $("me-status");
  status.innerHTML = "";
  const dot = document.createElement("span");
  dot.className = "dot " + (state.startError ? "err" : state.port ? "on" : "");
  status.appendChild(dot);
  const label = document.createElement("span");
  if (state.startError) label.textContent = t().networkStopped;
  else if (state.port) label.textContent = t().listening(state.port) + (state.settings.relay ? ` · ${t().hub}` : "");
  status.appendChild(label);

  const addr = $("my-addr");
  addr.innerHTML = "";
  if (state.addresses.length && state.port) {
    const primary = document.createElement("b");
    primary.textContent = `${state.addresses[0]}:${state.port}`;
    primary.title = state.addresses.map((a) => `${a}:${state.port}`).join("\n");
    primary.style.cursor = "copy";
    primary.onclick = () => navigator.clipboard?.writeText(primary.textContent ?? "").then(() => toast(t().copied));
    addr.appendChild(primary);
  }
}

function renderSidebar() {
  const list = $("conv-list");
  list.innerHTML = "";
  const online = [...state.peers.values()].sort((a, b) => a.name.localeCompare(b.name));
  const onlineIds = new Set(online.map((p) => p.id));

  const everyone = convButton(ALL, t().everyone, t().everyoneSub(online.length), null);
  list.appendChild(everyone);

  const sec = document.createElement("div");
  sec.className = "conv-section";
  sec.textContent = t().online;
  list.appendChild(sec);
  if (online.length === 0) {
    const e = document.createElement("div");
    e.className = "empty-peers";
    e.textContent = t().noPeersSidebar;
    list.appendChild(e);
  }
  for (const p of online) {
    const meta = p.relay ? t().hub : p.direct ? (p.addr ?? "") : t().viaHub;
    list.appendChild(convButton(p.id, p.name, meta, p));
  }

  // Offline peers we still have history with.
  const history = [...state.names.entries()].filter(([id]) => id !== state.me.id && !onlineIds.has(id));
  if (history.length) {
    const sec2 = document.createElement("div");
    sec2.className = "conv-section";
    sec2.textContent = t().recent;
    list.appendChild(sec2);
    for (const [id, name] of history.sort((a, b) => a[1].localeCompare(b[1]))) {
      const b = convButton(id, name, t().offline, null);
      b.classList.add("offline");
      list.appendChild(b);
    }
  }
}

function convButton(id: string, name: string, meta: string, peer: Peer | null) {
  const b = document.createElement("button");
  b.className = "conv" + (state.active === id ? " active" : "");
  b.dataset.id = id;
  if (id === ALL) {
    const av = document.createElement("div");
    av.className = "avatar";
    av.style.background = "var(--ink-2)";
    av.textContent = "#";
    b.appendChild(av);
  } else {
    b.appendChild(avatar(id, name));
  }
  const text = document.createElement("div");
  text.className = "conv-text";
  const n = document.createElement("div");
  n.className = "conv-name";
  n.textContent = name;
  const m = document.createElement("div");
  m.className = "conv-meta";
  m.textContent = meta;
  text.append(n, m);
  b.appendChild(text);
  if (peer && !peer.direct) {
    const tag = document.createElement("span");
    tag.className = "tag";
    tag.textContent = t().viaHub;
    b.appendChild(tag);
  }
  const unread = state.unread.get(id) ?? 0;
  if (unread > 0) {
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = String(unread);
    b.appendChild(badge);
  }
  b.onclick = () => selectConversation(id);
  return b;
}

function renderHeader() {
  const name = $("chat-name");
  const sub = $("chat-sub");
  const input = $<HTMLTextAreaElement>("input");
  if (state.active === ALL) {
    name.textContent = t().everyone;
    sub.textContent = t().everyoneSub(state.peers.size);
    input.placeholder = t().placeholderAll;
  } else {
    const p = state.peers.get(state.active);
    const n = nameOf(state.active);
    name.textContent = n;
    sub.textContent = p ? (p.direct ? (p.addr ?? t().connected) : t().viaHub) : t().offline;
    input.placeholder = t().placeholderTo(n);
  }
}

function renderMessages() {
  const box = $("messages");
  const stick = box.scrollHeight - box.scrollTop - box.clientHeight < 80;
  box.innerHTML = "";
  const msgs = state.messages.filter((m) => conversationOf(m) === state.active);
  if (msgs.length === 0) {
    const e = document.createElement("div");
    e.className = "empty-chat";
    const h = document.createElement("h3");
    h.textContent = t().emptyTitle;
    const p = document.createElement("p");
    p.textContent = state.active === ALL ? t().emptyAll : t().emptyPeer;
    e.append(h, p);
    box.appendChild(e);
    return;
  }
  let lastDay = "";
  let prev: Message | null = null;
  for (const m of msgs) {
    const day = new Date(m.sent_at).toDateString();
    if (day !== lastDay) {
      const sep = document.createElement("div");
      sep.className = "day-sep";
      sep.textContent = fmtDay(m.sent_at);
      box.appendChild(sep);
      lastDay = day;
      prev = null;
    }
    const cont = !!prev && prev.from === m.from && m.sent_at - prev.sent_at < 3 * 60 * 1000;
    box.appendChild(renderMessage(m, cont));
    prev = m;
  }
  if (stick) box.scrollTop = box.scrollHeight;
}

function renderMessage(m: Message, cont: boolean) {
  const mine = m.from === state.me.id;
  const wrap = document.createElement("div");
  wrap.className = "msg" + (mine ? " mine" : "") + (cont ? " cont" : "");
  wrap.appendChild(avatar(m.from, m.from_name, true));
  const col = document.createElement("div");
  col.style.minWidth = "0";
  if (!mine && state.active === ALL) {
    const s = document.createElement("div");
    s.className = "sender";
    s.textContent = m.from_name;
    col.appendChild(s);
  }
  const bubble = document.createElement("div");
  bubble.className = "bubble";
  const time = document.createElement("div");
  time.className = "time";
  time.textContent = fmtTime(m.sent_at);

  const a = m.attachment;
  if (!a) {
    bubble.textContent = m.text ?? "";
    bubble.appendChild(time);
  } else if (a.kind === "image" && a.path) {
    bubble.classList.add("media");
    const img = document.createElement("img");
    img.className = "photo";
    img.src = api.fileUrl(a.path);
    img.alt = a.file_name;
    img.onclick = () => showLightbox(m);
    img.onerror = () => {
      bubble.classList.remove("media");
      bubble.replaceChildren(fileCard(m), time);
    };
    bubble.append(img, time);
  } else if (a.kind === "audio" && a.path) {
    bubble.append(audioPlayer(m), time);
  } else {
    bubble.append(fileCard(m), time);
  }
  col.appendChild(bubble);
  wrap.appendChild(col);
  return wrap;
}

function fileCard(m: Message) {
  const a = m.attachment!;
  const card = document.createElement("div");
  card.className = "file-card";
  const ext = document.createElement("div");
  ext.className = "file-ext";
  const dot = a.file_name.lastIndexOf(".");
  ext.textContent = dot > 0 ? a.file_name.slice(dot + 1).toUpperCase().slice(0, 5) : "FILE";
  const text = document.createElement("div");
  text.className = "file-text";
  const name = document.createElement("div");
  name.className = "file-name";
  name.textContent = a.file_name;
  name.title = a.file_name;
  const meta = document.createElement("div");
  meta.className = "file-meta";
  const size = document.createElement("span");
  size.textContent = fmtSize(a.size);
  meta.appendChild(size);
  if (a.path) {
    const openB = document.createElement("button");
    openB.textContent = t().openFile;
    openB.onclick = () => api.openAttachment(m.id).catch((e) => toast(errText(e), true));
    const revealB = document.createElement("button");
    revealB.textContent = t().showInFolder;
    revealB.onclick = () => api.revealAttachment(m.id).catch((e) => toast(errText(e), true));
    meta.append(openB, revealB);
  }
  text.append(name, meta);
  card.append(ext, text);
  return card;
}

let playing: HTMLAudioElement | null = null;

function audioPlayer(m: Message) {
  const a = m.attachment!;
  const root = document.createElement("div");
  root.className = "audio";
  const audio = new Audio(api.fileUrl(a.path!));
  audio.preload = "metadata";
  const play = document.createElement("button");
  play.className = "play";
  const ICON_PLAY = '<svg viewBox="0 0 24 24" width="18" height="18"><path fill="currentColor" d="M8 5v14l11-7z"/></svg>';
  const ICON_PAUSE = '<svg viewBox="0 0 24 24" width="18" height="18"><path fill="currentColor" d="M6 5h4v14H6zm8 0h4v14h-4z"/></svg>';
  play.innerHTML = ICON_PLAY;
  const bar = document.createElement("div");
  bar.className = "bar";
  const fill = document.createElement("i");
  bar.appendChild(fill);
  const dur = document.createElement("span");
  dur.className = "dur";
  const total = () => (isFinite(audio.duration) && audio.duration > 0 ? audio.duration * 1000 : (a.duration_ms ?? 0));
  dur.textContent = fmtDuration(total());
  audio.onloadedmetadata = () => (dur.textContent = fmtDuration(total()));
  audio.ontimeupdate = () => {
    const tt = total();
    fill.style.width = tt ? `${(audio.currentTime * 1000 / tt) * 100}%` : "0";
    dur.textContent = fmtDuration(audio.currentTime * 1000);
  };
  audio.onended = () => {
    play.innerHTML = ICON_PLAY;
    fill.style.width = "0";
    dur.textContent = fmtDuration(total());
    playing = null;
  };
  audio.onpause = () => (play.innerHTML = ICON_PLAY);
  audio.onplay = () => (play.innerHTML = ICON_PAUSE);
  play.onclick = () => {
    if (audio.paused) {
      if (playing && playing !== audio) playing.pause();
      playing = audio;
      audio.play().catch((e) => toast(errText(e), true));
    } else {
      audio.pause();
    }
  };
  bar.onclick = (e) => {
    const r = bar.getBoundingClientRect();
    const tt = total();
    if (tt) audio.currentTime = ((e.clientX - r.left) / r.width) * (tt / 1000);
  };
  root.append(play, bar, dur);
  return root;
}

function showLightbox(m: Message) {
  const lb = $("lightbox");
  $<HTMLImageElement>("lightbox-img").src = api.fileUrl(m.attachment!.path!);
  $("lightbox-open").onclick = () => api.openAttachment(m.id).catch((e) => toast(errText(e), true));
  lb.hidden = false;
}

function renderAll() {
  renderMe();
  renderSidebar();
  renderHeader();
  renderMessages();
  updateTitle();
}

function updateTitle() {
  const total = [...state.unread.values()].reduce((a, b) => a + b, 0);
  const title = total ? `(${total}) ChatLocal` : "ChatLocal";
  document.title = title;
  getCurrentWindow().setTitle(title).catch(() => undefined);
}

// ---------------------------------------------------------------- actions

function selectConversation(id: string) {
  state.active = id;
  state.unread.delete(id);
  renderSidebar();
  renderHeader();
  renderMessages();
  updateTitle();
  $("input").focus();
}

function target(): string | null {
  return state.active === ALL ? null : state.active;
}

function addMessage(m: Message, incoming: boolean) {
  state.messages.push(m);
  state.names.set(m.from, m.from_name);
  const conv = conversationOf(m);
  if (incoming && conv !== state.active) {
    state.unread.set(conv, (state.unread.get(conv) ?? 0) + 1);
  }
  if (conv === state.active) renderMessages();
  renderSidebar();
  updateTitle();
}

async function sendCurrentText() {
  const input = $<HTMLTextAreaElement>("input");
  const text = input.value.trim();
  if (!text) return;
  input.value = "";
  autosize(input);
  try {
    const m = await api.sendText(target(), text);
    addMessage(m, false);
  } catch (e) {
    input.value = text;
    toast(friendlyError(e), true);
  }
}

function friendlyError(e: unknown) {
  const s = errText(e);
  if (s.includes("not reachable")) return t().unreachable;
  if (s.includes("too large") || s.includes("larger than")) return t().tooLarge;
  return s;
}

async function sendPaths(paths: string[]) {
  if (!paths.length) return;
  try {
    const sent = await api.sendFiles(target(), paths);
    sent.forEach((m) => addMessage(m, false));
  } catch (e) {
    toast(friendlyError(e), true);
  }
}

async function pickAndSend() {
  const picked = await openDialog({ multiple: true, directory: false });
  if (!picked) return;
  await sendPaths(Array.isArray(picked) ? picked : [picked]);
}

async function sendBlob(bytes: Uint8Array, fileName: string, mime: string, durationMs?: number) {
  try {
    const data = await bytesToBase64(bytes);
    const m = await api.sendBlob(target(), { file_name: fileName, mime, data, duration_ms: durationMs });
    addMessage(m, false);
  } catch (e) {
    toast(friendlyError(e), true);
  }
}

let recTimer: number | undefined;

async function startRecording() {
  try {
    await recorder.start(() => void finishRecording(true));
  } catch (e) {
    toast(t().micDenied + " " + errText(e), true);
    return;
  }
  $("recording").hidden = false;
  $("composer-row").hidden = true;
  $("btn-mic").classList.add("live");
  const tick = () => ($("rec-time").textContent = fmtDuration(recorder.elapsedMs()));
  tick();
  recTimer = window.setInterval(tick, 250);
}

async function finishRecording(keep: boolean) {
  window.clearInterval(recTimer);
  $("recording").hidden = true;
  $("composer-row").hidden = false;
  $("btn-mic").classList.remove("live");
  const result = await recorder.stop(keep);
  if (!result) return;
  if (result.durationMs < 500) return;
  const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
  await sendBlob(result.wav, `voice-${stamp}.wav`, "audio/wav", result.durationMs);
}

function autosize(el: HTMLTextAreaElement) {
  el.style.height = "auto";
  el.style.height = Math.min(el.scrollHeight, 160) + "px";
}

// ---------------------------------------------------------------- settings

function openSettings() {
  const f = $<HTMLFormElement>("settings-form");
  const s = state.settings;
  (f.elements.namedItem("display_name") as HTMLInputElement).value = s.display_name || state.me.name;
  (f.elements.namedItem("lang") as HTMLSelectElement).value = lang();
  (f.elements.namedItem("discovery") as HTMLInputElement).checked = s.discovery;
  (f.elements.namedItem("port") as HTMLInputElement).value = String(s.port);
  (f.elements.namedItem("manual_peers") as HTMLTextAreaElement).value = s.manual_peers.join("\n");
  (f.elements.namedItem("relay") as HTMLInputElement).checked = s.relay;
  $("settings-addrs").textContent = state.addresses.map((a) => `${a}:${state.port ?? s.port}`).join("  ") || "–";
  $("settings-datadir").textContent = state.dataDir;
  $("settings-modal").hidden = false;
}

async function submitSettings(ev: Event) {
  ev.preventDefault();
  const f = $<HTMLFormElement>("settings-form");
  const newLang = (f.elements.namedItem("lang") as HTMLSelectElement).value as Lang;
  if (newLang !== lang()) {
    setLang(newLang);
    applyStatic();
  }
  const settings: Settings = {
    display_name: (f.elements.namedItem("display_name") as HTMLInputElement).value.trim(),
    discovery: (f.elements.namedItem("discovery") as HTMLInputElement).checked,
    port: Number((f.elements.namedItem("port") as HTMLInputElement).value) || 47800,
    manual_peers: (f.elements.namedItem("manual_peers") as HTMLTextAreaElement).value
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean),
    relay: (f.elements.namedItem("relay") as HTMLInputElement).checked,
  };
  const btn = f.querySelector<HTMLButtonElement>("button[type=submit]")!;
  btn.disabled = true;
  try {
    const snap = await api.saveSettings(settings);
    applySnapshot(snap);
    $("settings-modal").hidden = true;
    toast(t().saved);
  } catch (e) {
    toast(errText(e), true);
  } finally {
    btn.disabled = false;
  }
}

async function submitPeer(ev: Event) {
  ev.preventDefault();
  const f = $<HTMLFormElement>("peer-form");
  const target = (f.elements.namedItem("target") as HTMLInputElement).value.trim();
  const remember = (f.elements.namedItem("remember") as HTMLInputElement).checked;
  if (!target) return;
  const btn = f.querySelector<HTMLButtonElement>("button[type=submit]")!;
  btn.disabled = true;
  try {
    await api.connectPeer(target);
    toast(t().connected);
    if (remember && !state.settings.manual_peers.includes(target)) {
      const snap = await api.saveSettings({ ...state.settings, manual_peers: [...state.settings.manual_peers, target] });
      applySnapshot(snap);
    }
    $("peer-modal").hidden = true;
    (f.elements.namedItem("target") as HTMLInputElement).value = "";
  } catch (e) {
    toast(`${t().connectFailed}: ${errText(e)}`, true);
  } finally {
    btn.disabled = false;
  }
}

function applySnapshot(snap: Snapshot) {
  state.me = snap.me;
  state.settings = snap.settings;
  state.port = snap.port;
  state.addresses = snap.addresses;
  state.dataDir = snap.data_dir;
  state.startError = snap.start_error;
  state.peers = new Map(snap.peers.map((p) => [p.id, p]));
  state.messages = snap.messages;
  state.names = new Map();
  for (const m of snap.messages) {
    state.names.set(m.from, m.from_name);
  }
  if (state.active !== ALL && !state.peers.has(state.active) && !state.names.has(state.active)) state.active = ALL;
  renderAll();
  if (snap.start_error) toast(snap.start_error, true);
}

function renderTransfers() {
  const box = $("transfers");
  box.replaceChildren();
  for (const [key, tr] of state.transfers) {
    const id = key.slice(0, key.indexOf(":"));
    const pct = tr.total > 0 ? Math.floor((tr.done / tr.total) * 100) : 0;
    const row = document.createElement("div");
    row.className = "transfer";
    const label = document.createElement("div");
    if (tr.direction === "sending") {
      const name = state.messages.find((m) => m.id === id)?.attachment?.file_name ?? "";
      label.textContent = t().sendingFile(name, pct);
    } else {
      label.textContent = t().receivingFile(pct);
    }
    const bar = document.createElement("div");
    bar.className = "bar";
    const fill = document.createElement("i");
    fill.style.width = pct + "%";
    bar.appendChild(fill);
    row.append(label, bar);
    box.appendChild(row);
  }
  box.hidden = state.transfers.size === 0;
}

// ---------------------------------------------------------------- events

function onNetEvent(e: NetEvent) {
  switch (e.type) {
    case "ready":
      state.port = e.port;
      state.addresses = e.addresses;
      state.startError = null;
      renderMe();
      renderHeader();
      break;
    case "peer_online":
      state.peers.set(e.peer.id, e.peer);
      state.names.set(e.peer.id, e.peer.name);
      renderSidebar();
      renderHeader();
      if (state.active === ALL && state.messages.every((m) => m.to)) renderMessages();
      break;
    case "peer_offline":
      state.peers.delete(e.id);
      renderSidebar();
      renderHeader();
      break;
    case "message":
      addMessage(e.message, true);
      break;
    case "transfer": {
      const key = e.message_id + ":" + e.peer;
      if (e.done >= e.total) state.transfers.delete(key);
      else state.transfers.set(key, { direction: e.direction, done: e.done, total: e.total });
      renderTransfers();
      break;
    }
    case "transfer_failed":
      state.transfers.delete(e.message_id + ":" + e.peer);
      renderTransfers();
      toast(t().transferFailed(e.reason), true);
      break;
    case "error":
      toast(e.message, true);
      break;
  }
}

async function main() {
  setLang(lang());
  applyStatic();

  const input = $<HTMLTextAreaElement>("input");
  input.addEventListener("input", () => autosize(input));
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void sendCurrentText();
    }
  });
  input.addEventListener("paste", (e) => {
    const items = e.clipboardData?.items;
    if (!items) return;
    for (const item of items) {
      if (item.kind === "file" && item.type.startsWith("image/")) {
        const file = item.getAsFile();
        if (!file) continue;
        e.preventDefault();
        file.arrayBuffer().then((buf) => {
          const ext = item.type.split("/")[1] || "png";
          void sendBlob(new Uint8Array(buf), `pasted-${Date.now()}.${ext}`, item.type);
        });
      }
    }
  });
  $("btn-send").onclick = () => void sendCurrentText();
  $("btn-attach").onclick = () => void pickAndSend();
  $("btn-mic").onclick = () => (recorder.running ? void finishRecording(true) : void startRecording());
  $("btn-rec-cancel").onclick = () => void finishRecording(false);
  $("btn-rec-send").onclick = () => void finishRecording(true);

  $("btn-settings").onclick = openSettings;
  $("btn-settings-close").onclick = () => ($("settings-modal").hidden = true);
  $("settings-form").addEventListener("submit", (e) => void submitSettings(e));
  $("btn-clear-history").onclick = async () => {
    const yes = await confirmDialog(t().clearHistoryConfirm, { title: "ChatLocal", kind: "warning" });
    if (!yes) return;
    try {
      await api.clearHistory();
      state.messages = [];
      state.unread.clear();
      renderAll();
      $("settings-modal").hidden = true;
    } catch (e) {
      toast(errText(e), true);
    }
  };
  $("btn-add-peer").onclick = () => {
    $("peer-modal").hidden = false;
    (document.querySelector("#peer-form input[name=target]") as HTMLInputElement).focus();
  };
  $("btn-peer-close").onclick = () => ($("peer-modal").hidden = true);
  $("peer-form").addEventListener("submit", (e) => void submitPeer(e));
  $("lightbox-close").onclick = () => ($("lightbox").hidden = true);
  $("lightbox").addEventListener("click", (e) => {
    if (e.target === e.currentTarget) $("lightbox").hidden = true;
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      $("lightbox").hidden = true;
      $("settings-modal").hidden = true;
      $("peer-modal").hidden = true;
    }
  });
  for (const m of ["settings-modal", "peer-modal"]) {
    $(m).addEventListener("click", (e) => {
      if (e.target === e.currentTarget) $(m).hidden = true;
    });
  }

  // Files dragged from the desktop onto the window.
  await getCurrentWebview().onDragDropEvent((ev) => {
    const overlay = $("drop-overlay");
    if (ev.payload.type === "enter" || ev.payload.type === "over") overlay.hidden = false;
    else if (ev.payload.type === "leave") overlay.hidden = true;
    else if (ev.payload.type === "drop") {
      overlay.hidden = true;
      void sendPaths(ev.payload.paths);
    }
  });

  await api.onNetEvent(onNetEvent);
  applySnapshot(await api.snapshot());
  // The node may finish starting after the first snapshot; refresh once.
  setTimeout(async () => applySnapshot(await api.snapshot()), 1500);
  input.focus();
}

main().catch((e) => toast(errText(e), true));
