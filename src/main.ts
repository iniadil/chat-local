import { confirm as confirmDialog, open as openDialog } from "@tauri-apps/plugin-dialog";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api, bytesToBase64, type FoundPeer, type Message, type NetEvent, type Peer, type Settings, type Snapshot } from "./api";
import { applyStatic, lang, setLang, t, type Lang } from "./i18n";
import { Recorder, listMics, preferredMic, setPreferredMic } from "./recorder";

// ---------------------------------------------------------------- state

const ALL = "all";

interface State {
  me: { id: string; name: string };
  settings: Settings;
  port: number | null;
  addresses: string[];
  dataDir: string;
  startError: string | null;
  firstRun: boolean;
  peers: Map<string, Peer>;
  messages: Message[];
  /** Names of peers we have ever talked to, for history of offline peers. */
  names: Map<string, string>;
  active: string;
  unread: Map<string, number>;
  /** Attachments coming in, keyed by `message_id:peer`. */
  transfers: Map<string, { done: number; total: number }>;
  /** Attachments we sent this session, per recipient connection. */
  outgoing: Map<string, Map<string, Outgoing>>;
}

interface Outgoing {
  done: number;
  total: number;
  /** Why the transfer failed; absent while running or once complete. */
  error?: string;
}

const state: State = {
  me: { id: "", name: "" },
  settings: { display_name: "", port: 47800, discovery: true, relay: false, manual_peers: [] },
  port: null,
  addresses: [],
  dataDir: "",
  startError: null,
  firstRun: false,
  peers: new Map(),
  messages: [],
  names: new Map(),
  active: ALL,
  unread: new Map(),
  transfers: new Map(),
  outgoing: new Map(),
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

/** Upper-case extension for a file icon, or "FILE" when the name has none. */
function extLabel(fileName: string) {
  const dot = fileName.lastIndexOf(".");
  return dot > 0 ? fileName.slice(dot + 1).toUpperCase().slice(0, 5) : "FILE";
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
  wrap.dataset.id = m.id;
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
  if (mine) applyStatus(wrap, m);
  return wrap;
}

// ---------- send status ----------
// Like a phone messenger: a clock while an attachment is on its way, a tick
// once it is sent, and a retry link when it did not get through.

type SendStatus = { kind: "sending"; pct: number } | { kind: "sent" } | { kind: "failed"; peers: string[]; reason: string };

function sendStatus(m: Message): SendStatus | null {
  // Text either leaves at once or the send call fails.
  if (!m.attachment) return { kind: "sent" };
  const entries = state.outgoing.get(m.id);
  // Sent in an earlier session (or to nobody): nothing is known.
  if (!entries || entries.size === 0) return null;
  const failed = [...entries].filter(([, o]) => o.error);
  if (failed.length) return { kind: "failed", peers: failed.map(([p]) => p), reason: failed[0][1].error! };
  let done = 0;
  let total = 0;
  for (const o of entries.values()) {
    done += o.done;
    total += o.total;
  }
  if (done < total) return { kind: "sending", pct: Math.floor((done / total) * 100) };
  return { kind: "sent" };
}

const ICON_CLOCK = '<svg viewBox="0 0 16 16" width="11" height="11"><circle cx="8" cy="8" r="6.5" fill="none" stroke="currentColor" stroke-width="1.5"/><path d="M8 4.5V8l2.5 1.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>';
const ICON_TICK = '<svg viewBox="0 0 16 16" width="12" height="12"><path d="M3 8.5l3 3 7-7" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>';

/** (Re)draw the status of one of our messages: the mark next to its time
 * and, after a failure, the retry line under the bubble. */
function applyStatus(wrap: HTMLElement, m: Message) {
  wrap.querySelector(".tick")?.remove();
  wrap.querySelector(".send-failed")?.remove();
  const st = sendStatus(m);
  if (!st) return;
  const tick = document.createElement("span");
  tick.className = "tick " + st.kind;
  if (st.kind === "sending") {
    tick.innerHTML = ICON_CLOCK;
    tick.append(` ${st.pct}%`);
    tick.title = t().sending;
  } else if (st.kind === "sent") {
    tick.innerHTML = ICON_TICK;
    tick.title = t().sent;
  } else {
    tick.textContent = "!";
    tick.title = st.reason;
  }
  wrap.querySelector(".time")?.appendChild(tick);
  if (st.kind !== "failed") return;
  const row = document.createElement("div");
  row.className = "send-failed";
  const label = document.createElement("span");
  label.textContent = t().sendFailed;
  label.title = st.reason;
  const retry = document.createElement("button");
  retry.type = "button";
  retry.textContent = t().retry;
  retry.onclick = async () => {
    retry.disabled = true;
    // The new attempt reports under the connection it takes, which is a
    // relay's when the peer is not directly connected, so the old failures
    // must go first or they would outlive a delivery. Peers it cannot reach
    // come back as fresh failures.
    const out = outgoingOf(m.id);
    const old = st.peers.map((p) => [p, out.get(p)] as const);
    for (const p of st.peers) out.delete(p);
    try {
      await api.resendAttachment(m.id, st.peers);
    } catch (e) {
      for (const [p, o] of old) if (o && !out.has(p)) out.set(p, o);
      updateStatus(m.id);
      toast(friendlyError(e), true);
    }
  };
  row.append(label, retry);
  wrap.querySelector(".bubble")?.parentElement?.appendChild(row);
}

function updateStatus(id: string) {
  const m = state.messages.find((x) => x.id === id);
  const wrap = $("messages").querySelector<HTMLElement>(`.msg[data-id="${CSS.escape(id)}"]`);
  if (m && wrap) applyStatus(wrap, m);
}

function fileCard(m: Message) {
  const a = m.attachment!;
  const card = document.createElement("div");
  card.className = "file-card";
  const ext = document.createElement("div");
  ext.className = "file-ext";
  ext.textContent = extLabel(a.file_name);
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
  // Clear the box before any await: a second Enter while attachments are
  // uploading must not send the same text twice or wipe newly typed text.
  input.value = "";
  autosize(input);
  if (pending.length) await sendPending();
  if (!text) return;
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

// ---------- pending attachments ----------
// Files dropped, picked or pasted wait here until the user presses Send, so
// nothing leaves the machine by accident.

type Pending =
  | { kind: "path"; key: string; path: string; file_name: string; size: number; mime: string; preview?: string | null }
  | { kind: "blob"; key: string; bytes: Uint8Array; file_name: string; size: number; mime: string; preview?: string | null };

const pending: Pending[] = [];
let pendingSeq = 0;

/** Free the object URL behind a pasted image's thumbnail. */
function releasePreview(p: Pending) {
  if (p.kind === "blob" && p.preview) URL.revokeObjectURL(p.preview);
}

function renderPending() {
  const root = $("pending");
  const list = $("pending-list");
  root.hidden = pending.length === 0;
  list.replaceChildren(
    ...pending.map((p) => {
      const chip = document.createElement("div");
      chip.className = "chip";
      if (p.preview) {
        const img = document.createElement("img");
        img.src = p.preview;
        img.alt = p.file_name;
        chip.appendChild(img);
      } else {
        const ico = document.createElement("div");
        ico.className = "file-ico";
        ico.textContent = extLabel(p.file_name);
        chip.appendChild(ico);
      }
      const meta = document.createElement("div");
      meta.className = "meta";
      const name = document.createElement("div");
      name.className = "name";
      name.textContent = p.file_name;
      name.title = p.file_name;
      const size = document.createElement("div");
      size.className = "size";
      size.textContent = fmtSize(p.size);
      meta.append(name, size);
      const rm = document.createElement("button");
      rm.className = "remove";
      rm.type = "button";
      rm.textContent = "✕";
      rm.title = t().removeAttachment;
      rm.setAttribute("aria-label", t().removeAttachment);
      rm.onclick = () => {
        const i = pending.findIndex((q) => q.key === p.key);
        if (i >= 0) pending.splice(i, 1);
        releasePreview(p);
        renderPending();
      };
      chip.append(meta, rm);
      return chip;
    }),
  );
  $("pending-hint").textContent = t().pendingHint(pending.length);
}

async function queuePaths(paths: string[]) {
  for (const path of paths) {
    const queued = () => pending.some((p) => p.kind === "path" && p.path === path);
    if (queued()) continue;
    try {
      const info = await api.fileInfo(path);
      // The same path may have been dropped again while we waited.
      if (!queued()) pending.push({ kind: "path", key: `p${pendingSeq++}`, ...info });
    } catch (e) {
      toast(friendlyError(e), true);
    }
  }
  renderPending();
  $("input").focus();
}

function queueBlob(bytes: Uint8Array, fileName: string, mime: string) {
  const preview = mime.startsWith("image/") ? URL.createObjectURL(new Blob([bytes as BlobPart], { type: mime })) : undefined;
  pending.push({ kind: "blob", key: `b${pendingSeq++}`, bytes, file_name: fileName, size: bytes.length, mime, preview });
  renderPending();
}

function clearPending() {
  pending.splice(0).forEach(releasePreview);
  renderPending();
  $("input").focus();
}

/// Send everything queued. Items that fail stay in the queue so the user can
/// retry or drop them.
async function sendPending() {
  const items = pending.splice(0);
  renderPending();
  const failed: Pending[] = [];
  // One call per item, in queue order, so a failure only re-queues that item
  // (a batched `send_files` drops the messages it already sent when a later
  // path fails).
  for (const p of items) {
    const ok = p.kind === "path" ? await sendPath(p.path) : await sendBlob(p.bytes, p.file_name, p.mime);
    if (ok) releasePreview(p);
    else failed.push(p);
  }
  if (failed.length) {
    pending.unshift(...failed);
    renderPending();
  }
}

async function pickFiles() {
  const picked = await openDialog({ multiple: true, directory: false });
  if (!picked) return;
  await queuePaths(Array.isArray(picked) ? picked : [picked]);
}

async function sendPath(path: string): Promise<boolean> {
  try {
    const sent = await api.sendFiles(target(), [path]);
    sent.forEach((m) => addMessage(m, false));
    return true;
  } catch (e) {
    toast(friendlyError(e), true);
    return false;
  }
}

async function sendBlob(bytes: Uint8Array, fileName: string, mime: string, durationMs?: number): Promise<boolean> {
  try {
    const data = await bytesToBase64(bytes);
    const m = await api.sendBlob(target(), { file_name: fileName, mime, data, duration_ms: durationMs });
    addMessage(m, false);
    return true;
  } catch (e) {
    toast(friendlyError(e), true);
    return false;
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
  void fillMicSelect();
}

async function fillMicSelect() {
  const sel = $<HTMLSelectElement>("mic-select");
  const chosen = preferredMic();
  const mics = await listMics();
  sel.replaceChildren();
  const def = document.createElement("option");
  def.value = "";
  def.textContent = t().microphoneDefault;
  sel.appendChild(def);
  for (const m of mics) {
    const o = document.createElement("option");
    o.value = m.id;
    o.textContent = m.label;
    sel.appendChild(o);
  }
  // Keep a saved device that is currently unplugged selectable, so the
  // choice is not silently lost.
  if (chosen && !mics.some((m) => m.id === chosen)) {
    const o = document.createElement("option");
    o.value = chosen;
    o.textContent = "(" + t().offline + ")";
    sel.appendChild(o);
  }
  sel.value = chosen;
}

async function submitSettings(ev: Event) {
  ev.preventDefault();
  const f = $<HTMLFormElement>("settings-form");
  const newLang = (f.elements.namedItem("lang") as HTMLSelectElement).value as Lang;
  if (newLang !== lang()) {
    setLang(newLang);
    applyStatic();
    renderPending();
  }
  setPreferredMic((f.elements.namedItem("mic") as HTMLSelectElement).value);
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

// ---------------------------------------------------------------- add computer

/** What the "Add computer" dialog currently lists, keyed by peer id. */
const found = new Map<string, FoundPeer>();
let scanRunning = false;

function openPeerModal() {
  $("peer-modal").hidden = false;
  found.clear();
  renderFound();
  void refreshFound();
}

/** Show what mDNS already knows at once, then run the slower subnet scan. */
async function refreshFound() {
  try {
    mergeFound(await api.discoveredPeers());
  } catch {
    /* node not running: the scan below reports it */
  }
  if (scanRunning) return;
  scanRunning = true;
  setScanStatus("scanning");
  try {
    // The scan already includes what mDNS knows, so it is the whole list:
    // a computer that has gone away since the last scan must drop out.
    const list = await api.scanNetwork();
    found.clear();
    mergeFound(list);
    setScanStatus("done");
  } catch (e) {
    setScanStatus("failed");
    toast(`${t().scanFailed}: ${errText(e)}`, true);
  } finally {
    scanRunning = false;
  }
}

function mergeFound(list: FoundPeer[]) {
  for (const p of list) {
    if (p.id === state.me.id) continue;
    const live = state.peers.get(p.id);
    found.set(p.id, live ? { ...p, connected: true, name: live.name, addr: live.addr ?? p.addr } : p);
  }
  renderFound();
}

function setScanStatus(kind: "scanning" | "done" | "failed") {
  const el = $("found-status");
  el.replaceChildren();
  $<HTMLButtonElement>("btn-scan").disabled = kind === "scanning";
  if (kind === "scanning") {
    const sp = document.createElement("span");
    sp.className = "spinner";
    el.append(sp, document.createTextNode(t().scanning));
  } else if (kind === "done") {
    el.textContent = t().scanDone(found.size);
  } else {
    el.textContent = t().scanFailed;
  }
}

function renderFound() {
  const list = $("found-list");
  list.replaceChildren();
  const rows = [...found.values()].sort((a, b) => a.name.localeCompare(b.name));
  if (rows.length === 0) {
    const e = document.createElement("div");
    e.className = "found-empty";
    e.textContent = t().foundEmpty;
    list.appendChild(e);
    return;
  }
  for (const p of rows) {
    const connected = p.connected || state.peers.has(p.id);
    const row = document.createElement("div");
    row.className = "found-row";
    row.appendChild(avatar(p.id, p.name));
    const text = document.createElement("div");
    text.className = "found-text";
    const name = document.createElement("div");
    name.className = "found-name";
    name.textContent = p.name || "?";
    if (p.relay) {
      const tag = document.createElement("span");
      tag.className = "tag";
      tag.textContent = t().hub;
      name.appendChild(tag);
    }
    const meta = document.createElement("div");
    meta.className = "found-meta";
    meta.textContent = `${p.addr} · ${p.via === "mdns" ? t().viaMdns : t().viaScan}`;
    text.append(name, meta);
    row.appendChild(text);
    if (connected) {
      const ok = document.createElement("span");
      ok.className = "found-ok";
      const dot = document.createElement("span");
      dot.className = "dot on";
      ok.append(dot, document.createTextNode(t().connected));
      row.appendChild(ok);
    } else {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "primary-btn";
      btn.textContent = t().connect;
      btn.onclick = () => void connectTo(p.addr, btn);
      row.appendChild(btn);
    }
    list.appendChild(row);
  }
}

/** Connect to an address and, if the box is ticked, keep it in settings. */
async function connectTo(target: string, btn: HTMLButtonElement): Promise<boolean> {
  const remember = $<HTMLInputElement>("peer-remember").checked;
  btn.disabled = true;
  try {
    await api.connectPeer(target);
    toast(t().connected);
    if (remember && !state.settings.manual_peers.includes(target)) {
      const snap = await api.saveSettings({ ...state.settings, manual_peers: [...state.settings.manual_peers, target] });
      applySnapshot(snap);
    }
    renderFound();
    return true;
  } catch (e) {
    toast(`${t().connectFailed}: ${errText(e)}`, true);
    return false;
  } finally {
    btn.disabled = false;
  }
}

async function submitPeer(ev: Event) {
  ev.preventDefault();
  const f = $<HTMLFormElement>("peer-form");
  const input = f.elements.namedItem("target") as HTMLInputElement;
  const target = input.value.trim();
  if (!target) return;
  const btn = f.querySelector<HTMLButtonElement>("button[type=submit]")!;
  if (await connectTo(target, btn)) {
    $("peer-modal").hidden = true;
    input.value = "";
  }
}

// ---------------------------------------------------------------- first run

/** Ask for a name once, before the app is used. Prefilled with the computer name. */
function openWelcome() {
  const f = $<HTMLFormElement>("welcome-form");
  const input = f.elements.namedItem("display_name") as HTMLInputElement;
  input.value = state.settings.display_name || state.me.name;
  $("welcome-modal").hidden = false;
  setTimeout(() => {
    input.focus();
    input.select();
  }, 50);
}

async function submitWelcome(ev: Event) {
  ev.preventDefault();
  const f = $<HTMLFormElement>("welcome-form");
  const input = f.elements.namedItem("display_name") as HTMLInputElement;
  const name = input.value.trim();
  if (!name) {
    // Spaces only satisfy `required`; say why nothing happens.
    input.value = "";
    f.reportValidity();
    return;
  }
  const btn = f.querySelector<HTMLButtonElement>("button[type=submit]")!;
  btn.disabled = true;
  try {
    const snap = await api.saveSettings({ ...state.settings, display_name: name });
    applySnapshot(snap);
    $("welcome-modal").hidden = true;
    $("input").focus();
  } catch (e) {
    // The name may be on disk even though the network failed to restart
    // (port in use, ...). This dialog has no other way out, so check: once
    // settings exist the error belongs to the main window, not here.
    const snap = await api.snapshot().catch(() => null);
    if (snap && !snap.first_run) {
      applySnapshot(snap); // reports start_error itself
      $("welcome-modal").hidden = true;
      $("input").focus();
    } else {
      toast(errText(e), true);
    }
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
  state.firstRun = snap.first_run;
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
  for (const tr of state.transfers.values()) {
    const pct = tr.total > 0 ? Math.floor((tr.done / tr.total) * 100) : 0;
    const row = document.createElement("div");
    row.className = "transfer";
    const label = document.createElement("div");
    label.textContent = t().receivingFile(pct);
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

function outgoingOf(id: string) {
  let m = state.outgoing.get(id);
  if (!m) state.outgoing.set(id, (m = new Map()));
  return m;
}

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
      if (!$("peer-modal").hidden) renderFound();
      if (state.active === ALL && state.messages.every((m) => m.to)) renderMessages();
      break;
    case "peer_offline":
      state.peers.delete(e.id);
      renderSidebar();
      renderHeader();
      if (!$("peer-modal").hidden) {
        const p = found.get(e.id);
        if (p) found.set(e.id, { ...p, connected: false });
        renderFound();
      }
      break;
    case "message":
      addMessage(e.message, true);
      break;
    case "transfer": {
      if (e.direction === "sending") {
        outgoingOf(e.message_id).set(e.peer, { done: e.done, total: e.total });
        updateStatus(e.message_id);
        break;
      }
      const key = e.message_id + ":" + e.peer;
      if (e.done >= e.total) state.transfers.delete(key);
      else state.transfers.set(key, { done: e.done, total: e.total });
      renderTransfers();
      break;
    }
    case "transfer_failed":
      if (e.direction === "sending") {
        const out = outgoingOf(e.message_id);
        out.set(e.peer, { done: out.get(e.peer)?.done ?? 0, total: out.get(e.peer)?.total ?? 0, error: e.reason });
        updateStatus(e.message_id);
        // The bubble shows it; speak up only when it is out of sight.
        const m = state.messages.find((x) => x.id === e.message_id);
        if (!m || conversationOf(m) !== state.active) toast(t().transferFailed(e.reason), true);
        break;
      }
      state.transfers.delete(e.message_id + ":" + e.peer);
      renderTransfers();
      toast(t().receiveFailed(e.reason), true);
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
          queueBlob(new Uint8Array(buf), `pasted-${Date.now()}.${ext}`, item.type);
        });
      }
    }
  });
  $("btn-send").onclick = () => void sendCurrentText();
  $("btn-attach").onclick = () => void pickFiles();
  $("btn-pending-clear").onclick = clearPending;
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
      state.outgoing.clear();
      renderAll();
      $("settings-modal").hidden = true;
    } catch (e) {
      toast(errText(e), true);
    }
  };
  $("btn-add-peer").onclick = openPeerModal;
  $("btn-peer-close").onclick = () => ($("peer-modal").hidden = true);
  $("btn-scan").onclick = () => void refreshFound();
  $("peer-form").addEventListener("submit", (e) => void submitPeer(e));
  $("welcome-form").addEventListener("submit", (e) => void submitWelcome(e));
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
      void queuePaths(ev.payload.paths);
    }
  });

  await api.onNetEvent(onNetEvent);
  applySnapshot(await api.snapshot());
  if (state.firstRun) openWelcome();
  // The node may finish starting after the first snapshot; refresh once.
  setTimeout(async () => applySnapshot(await api.snapshot()), 1500);
  input.focus();
}

main().catch((e) => toast(errText(e), true));
