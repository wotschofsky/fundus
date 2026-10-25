import OSLog

enum Log {
    static let audio = Logger(subsystem: "TranscriptVault", category: "audio")
    static let vault = Logger(subsystem: "TranscriptVault", category: "vault")
}
