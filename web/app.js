import init, { Emulator } from "./pkg/gba.js";

const TEST_ROM =
  "https://raw.githubusercontent.com/jsmolka/gba-tests/master/arm/arm.gba";

const canvas = document.getElementById("screen");
const ctx = canvas.getContext("2d");
const statusEl = document.getElementById("status");
const overlay = document.getElementById("overlay");
const startBtn = document.getElementById("start");
const demoBtn = document.getElementById("demo");
const romInput = document.getElementById("rom");

const image = ctx.createImageData(240, 160);
let emu = null;
let romName = "";
let saveKey = "";
let audioCtx = null;
let sink = null;
let backlog = 0;
let running = false;

function status(text) {
  statusEl.textContent = text;
}

// Keyboard, matching the desktop build and the README table. The values are
// KEYINPUT bit positions: A, B, Select, Start, Right, Left, Up, Down, R, L.
const KEYS = {
  KeyZ: 0,
  KeyX: 1,
  ShiftRight: 2,
  Enter: 3,
  ArrowRight: 4,
  ArrowLeft: 5,
  ArrowUp: 6,
  ArrowDown: 7,
  KeyW: 8,
  KeyQ: 9,
};
let pressed = 0;

function key(e, down) {
  if (!(e.code in KEYS)) return;
  const bit = 1 << KEYS[e.code];
  pressed = down ? pressed | bit : pressed & ~bit;
  e.preventDefault();
}

window.addEventListener("keydown", (e) => key(e, true));
window.addEventListener("keyup", (e) => key(e, false));
window.addEventListener("blur", () => {
  pressed = 0;
});

async function startAudio(sampleRate) {
  if (audioCtx) {
    await audioCtx.resume();
    return;
  }
  audioCtx = new AudioContext({ sampleRate });
  await audioCtx.audioWorklet.addModule("./audio-worklet.js");
  sink = new AudioWorkletNode(audioCtx, "gba-sink", {
    numberOfInputs: 0,
    numberOfOutputs: 1,
    outputChannelCount: [2],
  });
  sink.port.onmessage = (e) => {
    backlog = e.data;
  };
  sink.connect(audioCtx.destination);
}

// Battery saves have nowhere to go in a browser tab, so they are mirrored into
// local storage under the cartridge title and restored on the next load.
function persistSave() {
  if (!emu || !saveKey || !emu.save_dirty()) return;
  const bytes = emu.save_data();
  if (bytes.length === 0) return;
  try {
    let s = "";
    for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
    localStorage.setItem(saveKey, btoa(s));
  } catch {
    // Quota errors are not worth interrupting play for.
  }
}

function restoreSave() {
  if (!saveKey) return;
  const stored = localStorage.getItem(saveKey);
  if (!stored) return;
  try {
    const raw = atob(stored);
    const bytes = new Uint8Array(raw.length);
    for (let i = 0; i < raw.length; i++) bytes[i] = raw.charCodeAt(i);
    emu.load_save(bytes);
  } catch {
    // A corrupt entry just means the game starts fresh.
  }
}

function loadRom(bytes, name) {
  persistSave();
  try {
    emu = new Emulator(bytes);
  } catch (err) {
    status(`could not load ${name}: ${err}`);
    return;
  }
  romName = emu.title().trim() || name;
  saveKey = `gba-save:${romName}`;
  restoreSave();
  if (sink) sink.port.postMessage("flush");
  status(running ? `running ${romName}` : `${romName} ready, press start`);
}

let sinceSave = 0;

function frame() {
  requestAnimationFrame(frame);
  if (!emu || !running) return;

  emu.set_keys(pressed);
  emu.step_frame();
  image.data.set(emu.framebuffer());
  ctx.putImageData(image, 0, 0);

  if (sink) {
    // Keep at most half a second of stereo audio buffered; beyond that the
    // page has drifted ahead and the extra samples would only add latency.
    const samples = emu.take_audio();
    if (backlog > audioCtx.sampleRate / 2) {
      sink.port.postMessage("flush");
    } else if (samples.length > 0) {
      sink.port.postMessage(samples, [samples.buffer]);
    }
  } else {
    emu.clear_audio();
  }

  if (++sinceSave >= 180) {
    sinceSave = 0;
    persistSave();
  }
}

async function fetchTestRom() {
  status("fetching arm.gba from jsmolka/gba-tests");
  try {
    const res = await fetch(TEST_ROM);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    loadRom(new Uint8Array(await res.arrayBuffer()), "arm.gba");
  } catch (err) {
    status(`could not fetch the test ROM: ${err}`);
  }
}

async function main() {
  await init();
  status("no ROM loaded");
  requestAnimationFrame(frame);

  startBtn.addEventListener("click", async () => {
    if (emu) await startAudio(emu.sample_rate());
    overlay.classList.add("hidden");
    running = true;
    status(romName ? `running ${romName}` : "pick a ROM to start");
  });

  demoBtn.addEventListener("click", fetchTestRom);

  romInput.addEventListener("change", async () => {
    const file = romInput.files[0];
    if (!file) return;
    loadRom(new Uint8Array(await file.arrayBuffer()), file.name);
  });

  window.addEventListener("beforeunload", persistSave);
}

main().catch((err) => status(`failed to start: ${err}`));
