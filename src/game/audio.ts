// All sounds are synthesized with WebAudio at runtime. No audio files ship.

export class Sound {
  private ctx: AudioContext | null = null;
  private master: GainNode | null = null;
  private engine: { osc: OscillatorNode; gain: GainNode; filter: BiquadFilterNode } | null = null;
  private beepTimer = 0;
  muted = false;

  /** Browsers only allow audio after a user gesture. */
  wake(): void {
    if (this.ctx) {
      if (this.ctx.state === "suspended") void this.ctx.resume();
      return;
    }
    try {
      this.ctx = new AudioContext();
    } catch {
      return;
    }
    this.master = this.ctx.createGain();
    this.master.gain.value = this.muted ? 0 : 0.5;
    this.master.connect(this.ctx.destination);
    const osc = this.ctx.createOscillator();
    osc.type = "sawtooth";
    osc.frequency.value = 38;
    const filter = this.ctx.createBiquadFilter();
    filter.type = "lowpass";
    filter.frequency.value = 220;
    const gain = this.ctx.createGain();
    gain.gain.value = 0;
    osc.connect(filter).connect(gain).connect(this.master);
    osc.start();
    this.engine = { osc, gain, filter };
  }

  setMuted(m: boolean): void {
    this.muted = m;
    if (this.master && this.ctx) this.master.gain.setTargetAtTime(m ? 0 : 0.5, this.ctx.currentTime, 0.02);
  }

  private tone(freq: number, dur: number, type: OscillatorType = "square", vol = 0.15, slideTo?: number): void {
    if (!this.ctx || !this.master) return;
    const t = this.ctx.currentTime;
    const o = this.ctx.createOscillator();
    const g = this.ctx.createGain();
    o.type = type;
    o.frequency.setValueAtTime(freq, t);
    if (slideTo) o.frequency.exponentialRampToValueAtTime(slideTo, t + dur);
    g.gain.setValueAtTime(vol, t);
    g.gain.exponentialRampToValueAtTime(0.0001, t + dur);
    o.connect(g).connect(this.master);
    o.start(t);
    o.stop(t + dur + 0.02);
  }

  private noise(dur: number, vol: number, cutoff: number): void {
    if (!this.ctx || !this.master) return;
    const t = this.ctx.currentTime;
    const len = Math.floor(this.ctx.sampleRate * dur);
    const buf = this.ctx.createBuffer(1, len, this.ctx.sampleRate);
    const d = buf.getChannelData(0);
    for (let i = 0; i < len; i++) d[i] = (Math.random() * 2 - 1) * (1 - i / len) ** 2;
    const src = this.ctx.createBufferSource();
    src.buffer = buf;
    const f = this.ctx.createBiquadFilter();
    f.type = "lowpass";
    f.frequency.value = cutoff;
    const g = this.ctx.createGain();
    g.gain.setValueAtTime(vol, t);
    src.connect(f).connect(g).connect(this.master);
    src.start(t);
  }

  /** Engine idle/rev plus the reverse beeper, called every frame. */
  drive(speedFrac: number, reversing: boolean, dtMs: number): void {
    if (!this.ctx || !this.engine) return;
    const t = this.ctx.currentTime;
    const s = Math.abs(speedFrac);
    this.engine.osc.frequency.setTargetAtTime(38 + s * 50, t, 0.08);
    this.engine.filter.frequency.setTargetAtTime(200 + s * 500, t, 0.08);
    this.engine.gain.gain.setTargetAtTime(0.05 + s * 0.08, t, 0.1);
    if (reversing) {
      this.beepTimer -= dtMs;
      if (this.beepTimer <= 0) {
        this.tone(1050, 0.32, "square", 0.08);
        this.beepTimer = 640;
      }
    } else {
      this.beepTimer = 0;
    }
  }

  forks(up: boolean): void {
    this.tone(up ? 140 : 220, 0.35, "sawtooth", 0.05, up ? 240 : 120);
  }
  pickup(): void {
    this.noise(0.08, 0.5, 900);
    this.tone(90, 0.12, "sine", 0.35);
  }
  setDown(): void {
    this.noise(0.1, 0.4, 600);
    this.tone(70, 0.15, "sine", 0.3);
  }
  bump(impact: number): void {
    this.noise(0.12, 0.3 + impact * 0.5, 400);
    this.tone(55, 0.18, "sine", 0.3 + impact * 0.3);
  }
  crash(): void {
    this.noise(0.6, 0.9, 2200);
    this.tone(60, 0.5, "sine", 0.5, 30);
    setTimeout(() => this.noise(0.25, 0.4, 3000), 180);
    setTimeout(() => this.noise(0.2, 0.3, 2500), 340);
  }
  refuse(): void {
    this.tone(150, 0.18, "square", 0.08);
    setTimeout(() => this.tone(110, 0.22, "square", 0.08), 120);
  }
  delivered(): void {
    this.tone(660, 0.16, "triangle", 0.18);
    setTimeout(() => this.tone(990, 0.3, "triangle", 0.16), 110);
  }
  stamp(): void {
    this.noise(0.07, 0.8, 1200);
    this.tone(80, 0.1, "sine", 0.4);
  }
}
