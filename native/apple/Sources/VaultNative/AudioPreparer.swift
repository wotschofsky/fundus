import AVFoundation
import CryptoKit
import Foundation

public enum PreparationError: LocalizedError {
    case invalidAudio, verificationFailed, cancelled
    public var errorDescription: String? { switch self { case .invalidAudio: "The recording is unreadable or its format is unsupported."; case .verificationFailed: "Prepared audio did not match the source. The original is unchanged."; case .cancelled: "Preparation paused. Select the source again to retry." } }
}
public enum AudioPreparer {
    public static func prepare(_ source: URL, in folder: URL, progress: @escaping @Sendable (Double) -> Void) throws -> URL {
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        let ext = source.pathExtension.lowercased()
        let file = try AVAudioFile(forReading: source)
        guard file.length > 0 else { throw PreparationError.invalidAudio }
        let description = file.fileFormat.streamDescription.pointee
        let pcm = description.mFormatID == kAudioFormatLinearPCM
        let isFloat = description.mFormatFlags & kAudioFormatFlagIsFloat != 0
        let bits = Int(description.mBitsPerChannel)
        let canCompress = pcm && !isFloat && bits > 0 && bits <= 24
        let output = folder.appendingPathComponent(UUID().uuidString).appendingPathExtension(canCompress ? "flac" : ext)
        let partial = output.deletingPathExtension().appendingPathExtension("partial." + output.pathExtension)
        var operation = "prepare"
        do {
            if !canCompress {
                try copyAndVerify(source, partial, progress: progress)
            } else {
                let common: AVAudioCommonFormat = bits <= 16 ? .pcmFormatInt16 : .pcmFormatInt32
                operation = "open source PCM"
                let input = try AVAudioFile(forReading: source, commonFormat: common, interleaved: true)
                guard let buffer = AVAudioPCMBuffer(pcmFormat: input.processingFormat, frameCapacity: 65_536) else { throw PreparationError.invalidAudio }
                let settings: [String: Any] = [AVFormatIDKey: kAudioFormatFLAC, AVSampleRateKey: description.mSampleRate, AVNumberOfChannelsKey: Int(description.mChannelsPerFrame), AVEncoderBitDepthHintKey: bits]
                var expected = SHA256()
                try autoreleasepool {
                    operation = "create FLAC encoder"
                    let writer = try AVAudioFile(forWriting: partial, settings: settings.merging([AVAudioFileTypeKey: kAudioFileFLACType]) { _, new in new }, commonFormat: common, interleaved: true)
                    defer { if #available(iOS 18.0, macOS 15.0, *) { writer.close() } }
                    while input.framePosition < input.length {
                        if Task.isCancelled { throw PreparationError.cancelled }
                        try input.read(into: buffer)
                        guard buffer.frameLength > 0 else { throw PreparationError.invalidAudio }
                        update(&expected, buffer)
                        operation = "encode FLAC frames"
                        try writer.write(from: buffer)
                        progress(Double(input.framePosition) / Double(input.length) * 0.6)
                    }
                }
                operation = "decode prepared FLAC"
                let check = try AVAudioFile(forReading: partial, commonFormat: common, interleaved: true)
                guard check.length == input.length else { throw PreparationError.verificationFailed }
                var actual = SHA256()
                while check.framePosition < check.length {
                    if Task.isCancelled { throw PreparationError.cancelled }
                    try check.read(into: buffer)
                    guard buffer.frameLength > 0 else { throw PreparationError.verificationFailed }
                    update(&actual, buffer)
                    progress(0.6 + Double(check.framePosition) / Double(check.length) * 0.4)
                }
                guard expected.finalize() == actual.finalize() else { throw PreparationError.verificationFailed }
            }
            try FileManager.default.moveItem(at: partial, to: output)
            progress(1)
            return output
        } catch {
            try? FileManager.default.removeItem(at: partial)
            throw NSError(domain: "VaultAudio", code: (error as NSError).code, userInfo: [NSLocalizedDescriptionKey: "Could not \(operation): \(error.localizedDescription)"])
        }
    }
    private static func update(_ hash: inout SHA256, _ buffer: AVAudioPCMBuffer) {
        let audio = buffer.audioBufferList.pointee.mBuffers
        if let data = audio.mData {
            hash.update(bufferPointer: UnsafeRawBufferPointer(start: data, count: Int(buffer.frameLength) * Int(buffer.format.streamDescription.pointee.mBytesPerFrame)))
        }
    }
    private static func copyAndVerify(_ source: URL, _ destination: URL, progress: @escaping @Sendable (Double) -> Void) throws {
        let input = try FileHandle(forReadingFrom: source)
        defer { try? input.close() }
        let total = try source.resourceValues(forKeys: [.fileSizeKey]).fileSize ?? 1
        FileManager.default.createFile(atPath: destination.path, contents: nil)
        let output = try FileHandle(forWritingTo: destination)
        var expected = SHA256(), count = 0
        do {
            while let chunk = try input.read(upToCount: 65_536), !chunk.isEmpty {
                if Task.isCancelled { throw PreparationError.cancelled }
                expected.update(data: chunk); try output.write(contentsOf: chunk); count += chunk.count
                progress(Double(count) / Double(max(total, 1)) * 0.6)
            }
            try output.synchronize(); try output.close()
        } catch { try? output.close(); throw error }
        let check = try FileHandle(forReadingFrom: destination)
        defer { try? check.close() }
        var actual = SHA256()
        while let chunk = try check.read(upToCount: 65_536), !chunk.isEmpty { actual.update(data: chunk) }
        guard expected.finalize() == actual.finalize() else { throw PreparationError.verificationFailed }
    }
}
