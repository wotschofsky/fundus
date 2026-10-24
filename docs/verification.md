# Verification

This file records checks performed against synthetic fixtures. No personal recordings are used by the automated checks. Hardware connectivity and live model-account tests are separate acceptance steps.

Passed so far:

- CLI rejects absent or whitespace-only OpenAI API keys before creating storage.
- Rust unit and HTTP integration tests: grouping, FTS escaping, matching thresholds, credential scopes/revocation, cross-origin cookie mutation rejection, tus offsets after reopening SQLite, MCP initialization and read-only tool discovery.
- OAuth security/integration checks: discovery metadata and MCP challenges, explicit same-origin consent, exact callback validation, PKCE and single-use codes, read-only MCP audience separation, confidential/public clients, expiration, refresh rotation/replay revocation, owner disconnect, and bounded public registration. Native owner tests additionally cover exact native scope/resource pairing, rejection of MCP/legacy/cookie/proxy credentials on native APIs, native CRUD aliases, refresh, and revocation. Web consent/settings build and Rust Clippy passed.
- Proxy authentication regression checks: owner browser access without a password, disabled password login, token requirements on upload/MCP routes, scope separation, rejected legacy cookies on token routes, and same-origin browser mutations.
- End-to-end mock-provider integration with real FFmpeg: verified PCM ingest, WAV/FLAC deduplication, playback preparation, diarized transcript storage, structured enrichment, FTS/embedding indexing, citations, pagination, saved-response reuse, and voice-sample reassignment after correction.
- Native C conversion: 8/16/24-bit stereo PCM matches decoded source samples; float/int32 WAV stays byte identical; originals are unchanged.
- Swift preparation tests: 16/24-bit stereo FLAC readback verification and float/int32 passthrough. System codec services require running outside a restricted filesystem sandbox.
- The restored SwiftUI Mac app compiled in production mode with Swift 6 and was packaged as an ad hoc signed `.app`. It includes grouped source sessions, merge/split/trim controls, multi-clip preview, per-microphone sources, fragment filtering, remote library/transcripts, People, Trash, saved vault profiles, and native Settings. Compilation confirms integration, not a completed visual or physical-device acceptance test.
- The integrated Mac suite passed 26 tests covering recorder discovery/eject, session grouping and frame trims, source inspection, persistent overrides, ordered remote session manifests, source-history receipt matching, and seamless source preview. Discovery/eject tests simulate startup, mount/unmount notifications, a mount during eject, failed-eject retry, copy protection, and chosen folders.
- The shared Apple module passed 11 tests: lossless preparation, queue compatibility/recovery and bounded concurrency, plus native OAuth callback validation, synthetic loopback HTTP handling, saved-profile restoration, API routing, and one serialized rotating refresh across concurrent requests. OAuth tests use in-memory credentials and mock remote networking; no real browser consent or Keychain credentials are exercised.
- The shared Apple library compiled for the iOS 17 simulator after the native Mac additions.
- Mac transfer stages are configured for at most two copies, two preparations, and three simultaneous file uploads, with stages overlapping. Synthetic queue checks cover concurrency/recovery; actual throughput and recorder eject timing still need a physical-device run.
- Mobile TypeScript check, native project generation, Android debug APK build, and unsigned iOS simulator build.
- PNPM workspace frozen installation, web/mobile checks, and matching React runtime resolution.
- Real WeSpeaker model inference from Rust-generated filterbank features: 256 finite dimensions, deterministic repeat, successful process shutdown with ort 2.0.0-rc.15 / ONNX Runtime 1.31.0.
- Web production build, synthetic browser sign-in, recording browse/detail, keyword results, authenticated audio Range (206), responsive detail at 390px with no horizontal overflow.
- Backup command refused concurrent operation while the server lock was held.
- Graceful shutdown regression: a delayed provider response is preserved, a queued next job is left untouched, and saved transcription is reused. Genuine hard-crash checkpoints still require an explicit retry.
- Container smoke test with a synthetic local provider: HTTP/tus upload, real packaged WeSpeaker inference and enrollment, hybrid retrieval, and authenticated audio Range all passed without processing warnings.
- Reusable isolated container smoke test passed on Linux AMD64 and ARM64 with a read-only root filesystem, dropped capabilities, no external provider access, and archive persistence after restart.
- Proxy-mode AMD64 container smoke test passed without owner initialization: browser access, password login disabled, anonymous upload/MCP rejection, cross-origin mutation rejection, full synthetic ingestion, and restart persistence.
- Local Mac ZIP/DMG packages built; DMG verification and SHA-256 checks passed. These are ad hoc signed local test bundles.
- GitHub workflows passed Actionlint; packaging scripts passed ShellCheck; both Compose files passed configuration validation. Release packaging refuses to run without explicit signing credentials.
- Minimal white web redesign checked at desktop and 390px widths; independent review scored source-section clarity, mobile action spacing, and design-record reconciliation resolved.

Still to exercise on physical devices/account:

- USB storage visibility for the intended DJI microphone, cables, and OS document pickers.
- Long recordings and storage pressure, real disconnects, airplane mode, background execution, force quit, relaunch, token revocation during transfer, and final server receipts on signed native apps.
- Live OpenAI access, billing, languages, diarization quality, and transcript accuracy.
- Voice thresholds on enrolled real voices, noisy/overlapping speech, and deliberate unknown speakers. Synthetic inference validates integration, not recognition accuracy.
- Hosted chat-client and native-owner OAuth sign-in against the production proxy, profile switching/relaunch with real Keychain storage, and restore into a clean server. These checks require the generic native discovery and `/api/native/*` routes to be externally reachable while enforcing owner OAuth; browser consent stays behind browser authentication.
- First hosted AMD64/ARM64 container workflow, GHCR publication and vulnerability scan; Developer ID signing/notarization and signed mobile distribution.
