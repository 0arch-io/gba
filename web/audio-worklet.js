// Pulls interleaved stereo f32 out of a queue the main thread keeps topped up.
// Underrun plays silence; the main thread drops samples when the queue grows
// too long, which mirrors what the desktop cpal frontend does.

class GbaSink extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = [];
    this.offset = 0;
    this.queued = 0;
    this.port.onmessage = (e) => {
      if (e.data === "flush") {
        this.queue = [];
        this.offset = 0;
        this.queued = 0;
        return;
      }
      this.queue.push(e.data);
      this.queued += e.data.length;
    };
  }

  next() {
    while (this.queue.length > 0) {
      const chunk = this.queue[0];
      if (this.offset < chunk.length) {
        this.queued -= 1;
        return chunk[this.offset++];
      }
      this.queue.shift();
      this.offset = 0;
    }
    return 0;
  }

  process(_inputs, outputs) {
    const left = outputs[0][0];
    const right = outputs[0][1];
    for (let i = 0; i < left.length; i++) {
      const l = this.next();
      const r = this.next();
      left[i] = l;
      if (right) right[i] = r;
    }
    // Report backlog so the main thread can throttle.
    this.port.postMessage(this.queued);
    return true;
  }
}

registerProcessor("gba-sink", GbaSink);
