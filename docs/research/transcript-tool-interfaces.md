# Transcript vault: comparable tool interfaces

Researched 2026-10-24. Scope: publicly documented first-party MCP interfaces and closely related APIs. No authenticated customer tools were invoked. Published documentation and vendor source establish design precedents, not a guarantee that every account exposes the same tools. Subsequent decision: Rust for the backend.

## Findings

### Granola — documented MCP tools

Relevant names: `query_granola_meetings`, `list_meeting_folders`, `list_meetings`, `get_meetings`, and `get_meeting_transcript`. Listing returns identifying metadata; `get_meetings` reads notes and summaries; raw transcripts have a separate tool. Paid plans support folder filtering. The query tool delegates to Granola Chat. The documentation also lists attachment, account, and workspace operations. It does not publish complete argument schemas for these retrieval tools, so their signatures should not be invented.

Source: [Granola MCP documentation](https://docs.granola.ai/help-center/sharing/integrations/mcp).

### Fireflies — documented MCP names and parameters

`fireflies_get_transcripts` accepts keyword, title/sentences/all scope, dates, participants, organizers, channel, limit, and offset; results omit detailed transcript text. `fireflies_get_transcript(transcriptId)` returns attributed sentences with timestamp ranges. `fireflies_get_summary(transcriptId)` reads generated summaries separately. `fireflies_search(query, format)` offers query grammar; `fireflies_fetch(id)` returns combined meeting data. These last two are experimental. Folder discovery, renaming, moving, sharing, and soundbite tools are also documented. Connection methods can expose different subsets.

Sources: [MCP parameter reference](https://docs.fireflies.ai/mcp-tools/overview), [connection-dependent availability](https://fireflies.ai/blog/fireflies-mcp-server).

### Read AI — documented MCP functions

`list_meetings` supports dates and pagination; `get_meeting_by_id` reads details. `search_meeting_agent` accepts a natural-language query, optional dates, result limit, and execution time budget. It returns candidate meetings with relevance explanations and supporting excerpts, leaving the calling assistant to answer. Documentation describes slower, agentic search and approximately twelve months of searchable history. `list_folders`, `get_folder`, and folder item operations support organization. Recording-agent and sharing functions are separate actions.

Source: [Read AI MCP reference](https://support.read.ai/hc/en-us/articles/49381158409491-MCP-Server).

### Omi — hosted MCP schemas verified in vendor source

`get_conversations` lists small cards using dates, categories, and pagination. `search_conversations` accepts query/date bounds and returns cards plus timestamped, speaker-linked snippets and search-coverage information. `get_conversation_by_id` bounds transcript output using segment/character budgets and reports truncation. `get_conversations_by_ids` supports batch reads. Durable memories have separate retrieval and mutation tools. Hosted and local MCP implementations differ; these findings concern the hosted registry.

Source: [Omi hosted registry](https://github.com/BasedHardware/omi/blob/main/backend/utils/mcp_server/registry.py).

### Limitless — REST operations; MCP schema not verified

The developer page advertises MCP but does not enumerate its schemas. Its REST `GET /v1/lifelogs` supports chronological retrieval and hybrid keyword/semantic search, dates, timezone, starred state, and content-inclusion flags. Ordinary listing supports cursors; search does not. `GET /v1/lifelogs/{id}` reads an entry. `GET /v1/download-audio` reads a time range. Deletion is separate. These endpoint paths must not be presented as verified MCP function names. No arbitrary-label mutation was established in this reference.

Source: [Limitless developer API](https://www.limitless.ai/developers/docs/api). This page was retrieved by the background research pass; a subsequent main-pass fetch timed out.

## Recommended interface for this project

The following is a proposed design, not a claim about competitor functionality. Implement ordinary application functions once and expose thin MCP and HTTP adapters. The web chat can call those functions directly; external agents can use MCP. Retrieval returns evidence for the calling agent to interpret.

| Proposed tool | Responsibility | Principal inputs |
|---|---|---|
| `list_entries` | Browse matching recordings in a predictable order, including those not indexed for search | Recording date range, timezone, labels, optional speaker IDs, limit, cursor |
| `search` | Find relevant spoken passages, grouped or diversified across entries | Query, optional date/label/speaker filters, limit |
| `get_entries` | Read metadata, processing warnings, and available generated summaries in a bounded batch | Entry IDs |
| `read_transcript` | Read attributed original/corrected transcript segments and expand a search hit's context | Entry ID, optional audio time range, limit, cursor |
| `list_labels` | Discover actual labels and usage counts | Optional prefix, limit, cursor |

Add a separately authorized `update_entry` for explicit title changes and label additions/removals if chat-based organization is included. Return the updated metadata. Keep audio upload and long-running import controls in the web interface initially.

### Contract details that matter

- Search hits carry entry and segment IDs, title, labels, speaker, verbatim excerpt, recording clock time, audio offsets, and a canonical web URL opening that passage.
- Browse is filter-based and pageable; ranked search is not an exhaustive inventory. Report indexing coverage and degraded search modes so the agent can distinguish no match from unavailable content.
- Transcript reads report continuation and truncation explicitly. Do not silently present an initial excerpt as a complete multi-hour recording.
- Filter by recording time, not upload time. Resolve dates using a stated timezone. Use separate absolute clock times and audio offsets, retaining original chunk mappings where takes contain clock gaps.
- Summaries remain labeled as generated and point to source passages when available. Transcript corrections and manual speaker assignments survive reprocessing.
- Keep lexical and semantic retrieval behind `search`; callers need not select an index. Exact phrase behavior should be documented.
- Batch metadata reads help compare several entries without downloading every transcript. Apply total response budgets as well as per-entry limits.
- A dedicated question-answering tool can be added later, but is unnecessary for the initial interface because the calling chat agent already performs synthesis.
- Avoid exposing arbitrary SQL or filesystem layout as the retrieval contract.

### Compatibility

For ChatGPT deep research/company-knowledge compatibility, provide the documented `search(query)` and `fetch(id)` shapes through a compatibility adapter/profile. A fetchable result can represent a bounded passage with surrounding context and a link to its parent entry. Do not expose competing aliases to ordinary clients unless required by the target integration.

Sources: [OpenAI MCP compatibility](https://developers.openai.com/api/docs/mcp), [MCP tools specification](https://modelcontextprotocol.io/specification/2025-11-25/server/tools), [MCP resources specification](https://modelcontextprotocol.io/specification/2025-11-25/server/resources).

## Language note

Decision: use Rust for the backend. Both Go and Rust have official MCP SDKs: [Go SDK](https://github.com/modelcontextprotocol/go-sdk), [Rust SDK](https://github.com/modelcontextprotocol/rust-sdk). The retrieval contract is independent of that choice. Expected bottlenecks are audio processing, model calls, and retrieval quality; no performance benchmark was performed to establish a language advantage here. Client UI technology remains undecided.
