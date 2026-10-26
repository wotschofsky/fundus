# Transcript Vault
<!-- impeccable:product-schema 1 -->

## Platform
adaptive

## Stack
Confirmed: Rust/Axum/SQLite backend; Svelte web library; Expo/React Native with native Expo UI on iOS and Android; SwiftUI Mac workspace. JavaScript tooling uses PNPM workspaces with one root lockfile. Apple clients share native preparation and transfer code.

## Users
Individuals maintaining private recording archives. Each person operates a separate single-owner deployment.

## Product Purpose
Preserve the native recorder workflow while moving archive storage and processing to a server. Prepare recordings losslessly on a device, upload reliably, and retrieve timestamped evidence through the Mac library, web, or a preferred MCP chat agent.

## Capabilities and Constraints
The Mac app automatically detects recorder volumes, offers per-microphone sources and safe eject, groups compatible rollover clips into sessions, and supports fragment filtering, manual merge/split, seamless preview, and frame-based trim decisions. Session decisions remain metadata locally; the server merges and trims audio before processing. Persistent source-history receipts link selections to remote entries and processing stages.

The Mac workspace includes a remote transcript library with playback and editing, People and speaker corrections, recoverable Trash, saved vault profiles, and native connection/recorder/processing/diagnostics Settings. Browser-based native OAuth grants owner access; per-origin credentials live in Keychain. MCP OAuth remains separately scoped to read-only retrieval.

Clients copy, prepare lossless FLAC where appropriate, verify, and transfer durably without modifying originals. Mac stages overlap with bounded concurrency: two copies, two preparations, three file uploads. Mobile keeps a system document-picker upload workflow with native platform controls and transfer modules; server grouping is its fallback.

The Rust server owns storage, audio assembly, transcription, enrichment, search, and recurring voice recognition. Recording context describes the audio to the transcription model and gives background for summaries; it is never evidence for a speaker identity. Provider credentials and model selection stay on the server: AssemblyAI transcribes each recording as one job (a configured key is required before startup), Groq writes titles, summaries and name evidence, and embeddings for semantic search are optional.

The server serves HTTP behind separately managed HTTPS and browser authentication. Native API routes require owner OAuth even when browser requests are trusted from the proxy. No historical migration or built-in chat in v1. No personal device information in artifacts or fixtures. Hardware compatibility is not claimed as tested without physical validation.

## Product Principles
- Preserve the original Mac recorder workflow; keep expensive processing and archive storage remote.
- Use native controls and direct source/session actions; mobile clients stay focused on upload.
- Preserve original audio quality, recording time, and evidence.
- Uncertain identities remain unnamed; manual corrections win.
- Each deployment owns its data and credentials.

## Brand Commitments
Minimalist, modern UI with a white background. Use native platform controls in installed clients. The web library uses neutral text, thin dividers, plain navigation, and readable transcript layouts.
