# ADR 0005: SharePoint authentication modes and the REST fallback

## Status

Accepted. 2026-06-25. Extends [ADR 0004](0004-sharepoint-via-graph.md).

## Context

ADR 0004 chose Microsoft Graph with app-only client-credentials for SharePoint.
Real-tenant testing exposed two hard problems that client-credentials alone
can't solve for an end user who just wants *their own* documents:

1. **Admin consent.** Graph's SharePoint scopes (`Sites.Read.All`,
   `Files.Read.All`) are admin-consent-only in many tenants. A non-admin user
   signs in successfully but the token carries no SharePoint content scope, so
   every library read returns 403. They cannot grant the consent themselves.
2. **The default library is often empty.** Graph's `/sites/{id}/drive` returns
   the *default* "Documents" library. In practice the documents frequently live
   in a differently-named library (e.g. "Project Documents"), so even when Graph
   works it finds nothing.

The user could open the site in their browser the whole time — their delegated
access was never in question; only the *programmatic* path was blocked.

## Decision

Offer a spectrum of auth modes (config key `auth`) over two transports, and make
the **REST transport** a first-class path, not just a Graph wrapper:

| Mode | Transport | Gets a token how | When |
|---|---|---|---|
| `browser` | Graph | interactive auth-code + PKCE (Graph scopes) | tenant allows Graph |
| `device_code` | Graph | device-code (Graph scopes) | headless/SSH |
| `client_credentials` | Graph | app-only secret | unattended cron |
| `browser_rest` | **REST** | interactive auth-code for the *SharePoint resource* | Graph gated by admin consent |
| `cookie` | **REST** | the user's existing browser session cookies | nothing else works / quick pull |

The REST modes talk to SharePoint's own `/_api/web/...` endpoints — the same API
the browser uses — which authorize on the **user's** content permissions, not on
an admin-consented app permission. They:

- list **every** document library (`/_api/web/lists`, `BaseTemplate == 101`),
  skipping system libraries and the empty default, instead of only the default
  drive;
- recurse folders (`GetFolderByServerRelativeUrl(...)/Files|Folders`);
- can enumerate **all accessible sites** via the Search API (`all_sites=true`)
  and crawl each, for tenant-wide discovery.

## Consequences

**Why:** the REST path is what unblocks a real non-admin user — it reuses the
delegated access they already have. `browser_rest` is the durable form (the
interactive sign-in yields a refreshable token, the way `az login` does, with no
`az` install and no admin consent); `cookie` is the zero-prerequisite escape
hatch. Listing all libraries fixes the empty-default-library trap for every mode.

**Cost / limits:**
- `cookie` sessions expire in hours and can't refresh — fine for a one-off pull,
  not for an unattended job (that's what `client_credentials` is for).
- `browser_rest` depends on the tenant issuing a SharePoint-resource token to the
  default public client without admin consent — usually true (it's the Azure CLI
  path), but not guaranteed; a JWT `aud`/`scp` diagnostic is printed on 403 so the
  failure is precise rather than mysterious.
- Tenant-wide crawls can be huge (hundreds of sites, 100k+ documents) and hold
  all discoveries in one transaction; `sites_filter` and `max_sites` bound it.
- The live Graph/REST round-trips can't run in CI; the auth and traversal
  plumbing is exercised against mock Graph and mock REST servers, and was
  verified once end-to-end against a real tenant.
