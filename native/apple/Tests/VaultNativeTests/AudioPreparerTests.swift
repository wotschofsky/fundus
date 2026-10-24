import XCTest
import AVFoundation
@testable import VaultNative
final class AudioPreparerTests: XCTestCase {
    func wav(bits: Int, floating: Bool = false, frames: Int = 4800) -> Data {
        var payload = Data()
        for i in 0..<(frames * 2) {
            if floating { var value = Float(sin(Double(i) * 0.03) * 1.2).bitPattern.littleEndian; withUnsafeBytes(of: &value) { payload.append(contentsOf: $0) } }
            else { let value = (i % 2 == 0 ? 1 : -1) * (i * 1619 % (1 << (bits - 1))); for byte in 0..<(bits / 8) { payload.append(UInt8(truncatingIfNeeded: value >> (byte * 8))) } }
        }
        var result = Data()
        func text(_ value: String) { result.append(contentsOf: value.utf8) }
        func number(_ value: Int, _ bytes: Int) { for byte in 0..<bytes { result.append(UInt8(truncatingIfNeeded: value >> (byte * 8))) } }
        text("RIFF"); number(payload.count + 36, 4); text("WAVEfmt "); number(16, 4); number(floating ? 3 : 1, 2); number(2, 2); number(48000, 4); number(48000 * 2 * bits / 8, 4); number(2 * bits / 8, 2); number(bits, 2); text("data"); number(payload.count, 4); result.append(payload); return result
    }
    func testIntegerPCMBecomesVerifiedFLAC() throws {
        for bits in [16, 24] {
            let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: folder) }
            let source = folder.appendingPathComponent("source.wav"), original = wav(bits: bits)
            try original.write(to: source)
            let output = try AudioPreparer.prepare(source, in: folder) { _ in }
            XCTAssertEqual(output.pathExtension, "flac")
            let audio = try AVAudioFile(forReading: output)
            XCTAssertEqual(audio.length, 4800)
            XCTAssertEqual(audio.fileFormat.channelCount, 2)
            XCTAssertEqual(try Data(contentsOf: source), original)
        }
    }
    func testFloatAnd32BitIntegerAreByteIdentical() throws {
        for floating in [false, true] {
            let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            defer { try? FileManager.default.removeItem(at: folder) }
            let source = folder.appendingPathComponent("source.wav"), original = wav(bits: 32, floating: floating)
            try original.write(to: source)
            let output = try AudioPreparer.prepare(source, in: folder) { _ in }
            XCTAssertEqual(output.pathExtension, "wav")
            XCTAssertEqual(try Data(contentsOf: output), original)
        }
    }
}
