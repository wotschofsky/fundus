import AVFoundation
import Observation

/// Audio playback (a transcript's archive, or a session preview): play/pause, seek, current time.
@MainActor
@Observable
final class PlayerModel {
    private(set) var isPlaying = false
    private(set) var currentSeconds = 0.0
    private(set) var durationSeconds = 0.0
    private(set) var isAvailable = false
    /// Set when the audio cannot be played, e.g. the vault stopped answering range requests.
    private(set) var error: String?

    private var player: AVPlayer?
    private var timeObserver: Any?
    private var rateObservation: NSKeyValueObservation?
    private var statusObservation: NSKeyValueObservation?
    /// Where the latest seek is going. Until it lands, the player still reports the old position;
    /// showing that would make the scrubber jump back for a moment.
    private var seekTarget: Double?

    func load(_ url: URL?, duration: Double) {
        guard let url, FileManager.default.fileExists(atPath: url.path) else {
            unload()
            durationSeconds = duration
            return
        }
        load(AVPlayerItem(url: url), duration: duration)
    }

    func load(_ item: AVPlayerItem, duration: Double) {
        unload()
        durationSeconds = duration
        let player = AVPlayer(playerItem: item)
        self.player = player
        isAvailable = true
        timeObserver = player.addPeriodicTimeObserver(forInterval: CMTime(value: 1, timescale: 4), queue: .main) { [weak self] time in
            MainActor.assumeIsolated {
                guard let self, self.seekTarget == nil else { return }
                self.currentSeconds = time.seconds
            }
        }
        rateObservation = player.observe(\.rate) { [weak self] player, _ in
            Task { @MainActor in self?.isPlaying = player.rate != 0 }
        }
        statusObservation = item.observe(\.status) { [weak self] item, _ in
            let message = item.status == .failed ? "Could not play the audio: \(item.error?.localizedDescription ?? "unknown error")" : nil
            Task { @MainActor in self?.error = message }
        }
    }

    func unload() {
        if let timeObserver { player?.removeTimeObserver(timeObserver) }
        timeObserver = nil
        rateObservation = nil
        statusObservation = nil
        error = nil
        player?.pause()
        player = nil
        seekTarget = nil
        isAvailable = false
        isPlaying = false
        currentSeconds = 0
    }

    func togglePlayback() {
        guard let player else { return }
        if player.rate != 0 { player.pause() } else { player.play() }
    }

    func seek(to seconds: Double, play: Bool = false) {
        guard let player else { return }
        currentSeconds = seconds
        seekTarget = seconds
        player.seek(to: CMTime(seconds: seconds, preferredTimescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero) { [weak self] _ in
            // A seek replaced by a newer one also completes (unfinished); only the latest one clears the target.
            Task { @MainActor in
                guard let self, self.seekTarget == seconds else { return }
                self.seekTarget = nil
            }
        }
        if play { player.play() }
    }
}
