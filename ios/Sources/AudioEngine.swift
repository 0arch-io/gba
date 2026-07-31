import AVFoundation

/// Pull-model audio: the source node asks for samples, and the emulator is
/// pumped exactly as fast as audio is consumed — the same audio-clock pacing
/// as the desktop frontend, with the OS doing the pacing for us.
final class AudioEngine {
    private let engine = AVAudioEngine()
    private var node: AVAudioSourceNode?
    private var pending: [Float] = []
    private weak var core: EmulatorCore?
    var running = true

    init(core: EmulatorCore) {
        self.core = core
        let format = AVAudioFormat(standardFormatWithSampleRate: 44100, channels: 2)!
        let node = AVAudioSourceNode(format: format) { [weak self] _, _, frameCount, audioBufferList -> OSStatus in
            guard let self else { return noErr }
            let abl = UnsafeMutableAudioBufferListPointer(audioBufferList)
            let needed = Int(frameCount) * 2
            var scratch = [Float](repeating: 0, count: 4096)
            while self.pending.count < needed, self.running, let core = self.core {
                let n = core.pump(into: &scratch, max: scratch.count)
                if n == 0 { break }
                self.pending.append(contentsOf: scratch[..<n])
            }
            let left = abl[0].mData!.assumingMemoryBound(to: Float.self)
            let right = abl.count > 1 ? abl[1].mData!.assumingMemoryBound(to: Float.self) : left
            for i in 0..<Int(frameCount) {
                if self.running && i * 2 + 1 < self.pending.count {
                    left[i] = self.pending[i * 2]
                    right[i] = self.pending[i * 2 + 1]
                } else {
                    left[i] = 0
                    right[i] = 0
                }
            }
            let consumed = min(needed, self.pending.count)
            self.pending.removeFirst(consumed)
            return noErr
        }
        self.node = node
        engine.attach(node)
        engine.connect(node, to: engine.mainMixerNode, format: format)
        try? AVAudioSession.sharedInstance().setCategory(.ambient)
        try? AVAudioSession.sharedInstance().setActive(true)
        try? engine.start()
    }

    func stop() {
        running = false
        engine.stop()
    }
}
