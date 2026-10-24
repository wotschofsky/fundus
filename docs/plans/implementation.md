# Implementation contract

Rust server with SQLite and filesystem audio storage, HTTP behind external TLS. Single-owner deployments with web sessions and scoped device/MCP tokens. Expo UI native mobile uploader and SwiftUI Mac uploader prepare lossless FLAC locally, verify audio, persist queue state, and transfer via tus. Server owns DJI grouping, timestamp mapping, OpenAI transcription/enrichment, local voice matching, hybrid retrieval, web playback, and MCP. See README for current verification and operating instructions. No personal phone models belong in project artifacts. No real-hardware test claim without an actual test.

V1 excludes historical migration, a built-in chat agent, multi-user sharing, and manual audio editing.
