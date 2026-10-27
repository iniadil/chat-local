/**
 * Records a mono 16 kHz, 16-bit PCM WAV from the microphone.
 *
 * WAV is chosen on purpose: every WebView (WebView2 on Windows, WKWebView on
 * macOS) can play it, whereas MediaRecorder's output format differs per
 * platform (webm/opus vs mp4/aac) and does not always play on the other side.
 */

const TARGET_RATE = 16000;
const MAX_SECONDS = 5 * 60;

export class Recorder {
  private ctx: AudioContext | null = null;
  private stream: MediaStream | null = null;
  private chunks: Float32Array[] = [];
  private sourceRate = TARGET_RATE;
  private startedAt = 0;
  private stopTimer: number | undefined;
  private onAutoStop: (() => void) | null = null;

  get running() {
    return this.ctx !== null;
  }

  elapsedMs() {
    return this.ctx ? Date.now() - this.startedAt : 0;
  }

  async start(onAutoStop: () => void) {
    this.stream = await navigator.mediaDevices.getUserMedia({
      audio: { echoCancellation: true, noiseSuppression: true, channelCount: 1 },
    });
    let ctx: AudioContext;
    try {
      ctx = new AudioContext({ sampleRate: TARGET_RATE });
    } catch {
      ctx = new AudioContext();
    }
    this.ctx = ctx;
    this.sourceRate = ctx.sampleRate;
    this.chunks = [];
    const source = ctx.createMediaStreamSource(this.stream);
    // ScriptProcessorNode is deprecated but is the one capture path that
    // works the same in every WebView we ship to.
    const proc = ctx.createScriptProcessor(4096, 1, 1);
    proc.onaudioprocess = (e) => {
      this.chunks.push(new Float32Array(e.inputBuffer.getChannelData(0)));
    };
    source.connect(proc);
    proc.connect(ctx.destination);
    this.startedAt = Date.now();
    this.onAutoStop = onAutoStop;
    this.stopTimer = window.setTimeout(() => this.onAutoStop?.(), MAX_SECONDS * 1000);
  }

  /** Stop and return the WAV bytes plus the duration, or null if cancelled. */
  async stop(keep: boolean): Promise<{ wav: Uint8Array; durationMs: number } | null> {
    window.clearTimeout(this.stopTimer);
    const durationMs = this.elapsedMs();
    this.stream?.getTracks().forEach((t) => t.stop());
    this.stream = null;
    const ctx = this.ctx;
    this.ctx = null;
    if (ctx) await ctx.close().catch(() => undefined);
    const chunks = this.chunks;
    this.chunks = [];
    if (!keep || chunks.length === 0) return null;

    let samples = concat(chunks);
    if (this.sourceRate !== TARGET_RATE) samples = resample(samples, this.sourceRate, TARGET_RATE);
    return { wav: encodeWav(samples, TARGET_RATE), durationMs };
  }
}

function concat(chunks: Float32Array[]): Float32Array {
  const len = chunks.reduce((n, c) => n + c.length, 0);
  const out = new Float32Array(len);
  let o = 0;
  for (const c of chunks) {
    out.set(c, o);
    o += c.length;
  }
  return out;
}

function resample(input: Float32Array, from: number, to: number): Float32Array {
  const ratio = from / to;
  const outLen = Math.floor(input.length / ratio);
  const out = new Float32Array(outLen);
  for (let i = 0; i < outLen; i++) {
    const pos = i * ratio;
    const i0 = Math.floor(pos);
    const i1 = Math.min(i0 + 1, input.length - 1);
    const frac = pos - i0;
    out[i] = input[i0] * (1 - frac) + input[i1] * frac;
  }
  return out;
}

function encodeWav(samples: Float32Array, rate: number): Uint8Array {
  const bytesPerSample = 2;
  const dataLen = samples.length * bytesPerSample;
  const buf = new ArrayBuffer(44 + dataLen);
  const v = new DataView(buf);
  const str = (o: number, s: string) => {
    for (let i = 0; i < s.length; i++) v.setUint8(o + i, s.charCodeAt(i));
  };
  str(0, "RIFF");
  v.setUint32(4, 36 + dataLen, true);
  str(8, "WAVE");
  str(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true); // PCM
  v.setUint16(22, 1, true); // mono
  v.setUint32(24, rate, true);
  v.setUint32(28, rate * bytesPerSample, true);
  v.setUint16(32, bytesPerSample, true);
  v.setUint16(34, 16, true);
  str(36, "data");
  v.setUint32(40, dataLen, true);
  let o = 44;
  for (let i = 0; i < samples.length; i++, o += 2) {
    const s = Math.max(-1, Math.min(1, samples[i]));
    v.setInt16(o, s < 0 ? s * 0x8000 : s * 0x7fff, true);
  }
  return new Uint8Array(buf);
}
