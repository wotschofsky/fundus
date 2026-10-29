import OSLog

enum Log {
    static let app = Logger(subsystem: "Fundus", category: "app")
    static let audio = Logger(subsystem: "Fundus", category: "audio")
    static let vault = Logger(subsystem: "Fundus", category: "vault")
}
