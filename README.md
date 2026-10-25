# Transcript Vault

A private recording archive: native apps prepare and upload audio, a Rust server runs the processing pipeline, and a web library or any compatible MCP agent retrieves the results. Each person runs their own deployment.

## Run the server

The server accepts **HTTP only**. Put your existing HTTPS reverse proxy in front of it. Set `VAULT_PUBLIC_URL` to that external HTTPS origin; this controls browser-origin checks, secure cookies, and citation URLs.

```sh
cp .env.example .env
# Edit .env: external URL, AssemblyAI and Groq API keys, recorder timezone.
docker compose build
python3 -c 'import getpass; print(getpass.getpass("Vault password: "))' | docker compose run --rm -T server init
docker compose up -d
```

Open your external URL and sign in. Connect the Mac app and compatible chat agents through browser-based OAuth, without copying tokens. Mobile upload clients use **upload** tokens created under Settings; **read** tokens remain available for older MCP clients. The owner password requires at least 12 characters. Upload tokens cannot read transcripts; read tokens cannot mutate the archive. Revocation takes effect immediately. Tokens are displayed once; the database stores hashes. The owner password is Argon2 hashed.

If an authenticating reverse proxy already protects the browser UI, set `VAULT_AUTH_MODE=proxy` and skip `init`. Browser requests reaching the server are then trusted as the owner, with no second password or app-level sign-out. No provider-specific headers or identity validation are required. The default, `VAULT_AUTH_MODE=password`, retains standalone password authentication.

In proxy mode, protect every browser route at the proxy and prevent direct access to the origin. `/mcp`, `/mcp/*`, `/api/v1/imports`, `/api/v1/imports/*`, `/api/v1/uploads` and `/api/v1/uploads/*` bypass proxy login but require scoped bearer tokens. OAuth clients also need public access to these exact discovery and protocol endpoints:

- `/.well-known/oauth-protected-resource`
- `/.well-known/oauth-protected-resource/mcp`
- `/.well-known/oauth-protected-resource/api/native`
- `/.well-known/oauth-authorization-server`
- `/oauth/register`
- `/oauth/token`
- `/oauth/revoke`

Also make `/api/native/*` externally reachable. These routes require a valid owner OAuth bearer token; they never trust proxy authentication or browser cookies. Native owner grants use scope `vault:owner` and resource `<public-origin>/api/native`; MCP read grants cannot authorize native routes, and owner grants cannot authorize MCP.

Allow GET/OPTIONS for metadata and POST/OPTIONS for registration, token exchange, and revocation. Preserve the native API’s HTTP methods, including PATCH/HEAD/OPTIONS. Keep `/oauth/authorize` and every `/api/v1/oauth/` route behind browser authentication. Browser mutations, including consent, require an `Origin` matching `VAULT_PUBLIC_URL`.

The included Compose file binds port 3000 on loopback. If your reverse proxy runs in another container, connect them on a private Docker network and point it at `server:3000`. Forward `/`, `/api/`, and `/mcp` unchanged; preserve the public Host and Origin headers. Allow PATCH/HEAD/OPTIONS, bodies of at least 8 MiB, and streaming MCP responses. Do not buffer SSE indefinitely. No TLS certificates belong in this application.

For a local binary, install Rust 1.94+, Node 26, PNPM 10.34.5, FFmpeg/ffprobe, then:

```sh
pnpm install --frozen-lockfile
pnpm build
cargo build --locked --release
# Export configuration from .env in your shell (the binary does not auto-load it).
python3 -c 'import getpass; print(getpass.getpass("Vault password: "))' | target/release/transcript-vault init
target/release/transcript-vault serve
```

Default local bind is `127.0.0.1:3000`, public URL `http://localhost:3000`, and data directory `data`. Build the web app before compiling Rust: its assets are embedded in the binary. `ASSEMBLYAI_API_KEY` is required: the server refuses to start when it is missing or blank. `GROQ_API_KEY` is recommended (without it, recordings get no titles, summaries or name evidence and show a warning); `OPENAI_API_KEY` is optional and only enables semantic search. Initialization, backup, and other administrative commands do not need an API key.

## Upload apps

For prebuilt GHCR deployment, multi-platform CI and versioned native bundles, see
[packaging and releases](docs/packaging.md). `compose.image.yaml` runs a published
image without compiling on the server; `compose.yaml` builds from source.

**Mac:** `scripts/build-macos.sh` builds `build/Transcript Vault.app`; `scripts/package-macos.sh` also creates versioned ZIP/DMG bundles and checksums. The app runs on macOS 15+ and is built with Xcode 26 or newer, which supplies the Liquid Glass design. The build signs with `MACOS_SIGNING_IDENTITY`, otherwise with the first Developer ID Application or Apple Development identity in your keychain, and only falls back to ad hoc signing without one. Keep the identity stable: Keychain access to saved connections is tied to it, so every ad hoc build asks again. Connect your server in Settings (⌘,) and approve owner access in your browser. Credentials are stored per vault in Keychain; saved vaults are listed at the bottom of the sidebar. A manual upload-token fallback is available for older servers. Use `scripts/package-macos.sh --release` with your own signing/notarization credentials for distribution.

The Mac app keeps the Mic Ingest layout: an Import screen for the connected recorder or a chosen folder, and the vault's transcripts in the sidebar. It preserves the recorder workflow: automatic discovery at startup and mount, per-microphone tabs, grouped rollover sessions, short-fragment filtering, manual merge/split, seamless multi-clip preview, frame-based trim selection, and safe eject after copying. Individual files can be imported with ⌘O. Grouping and trim decisions are metadata sent with the import; the server performs the actual merge, trim, transcription, and indexing. Source-history receipts show prior imports and remote processing progress. Transcripts stream their audio from the vault as you listen, follow processing while it runs, and support title/label/date edits, people and speaker corrections, Markdown export, and recoverable Trash. Settings cover the vault connection, recorder clock, recording context, people, and voice-match thresholds. The app polls the vault only while something is uploading or processing, and otherwise refreshes when it becomes active.

**iOS and Android:** `apps/mobile` uses Expo SDK 58, React Native, and **Expo UI’s SwiftUI / Jetpack Compose controls**. It requires a custom native build, not Expo Go:

```sh
scripts/fetch-native-deps.sh
cd apps/mobile
pnpm install --frozen-lockfile
pnpm exec expo run:ios
# or
pnpm exec expo run:android
```

iOS 17+ / Android 8+ are targeted. Connect using the externally served HTTPS URL and an upload token. Use the recorder’s configured timezone. Select multiple audio files from connected storage or any document provider. USB visibility is controlled by the recorder, cable, OS, and file provider; the app makes no undocumented USB protocol assumptions. DJI Mic Mini 2S is an intended compatibility target; physical connectivity has not yet been validated.

The apps copy selected files into private app storage, convert **integer PCM WAV up to 24 bits to full-quality FLAC**, then decode and verify it before upload. Float and 32-bit integer WAV stay byte identical; FLAC, MP3, and M4A pass through. Original recorder files are never modified. Mobile staging can temporarily hold the document picker’s cache copy, the app’s source copy, and the prepared file; allow up to three times the selected size. Document-provider caches are managed by the OS. Keep the app open while copying/preparing. Transfers use bounded 8 MiB tus chunks, with server-confirmed offsets after interruption. iOS uses background URLSession uploads; Android uses a data-sync foreground service. Operating systems can stop work, especially after force quit; reopen and Resume. Local copies remain until you clear the queue, including after a verified server receipt.

One selected batch runs at a time. On Mac, copying, FLAC preparation, and uploads overlap with limits of two copy operations, two preparation operations, and three file uploads. A batch is finalized only after all files arrive. Mac imports include the selected session order and trims; picker-based mobile imports use server-side grouping. Clearing a local queue does not delete a partially received server import; unfinished imports are retained for manual recovery. No recorder files are deleted, and clients do not transcribe recordings.

## Pipeline and retrieval

SQLite uses WAL, foreign keys, migrations, and durable commits. Audio stays in server-owned paths. The pipeline verifies decoded PCM, deduplicates equivalent WAV/FLAC audio, groups compatible contiguous DJI chunks, prepares browser playback (playable while the rest is still processing), transcribes the whole recording as one AssemblyAI job so speaker labels stay consistent from start to end, extracts a title/summary, and indexes passages. Two recordings are processed at a time. Generic audio with no reliable timestamp stays undated until edited in the web library. Ambiguous timezone/DST timestamps are not guessed. Transmitter streams remain separate; simultaneous transmitter tracks are not aligned or mixed.

Every transcription job gets a durable checkpoint before submission: the AssemblyAI upload URL is saved before the paid request, its job ID as soon as it exists, and the completed result once it arrives, so a recording is paid for once. Retries, pauses and restarts poll the saved job or reuse its result; when a creation request's answer was lost (network failure, crash), the job is found again by its upload URL in AssemblyAI's transcript list instead of being submitted twice. A job AssemblyAI reports as failed, or no longer has, is submitted again only on Retry. Once the result is saved, the job and its uploaded audio are deleted at AssemblyAI. One job covers at most 10 hours; split longer sessions before importing. Summaries and embeddings may be requested again during an explicit retry. Missing optional processing creates visible warnings rather than hiding an otherwise readable transcript. No live provider call is needed for the automated tests.

Search uses SQLite FTS5 keyword ranking, fused with passage embeddings when `OPENAI_API_KEY` is set (any OpenAI-compatible embeddings endpoint via `OPENAI_BASE_URL`); without it, keyword search is the complete design and is not reported as partial. Vectors are stored in SQLite and compared in Rust: no vector service. Results include source entry, excerpt, audio times, and links. Coverage flags expose unfinished entries and missing embeddings. Transcript reads are paginated; consumers must follow `next_cursor` for full coverage.

The web and native Mac libraries support playback, time-linked transcripts, labels, titles, recording-date correction, people, manual speaker correction, processing errors, and retries. The Mac library also provides recoverable Trash and permanent deletion. Recording context describes the audio to AssemblyAI (vocabulary, languages, setting) and gives background for summaries; it is never evidence for a speaker identity. Built-in chat and historic filesystem-vault migration are deferred; chat happens through your preferred MCP client.

### Models

- Speaker-labeled transcription: AssemblyAI pre-recorded API, `universal-3-5-pro` (`VAULT_TRANSCRIPTION_MODEL`, comma-separated), code-switching between `de,en` (`VAULT_TRANSCRIPTION_LANGUAGES`), speaker labels on. The server uploads a 16 kHz mono FLAC rendition; the archive stays full quality. The vault's recording context is sent as AssemblyAI's `prompt`, which describes the audio.
- Titles, summaries, explicit name evidence: Groq chat completions with strict JSON Schema output, `openai/gpt-oss-120b` (`VAULT_TEXT_MODEL`). Long transcripts are summarized in sections of about 12,000 characters and then combined; rate limits are retried.
- Passage embeddings (optional): `text-embedding-3-small` (`VAULT_EMBEDDING_MODEL`).
- Recurring voice recognition: local WeSpeaker ResNet34-LM ONNX, separate from text embeddings. Docker includes the pinned model and ONNX Runtime 1.31.0. For a local binary, run `scripts/fetch-speaker-model.sh`, set `VAULT_VOICE_MODEL` to its output and `ORT_DYLIB_PATH` to your runtime library.

All provider calls live in the isolated `provider.rs` adapter. Model IDs and the base URLs (`ASSEMBLYAI_BASE_URL`, `GROQ_BASE_URL`, `OPENAI_BASE_URL`) are configurable. Entries transcribed by the former OpenAI pipeline keep their transcripts; an explicit Retry of such an entry transcribes it again with AssemblyAI and replaces its per-part speaker labels.

Speaker profiles require manual identity confirmation. Clean, non-overlapping excerpts form acoustic probes; automatic matches require both a strong score and separation from the runner-up. Uncertain speakers stay unnamed. Textual name findings are reviewable evidence, never automatic voice enrollment. Correcting or deleting a person/entry removes its contributed voice samples. The initial thresholds are conservative defaults, not a calibrated accuracy guarantee.

### MCP

Connect an MCP client using Streamable HTTP at `https://your-vault.example/mcp` and browser-based OAuth. Five read-only tools are exposed:

| Tool | Purpose |
| --- | --- |
| `list_entries` | Chronological browse with dates, labels, person, and processing status |
| `search` | Ranked transcript passages with time-based citations and coverage |
| `get_entries` | Entry metadata, sources, and speakers by ID |
| `read_transcript` | Bounded, paginated transcript evidence |
| `list_labels` | Discover available labels and counts |

Add `https://your-vault.example/mcp` in your chat app’s MCP or connector settings, then choose its **Connect** or **Sign in** action. Your browser opens the vault, where you sign in and explicitly approve **read access** for that app. The client discovers the OAuth endpoints and registers automatically; no token copying is needed.

OAuth connections can search and read the archive through MCP. They cannot upload, edit, delete, or administer the vault. Access tokens expire after one hour; clients renew them using rotating refresh tokens. **Settings → Connect a chat app** lists approved connections and lets you disconnect one immediately. Reusing an old refresh token revokes its connection.

The server supports OAuth authorization code flow with PKCE S256, dynamic client registration, public clients, and `client_secret_basic` / `client_secret_post` clients. Client ID Metadata Documents are not advertised; clients must support dynamic registration. The public server URL must use HTTPS outside local development.

For older clients without OAuth, create a **read** token under Settings and use their Bearer-token setting. For example, a Codex configuration can read a manually provisioned token from the client environment:

```toml
[mcp_servers.transcript_vault]
url = "https://your-vault.example/mcp"
bearer_token_env_var = "VAULT_READ_TOKEN"
```

## Data, deletion, and backup

Audio archives, browser playback, raw transcription responses, summaries, transcripts, labels, embeddings, speaker probes, and voice samples live in `VAULT_DATA_DIR`. Processing sends a 16 kHz rendition of the audio to AssemblyAI, transcript text to Groq, and, if semantic search is enabled, passage text to the embeddings endpoint; voice embeddings never leave the server. Self-hosting the archive does not make transcription or summarization local. There is no analytics SDK. Protect server disks and backups using your own encryption and access controls.

Stop the server before backup; the CLI enforces an exclusive server lock:

On a normal shutdown, the worker finishes its current job and saves the results before exiting. The included Compose files allow up to 30 minutes for this; keep the supervisor's stop timeout aligned. A job waiting for AssemblyAI stops waiting immediately and resumes polling the same job after restart. No new upload or paid request starts during shutdown. After a forced stop or crash, saved jobs and results are reused and an unanswered request is resolved by its upload URL.

```sh
docker compose stop server
docker compose run --rm -v "$PWD/backups:/backup" server backup /backup/first
# Ensure the mounted backup directory is writable by container UID 10001.
docker compose up -d
```

Or locally: `transcript-vault backup /path/to/new-backup`. Backups include a SQLite snapshot and archived audio directories, including unfinished uploads. Restore into an empty data directory while the server is stopped; preserve file ownership, then start the same or a newer compatible binary. Backups contain credential hashes and password hashes: treat them as private.

Moving an entry to Trash hides it from the library and MCP retrieval until restored. Permanent deletion removes its transcript, indexes, voice contributions, owned source blobs, and working/playback files. It does not touch originals on the recorder or copies in older backups. Unfinished/orphan uploads are retained; no automatic age-based deletion is enabled.

## Checks

```sh
pnpm build
pnpm --dir apps/mobile check
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
swift test --package-path native/apple
swift test --package-path apps/macos
swift build --package-path apps/macos
scripts/test-native-codec.sh
```

The server integration tests run local AssemblyAI, Groq and embeddings mocks and real FFmpeg, covering audio preparation through searchable transcripts, checkpoint reuse across pause/shutdown/restart, failed-job resubmission, keyword-only search without embeddings, PCM deduplication, sample-exact trims, and speaker correction. Additional tests cover scopes/revocation, browser origin checks, resumable offsets after reopening SQLite, and actual MCP initialization/tool discovery. Native tests verify lossless integer conversion and byte-preserving float/int32 passthrough. A separate ignored Rust test can validate inference with the pinned voice model: set `VAULT_VOICE_MODEL` and `ORT_DYLIB_PATH`, then `cargo test pinned_model_runs -- --ignored`.

See [verification notes](docs/verification.md) for what has and has not been exercised. Real microphone USB access, signed-device background execution, live provider behavior, and accuracy on your own voices still require device/account validation.
