import AVFoundation
import Observation

/// File extensions shown as sounds rather than images. Anki's `[sound:…]` also takes
/// video files, of which only the audio is played.
let soundExtensions: Set<String> = [
    "mp3", "m4a", "m4b", "aac", "wav", "aif", "aiff", "caf", "flac",
    "ogg", "oga", "opus", "mp4", "m4v", "mov", "webm",
]

func isSound(_ path: String) -> Bool {
    soundExtensions.contains((path as NSString).pathExtension.lowercased())
}

/// Plays card sounds one after another. Starting a new list stops the current one.
@Observable
@MainActor
final class SoundPlayer: NSObject {
    /// The sound playing or loading.
    private(set) var current: URL?
    /// Sounds that could not be read or decoded.
    private(set) var unplayable: Set<URL> = []

    private var queue: [URL] = []
    private var player: AVAudioPlayer?
    private var loading: Task<Void, Never>?

    func play(_ urls: [URL]) {
        stop()
        queue = urls
        playNext()
    }

    func stop() {
        loading?.cancel()
        loading = nil
        player?.stop()
        player = nil
        queue = []
        current = nil
    }

    private func playNext() {
        guard !queue.isEmpty else {
            current = nil
            return
        }
        let url = queue.removeFirst()
        current = url
        loading = Task {
            let data: Data? = if url.isFileURL {
                await Task.detached { try? Data(contentsOf: url) }.value
            } else {
                try? await URLSession.shared.data(from: url).0
            }
            guard !Task.isCancelled else { return }
            guard let data, let player = try? AVAudioPlayer(data: data), player.play() else {
                unplayable.insert(url)
                playNext()
                return
            }
            player.delegate = self
            self.player = player
        }
    }
}

extension SoundPlayer: AVAudioPlayerDelegate {
    nonisolated func audioPlayerDidFinishPlaying(_ player: AVAudioPlayer, successfully flag: Bool) {
        let finished = ObjectIdentifier(player)
        Task { @MainActor in
            // A sound stopped in favour of another one also ends up here.
            guard self.player.map(ObjectIdentifier.init) == finished else { return }
            self.player = nil
            self.playNext()
        }
    }
}
