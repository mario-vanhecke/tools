# ADR 0004: SharePoint via Microsoft Graph, secrets from the environment

## Status

Accepted. 2026-06-24.

## Context

"Find documents in SharePoint" is a core requirement. SharePoint Online exposes
its document libraries as **drives** through the Microsoft Graph REST API.
Reaching them needs authentication, a way to enumerate drive items, and an
incremental mode for large libraries. We must also decide where credentials
live — and a discovery tool that writes a SharePoint client secret into a SQLite
file on disk would be a liability.

## Decision

Talk to Graph directly over HTTPS using the toolkit's existing `ureq`
dependency. Authenticate with **OAuth2 client credentials** (an Azure AD app
registration), which suits an unattended CLI better than interactive/device-code
flows.

Store the non-secret selectors (`tenant_id`, `client_id`, drive/site selectors)
in the source's JSON config. **Read the client secret from an environment
variable at run time** (`CRAWL_SHAREPOINT_SECRET` by default, or whatever
`secret_env` names). The secret is never persisted in the vault.

Map strategies onto Graph: `recursive`/`shallow`/`targeted` use depth-bounded
children traversal (`/drives/{id}/root/children` → recurse); `incremental` uses
the `/delta` endpoint and persists a delta link on the source for the next run.
Use each item's `webUrl` as its canonical URI and Graph's `quickXorHash` for
change detection, so we never download file contents just to notice a change.

## Consequences

**Why:** no SDK dependency, no secret-at-rest, and a clean mapping from the
tool's strategy vocabulary to Graph's traversal and delta primitives. `webUrl`
gives stable, user-clickable document identities; the provider hash makes change
detection free.

**Cost / limits:** client-credentials requires tenant-admin consent for the
app's application permissions — an operational prerequisite, not something the
tool can paper over. The `/delta` incremental mode does not reconcile deletions
into `gone`; a periodic `recursive` crawl does. And because live Graph calls
need real credentials and network, the SharePoint transport cannot be exercised
in CI — only its pure helpers (date and path parsing, item mapping) are
unit-tested. These are accepted trade-offs of talking to a real tenant.
