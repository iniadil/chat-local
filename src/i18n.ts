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
    foundTitle: "Komputer yang memakai ChatLocal",
    foundHelp: "Daftar ini berasal dari pengumuman mDNS/Bonjour dan pemindaian setiap alamat di jaringan lokal (subnet /24) pada port ChatLocal. Butuh beberapa detik.",
    scanning: "Memindai jaringan…",
    scanAgain: "Pindai ulang",
    scanDone: (n: number) => (n === 0 ? "Tidak ada yang ditemukan" : `${n} ditemukan`),
    scanFailed: "Pemindaian gagal",
    foundEmpty: "Belum ada komputer lain yang ditemukan. Pastikan ChatLocal sudah dibuka di komputer lain dan terhubung ke Wi-Fi/LAN yang sama, atau masukkan alamatnya di bawah.",
    viaMdns: "otomatis",
    viaScan: "pemindaian",
    orManual: "atau masukkan alamat secara manual",
    welcomeTitle: "Selamat datang di ChatLocal",
    welcomeIntro: "Sebelum mulai, tentukan nama yang akan dilihat orang lain di jaringan ini.",
    welcomeName: "Nama kamu atau nama perangkat ini",
    welcomeNameHelp: "Contoh: 'Adil' atau 'PC Dapur'. Bisa diubah kapan saja di Pengaturan.",
    welcomeStart: "Mulai",
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
    dropHere: "Lepaskan untuk melampirkan",
    pendingHint: (n: number) => `${n} lampiran siap dikirim`,
    removeAttachment: "Hapus lampiran",
    clearAttachments: "Hapus semua",
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
    receivingFile: (pct: number) => `Menerima berkas… ${pct}%`,
    transferFailed: (reason: string) => `Berkas gagal dikirim: ${reason}`,
    receiveFailed: (reason: string) => `Berkas gagal diterima: ${reason}`,
    sending: "Sedang dikirim",
    sent: "Terkirim",
    sendFailed: "Gagal terkirim",
    retry: "Kirim ulang",
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
    foundTitle: "Computers running ChatLocal",
    foundHelp: "Combines mDNS/Bonjour announcements with a probe of every address on the local network (/24 subnet) at the ChatLocal port. Takes a few seconds.",
    scanning: "Scanning the network…",
    scanAgain: "Scan again",
    scanDone: (n: number) => (n === 0 ? "Nothing found" : `${n} found`),
    scanFailed: "Scan failed",
    foundEmpty: "No other computer found yet. Make sure ChatLocal is open on the other computer and on the same Wi-Fi/LAN, or type its address below.",
    viaMdns: "automatic",
    viaScan: "scan",
    orManual: "or enter an address by hand",
    welcomeTitle: "Welcome to ChatLocal",
    welcomeIntro: "Before you start, choose the name other people on this network will see.",
    welcomeName: "Your name or this device's name",
    welcomeNameHelp: "For example 'Adil' or 'Kitchen PC'. You can change it any time in Settings.",
    welcomeStart: "Start",
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
    dropHere: "Drop to attach",
    pendingHint: (n: number) => `${n} attachment${n === 1 ? "" : "s"} ready to send`,
    removeAttachment: "Remove attachment",
    clearAttachments: "Clear all",
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
    receivingFile: (pct: number) => `Receiving a file… ${pct}%`,
    transferFailed: (reason: string) => `File could not be sent: ${reason}`,
    receiveFailed: (reason: string) => `File could not be received: ${reason}`,
    sending: "Sending",
    sent: "Sent",
    sendFailed: "Not sent",
    retry: "Retry",
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
