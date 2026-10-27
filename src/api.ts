import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type AttachmentKind = "image" | "audio" | "file";

export interface Attachment {
  kind: AttachmentKind;
  file_name: string;
  mime: string;
  size: number;
  sha256: string;
  duration_ms?: number;
  path?: string;
}

export interface Message {
  id: string;
  from: string;
  from_name: string;
  to?: string;
  sent_at: number;
  text?: string;
  attachment?: Attachment;
}

export interface Peer {
  id: string;
  name: string;
  addr?: string;
  relay: boolean;
  direct: boolean;
  via?: string;
}

export interface Settings {
  display_name: string;
  port: number;
  discovery: boolean;
  relay: boolean;
  manual_peers: string[];
}

export interface Snapshot {
  me: { id: string; name: string };
  settings: Settings;
  port: number | null;
  addresses: string[];
  peers: Peer[];
  messages: Message[];
  data_dir: string;
  start_error: string | null;
}

export type NetEvent =
  | { type: "ready"; port: number; addresses: string[] }
  | { type: "peer_online"; peer: Peer }
  | { type: "peer_offline"; id: string }
  | { type: "message"; message: Message }
  | { type: "error"; message: string };

export const api = {
  snapshot: () => invoke<Snapshot>("app_snapshot"),
  saveSettings: (settings: Settings) => invoke<Snapshot>("save_settings", { settings }),
  sendText: (to: string | null, text: string) => invoke<Message>("send_text", { to, text }),
  sendFiles: (to: string | null, paths: string[]) => invoke<Message[]>("send_files", { to, paths }),
  sendBlob: (to: string | null, blob: { file_name: string; mime: string; data: string; duration_ms?: number }) =>
    invoke<Message>("send_blob", { to, blob }),
  connectPeer: (target: string) => invoke<void>("connect_peer", { target }),
  openAttachment: (messageId: string) => invoke<void>("open_attachment", { messageId }),
  revealAttachment: (messageId: string) => invoke<void>("reveal_attachment", { messageId }),
  clearHistory: () => invoke<void>("clear_history"),
  onNetEvent: (cb: (e: NetEvent) => void): Promise<UnlistenFn> =>
    listen<NetEvent>("net", (ev) => cb(ev.payload)),
  fileUrl: (path: string) => convertFileSrc(path),
};

export function bytesToBase64(bytes: Uint8Array): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error);
    reader.onload = () => {
      const url = reader.result as string;
      resolve(url.slice(url.indexOf(",") + 1));
    };
    reader.readAsDataURL(new Blob([bytes as BlobPart]));
  });
}
