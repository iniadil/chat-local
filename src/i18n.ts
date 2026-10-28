export type Lang = "id" | "en";

const strings = {
  id: {
    everyone: "Semua orang",
    everyoneSub: (n: number) => (n === 0 ? "Belum ada yang online" : `${n} orang online`),
    online: "Online",
    recent: "Riwayat",
    offline: "offline",
    viaHub: "lewat server",
    hub: "server",
    you: "Kamu",
    addComputer: "Tambah komputer…",
    peerAddress: "Alamat komputer",
    peerAddressHelp: "Alamat IP atau nama komputer lain, terlihat di bagian bawah sidebar aplikasi di komputer tersebut.",
    rememberPeer: "Ingat dan sambungkan otomatis",
    connect: "Sambungkan",
    connected: "Tersambung",
    connectFailed: "Tidak bisa tersambung",
    settings: "Pengaturan",
    displayName: "Nama tampilan",
    language: "Bahasa",
    network: "Jaringan",
    discovery: "Temukan komputer lain secara otomatis",
    discoveryHelp: "Menggunakan mDNS/Bonjour di jaringan yang sama. Matikan jika jaringan memblokirnya, lalu isi alamat secara manual.",
    port: "Port",
    manualPeers: "Alamat manual (satu per baris)",
    manualPeersHelp: "Isi alamat komputer lain atau alamat server ChatLocal. Format: IP atau IP:port.",
    serverMode: "Mode server (opsional)",
    relay: "Jadikan komputer ini server (relay)",
    relayHelp: "Semua pesan yang masuk diteruskan ke semua komputer yang tersambung ke sini, dan daftar peserta dibagikan. Berguna jika komputer tidak bisa saling melihat secara langsung. Komputer lain cukup mengisi alamat komputer ini di 'Alamat manual'.",
    yourAddresses: "Alamat komputer ini",
    dataFolder: "Folder data",
    clearHistory: "Hapus riwayat",
    clearHistoryConfirm: "Hapus semua riwayat pesan di komputer ini? File yang sudah diterima tetap ada di folder data.",
    saveRestart: "Simpan",
    saved: "Pengaturan disimpan",
    send: "Kirim",
    cancel: "Batal",
    close: "Tutup",
    openFile: "Buka",
    showInFolder: "Tampilkan di folder",
    recordingHint: "Merekam… klik Kirim jika sudah selesai",
    micDenied: "Mikrofon tidak bisa diakses. Periksa izin mikrofon untuk ChatLocal.",
    dropHere: "Lepaskan untuk mengirim",
    placeholderAll: "Tulis pesan untuk semua orang…",
    placeholderTo: (name: string) => `Tulis pesan untuk ${name}…`,
    emptyTitle: "Belum ada percakapan",
    emptyAll: "Buka ChatLocal di komputer lain yang terhubung ke Wi-Fi/LAN yang sama. Komputer itu akan muncul di sidebar secara otomatis. Jika tidak muncul, tambahkan alamatnya lewat tombol 'Tambah komputer…'.",
    emptyPeer: "Belum ada pesan. Pesan langsung hanya terlihat oleh kalian berdua.",
    noPeersSidebar: "Belum ada komputer lain yang terlihat.",
    voiceNote: "Pesan suara",
    image: "Gambar",
    file: "Berkas",
    today: "Hari ini",
    yesterday: "Kemarin",
    peerOffline: "Komputer ini sedang offline. Pesan tidak bisa dikirim sampai online lagi.",
    networkStopped: "Jaringan tidak aktif",
    listening: (port: number) => `Port ${port}`,
    unreachable: "Komputer tujuan tidak terjangkau",
    tooLarge: "Berkas terlalu besar",
    copied: "Disalin",
    voiceNotes: "Pesan suara",
    microphone: "Mikrofon",
    microphoneDefault: "Bawaan sistem",
    microphoneHelp: "Nama mikrofon baru terlihat setelah pesan suara pertama direkam, saat izin mikrofon sudah diberikan.",
    sendingFile: (name: string, pct: number) => `Mengirim ${name || "berkas"}… ${pct}%`,
    receivingFile: (pct: number) => `Menerima berkas… ${pct}%`,
    transferFailed: (reason: string) => `Berkas gagal dikirim: ${reason}`,
  },
  en: {
    everyone: "Everyone",
    everyoneSub: (n: number) => (n === 0 ? "No one online yet" : `${n} online`),
    online: "Online",
    recent: "History",
    offline: "offline",
    viaHub: "via server",
    hub: "server",
    you: "You",
    addComputer: "Add computer…",
    peerAddress: "Computer address",
    peerAddressHelp: "The IP address or host name of the other computer, shown at the bottom of its sidebar.",
    rememberPeer: "Remember and reconnect automatically",
    connect: "Connect",
    connected: "Connected",
    connectFailed: "Could not connect",
    settings: "Settings",
    displayName: "Display name",
    language: "Language",
    network: "Network",
    discovery: "Find other computers automatically",
    discoveryHelp: "Uses mDNS/Bonjour on the same network. Turn it off if your network blocks it and add addresses by hand.",
    port: "Port",
    manualPeers: "Manual addresses (one per line)",
    manualPeersHelp: "Addresses of other computers or of a ChatLocal server. Format: IP or IP:port.",
    serverMode: "Server mode (optional)",
    relay: "Make this computer a server (relay)",
    relayHelp: "Every message received here is forwarded to all connected computers, and the list of participants is shared. Useful when computers cannot see each other directly. Other computers only need this computer's address under 'Manual addresses'.",
    yourAddresses: "This computer's addresses",
    dataFolder: "Data folder",
    clearHistory: "Clear history",
    clearHistoryConfirm: "Delete all message history on this computer? Received files stay in the data folder.",
    saveRestart: "Save",
    saved: "Settings saved",
    send: "Send",
    cancel: "Cancel",
    close: "Close",
    openFile: "Open",
    showInFolder: "Show in folder",
    recordingHint: "Recording… click Send when done",
    micDenied: "The microphone is not available. Check the microphone permission for ChatLocal.",
    dropHere: "Drop to send",
    placeholderAll: "Message everyone…",
    placeholderTo: (name: string) => `Message ${name}…`,
    emptyTitle: "No conversation yet",
    emptyAll: "Open ChatLocal on another computer on the same Wi-Fi/LAN. It will show up in the sidebar automatically. If it does not, add its address with 'Add computer…'.",
    emptyPeer: "No messages yet. Direct messages are only seen by the two of you.",
    noPeersSidebar: "No other computer is visible yet.",
    voiceNote: "Voice note",
    image: "Image",
    file: "File",
    today: "Today",
    yesterday: "Yesterday",
    peerOffline: "This computer is offline. Messages cannot be sent until it is back.",
    networkStopped: "Networking is off",
    listening: (port: number) => `Port ${port}`,
    unreachable: "The other computer cannot be reached",
    tooLarge: "File is too large",
    copied: "Copied",
    voiceNotes: "Voice notes",
    microphone: "Microphone",
    microphoneDefault: "System default",
    microphoneHelp: "Microphone names appear after the first voice note, once the microphone permission has been granted.",
    sendingFile: (name: string, pct: number) => `Sending ${name || "a file"}… ${pct}%`,
    receivingFile: (pct: number) => `Receiving a file… ${pct}%`,
    transferFailed: (reason: string) => `File transfer failed: ${reason}`,
  },
} as const;

export type Strings = (typeof strings)["en"];

let current: Lang = detect();

function detect(): Lang {
  try {
    const saved = localStorage.getItem("lang");
    if (saved === "id" || saved === "en") return saved;
  } catch {
    /* storage may be unavailable */
  }
  return navigator.language.toLowerCase().startsWith("id") ? "id" : "en";
}

export function lang(): Lang {
  return current;
}

export function setLang(l: Lang) {
  current = l;
  try {
    localStorage.setItem("lang", l);
  } catch {
    /* ignore */
  }
  document.documentElement.lang = l;
}

export function t(): Strings {
  return strings[current] as Strings;
}

/** Fill every element carrying `data-i18n` with its string. */
export function applyStatic() {
  const s = t() as unknown as Record<string, unknown>;
  document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    const v = s[el.dataset.i18n ?? ""];
    if (typeof v === "string") el.textContent = v;
  });
}
