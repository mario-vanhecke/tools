//! SharePoint crawler via the Microsoft Graph API.
//!
//! Six authentication modes (config key `auth`):
//!   * `browser_rest` — **interactive browser sign-in for the SharePoint
//!     resource**, then its REST API (not Graph). Like `az login` built in:
//!     avoids the Graph admin-consent wall, needs no `az`, and the token
//!     auto-refreshes (unlike raw cookies). Best choice when Graph is gated.
//!   * `browser` — interactive browser sign-in (authorization code + PKCE,
//!     loopback redirect) for **Graph**. `crawl` opens your browser, you log in
//!     normally, the redirect is caught on `http://localhost:<port>`. No secret.
//!     Token + refresh token cached under `.crawl/`. Needs Graph scopes your
//!     tenant may gate behind admin consent.
//!   * `device_code` — **sign in via a code on another device** (delegated). No
//!     secret; `crawl` prints a code, you open a browser and log in once. Works
//!     over SSH/headless. Default `client_id` is the Microsoft Graph PowerShell
//!     public client, so it works with no app registration in many tenants.
//!   * `azure_cli` — reuse an existing **`az login`**. `crawl` shells out to
//!     `az account get-access-token`. Truly zero setup if you already use the
//!     Azure CLI; no app registration, no secret.
//!   * `client_credentials` — unattended app-only (an Azure AD app registration
//!     with application permission). The client **secret is read from an
//!     environment variable** and never written to the vault. Best for cron.
//!   * `cookie` — talk to SharePoint's own **REST API** (`/_api/web/...`) with
//!     the user's **browser session cookies**, no Graph token. The escape hatch
//!     when Graph access is blocked by admin-consent policy: it needs no admin,
//!     no app, no `az`. Cookies (`FedAuth`/`rtFa`) are read from an env var and
//!     expire after a few hours.
//!
//! Source config keys (JSON, set via `crawl source add ... --set k=v`):
//!   auth            browser_rest | browser | device_code | azure_cli | client_credentials | cookie (default device_code)
//!   tenant_id       Azure AD tenant id or domain (default: organizations / common)
//!   client_id       app registration (client) id (browser/device_code/client_credentials)
//!   secret_env      env var holding the client secret (client_credentials)
//!   drive_id        target drive id, OR
//!   site_hostname + site_path  e.g. "contoso.sharepoint.com" + "/sites/Marketing"
//!   folder_path     start under this folder, relative to the drive root (optional)
//!   graph_base      Graph API base override (sovereign/GCC-High clouds, tests)
//!   oauth_base      OAuth2 authority base override (browser/device_code/client_credentials)
//!   az_path         path to the `az` binary (azure_cli; default "az")
//!   scopes          delegated scopes for interactive sign-in (default: read sites)
//!   cookie_env      env var holding the session cookie string (cookie; default CRAWL_SHAREPOINT_COOKIE)
//!   all_sites       (REST auth) discover every accessible site via Search and crawl each
//!   sites_query     Search query for `all_sites` (default contentclass:STS_Site)
//!   sites_filter    only crawl discovered site URLs containing this substring
//!   max_sites       cap on sites a tenant-wide crawl visits (default 50)
//!
//! Strategies map to Graph as: recursive/shallow/targeted → children traversal
//! (depth-bounded), incremental → the `/delta` endpoint with a stored delta
//! link. Incremental does not detect deletions; re-run a `recursive` crawl to
//! reconcile removals into `gone`.
//!
//! Note: live Graph calls need network + real credentials; the pure helpers and
//! the auth/transport plumbing are exercised against a mock Graph server, the
//! interactive sign-in itself against a real tenant.

use super::{CrawlContext, CrawlStats, Crawler, DiscoveredItem};
use crate::error::{Error, Result};
use crate::source::{Source, Strategy};
use crate::SourceKind;
use serde_json::{json, Value};
use std::path::Path;

const GRAPH: &str = "https://graph.microsoft.com/v1.0";
const DEFAULT_SECRET_ENV: &str = "CRAWL_SHAREPOINT_SECRET";
/// Microsoft Graph PowerShell — a Microsoft first-party multi-tenant *public*
/// client that supports the device-code flow with delegated Graph scopes. Used
/// as the default so interactive sign-in works without registering an app.
const DEFAULT_PUBLIC_CLIENT_ID: &str = "14d82eec-204b-4c2f-b7e8-296a70dab67e";
/// Azure CLI — a Microsoft first-party public client that allows loopback
/// redirect and can obtain tokens for the SharePoint *resource* (the same path
/// `az login` uses). Default client for `browser_rest`, so signing in for
/// SharePoint works without `az` installed and without admin-consented Graph.
const AZURE_CLI_CLIENT_ID: &str = "04b07795-8ddb-461a-bbee-02f9e1bf7b46";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

pub struct SharePointCrawler;

impl Crawler for SharePointCrawler {
    fn kind(&self) -> SourceKind {
        SourceKind::SharePoint
    }

    fn crawl(
        &self,
        source: &Source,
        ctx: &CrawlContext,
        sink: &mut dyn FnMut(DiscoveredItem),
    ) -> Result<CrawlStats> {
        let cfg = GraphConfig::from_source(source)?;

        // Cookie auth talks to SharePoint's own REST API (not Graph), using the
        // user's browser session — no token, no admin-consented Graph scopes.
        if cfg.auth == AuthMode::Cookie {
            return rest::crawl_with_cookie(&cfg, ctx, sink);
        }

        // browser_rest signs in interactively for the SharePoint *resource*, then
        // uses the same REST API as cookie mode — durable, no Graph admin wall.
        if cfg.auth == AuthMode::BrowserRest {
            let token = token_auth_code(&cfg, ctx.cache_dir, &source.name)?;
            return rest::crawl_with_token(&cfg, ctx, &token, sink).map_err(|e| {
                // On an access failure, surface what the token actually carried —
                // turns "403" into a precise audience/scope diagnosis.
                match token_diagnostics(&token) {
                    Some(diag) => Error::SharePoint(format!("{e}\n  {diag}")),
                    None => e,
                }
            });
        }

        let token = fetch_token(&cfg, ctx.cache_dir, &source.name)?;
        let drives = resolve_drives(&token, &cfg)?;

        let mut stats = CrawlStats::default();
        // Crawl each document library. When more than one, prefix paths with the
        // library name so they stay distinct and you can see where each came from.
        let multi = drives.len() > 1;
        if multi {
            let names: Vec<&str> = drives
                .iter()
                .map(|d| d.name.as_deref().unwrap_or("?"))
                .collect();
            eprintln!(
                "Crawling {} document libraries: {}",
                drives.len(),
                names.join(", ")
            );
        }

        // Delta (incremental) only makes sense against a single, explicitly
        // pinned drive; otherwise every library gets a full traversal.
        let use_delta =
            source.strategy == Strategy::Incremental && cfg.drive_id.is_some() && drives.len() == 1;

        for drive in &drives {
            let prefix = if multi { drive.name.as_deref() } else { None };
            let result = if use_delta {
                crawl_delta(
                    &cfg.graph_base,
                    &token,
                    &drive.id,
                    cfg.delta_link.as_deref(),
                    sink,
                    &mut stats,
                )
                .map(|new_link| {
                    stats.config_patch = Some(("delta_link".into(), Value::String(new_link)));
                })
            } else {
                traverse(
                    &cfg.graph_base,
                    &token,
                    &drive.id,
                    cfg.folder_path.as_deref(),
                    ctx.params.max_depth,
                    prefix,
                    sink,
                    &mut stats,
                )
            };
            // A single restricted library shouldn't abort the others.
            if let Err(e) = result {
                eprintln!(
                    "warning: skipping library '{}': {e}",
                    drive.name.as_deref().unwrap_or(&drive.id)
                );
                stats.item_errors += 1;
            }
        }
        Ok(stats)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthMode {
    /// Interactive browser sign-in (authorization code + PKCE, loopback redirect).
    AuthCode,
    DeviceCode,
    AzureCli,
    ClientCredentials,
    /// Browser session cookies against the SharePoint REST API (no Graph token).
    Cookie,
    /// Interactive browser sign-in for the SharePoint *resource*, then the REST
    /// API. Avoids the Graph admin-consent wall; the token auto-refreshes.
    BrowserRest,
}

impl AuthMode {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s.to_lowercase().as_str() {
            "browser" | "auth_code" | "authcode" | "interactive" => Self::AuthCode,
            "device_code" | "device" => Self::DeviceCode,
            "azure_cli" | "az" | "cli" => Self::AzureCli,
            "client_credentials" | "app" | "secret" => Self::ClientCredentials,
            "cookie" | "session" | "browser_cookie" => Self::Cookie,
            "browser_rest" | "browser_sp" | "login" | "spo" => Self::BrowserRest,
            other => {
                return Err(Error::SharePoint(format!(
                    "unknown auth mode '{other}' (expected: browser, browser_rest, device_code, azure_cli, client_credentials, cookie)"
                )))
            }
        })
    }
}

#[derive(Debug)]
struct GraphConfig {
    auth: AuthMode,
    tenant_id: String,
    client_id: String,
    secret_env: String,
    az_path: String,
    drive_id: Option<String>,
    site_hostname: Option<String>,
    site_path: Option<String>,
    folder_path: Option<String>,
    delta_link: Option<String>,
    /// Graph API base, no trailing slash. Override via `graph_base`.
    graph_base: String,
    /// OAuth2 authority base (`{base}/token`, `{base}/devicecode`). Override
    /// via `oauth_base` (also lets tests point auth at a mock server).
    oauth_base: String,
    /// Delegated OAuth scopes requested by the interactive flows. Override via
    /// `scopes`. Default asks explicitly for SharePoint read so the token can
    /// actually read document libraries (not just site metadata).
    scopes: String,
    /// Env var holding the SharePoint session cookie string (auth=cookie).
    cookie_env: String,
    /// REST base override for cookie mode (`https://{site_hostname}` by default;
    /// tests point this at a mock server).
    rest_base: Option<String>,
    /// Tenant-wide mode (REST auth only): discover every accessible site via
    /// Search and crawl each, instead of the single `site_path`.
    all_sites: bool,
    /// Search query used to enumerate sites. Default finds all site collections.
    sites_query: String,
    /// Only crawl discovered site URLs containing this substring (optional).
    sites_filter: Option<String>,
    /// Safety cap on how many sites a tenant-wide crawl visits.
    max_sites: usize,
}

/// Delegated scopes for interactive sign-in. `Sites.Read.All` is the
/// least-privilege scope that reads a site's document libraries; `offline_access`
/// yields a refresh token. Explicit (not `.default`) so consent grants content
/// read. Add `Files.Read.All` via the `scopes` config if a tenant needs it.
const DEFAULT_DELEGATED_SCOPES: &str = "offline_access https://graph.microsoft.com/Sites.Read.All";

impl GraphConfig {
    fn from_source(source: &Source) -> Result<Self> {
        let auth = AuthMode::parse(
            &source
                .config_str("auth")
                .unwrap_or_else(|| "device_code".into()),
        )?;

        // Tenant default depends on the flow: app-only needs a real tenant;
        // delegated sign-in can use the multi-tenant "organizations" authority.
        let tenant_id = source
            .config_str("tenant_id")
            .unwrap_or_else(|| match auth {
                AuthMode::ClientCredentials => String::new(),
                _ => "organizations".to_string(),
            });
        if auth == AuthMode::ClientCredentials && tenant_id.is_empty() {
            return Err(Error::SharePoint(
                "client_credentials auth requires `tenant_id` in source config".into(),
            ));
        }

        // The interactive flows (browser, device_code) fall back to a public
        // client so they work with no app registration; client_credentials must
        // name its own app.
        let client_id = source
            .config_str("client_id")
            .unwrap_or_else(|| match auth {
                AuthMode::AuthCode | AuthMode::DeviceCode => DEFAULT_PUBLIC_CLIENT_ID.to_string(),
                AuthMode::BrowserRest => AZURE_CLI_CLIENT_ID.to_string(),
                AuthMode::AzureCli | AuthMode::ClientCredentials | AuthMode::Cookie => {
                    String::new()
                }
            });
        if auth == AuthMode::ClientCredentials && client_id.is_empty() {
            return Err(Error::SharePoint(
                "client_credentials auth requires `client_id` in source config".into(),
            ));
        }

        let graph_base = source
            .config_str("graph_base")
            .unwrap_or_else(|| GRAPH.to_string())
            .trim_end_matches('/')
            .to_string();
        let oauth_base = source
            .config_str("oauth_base")
            .unwrap_or_else(|| format!("https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0"))
            .trim_end_matches('/')
            .to_string();

        let site_hostname = source.config_str("site_hostname");
        // browser_rest needs a token for the SharePoint *resource*, not Graph.
        let default_scopes = match (auth, &site_hostname) {
            (AuthMode::BrowserRest, Some(host)) => {
                format!("offline_access https://{host}/.default")
            }
            _ => DEFAULT_DELEGATED_SCOPES.to_string(),
        };

        Ok(Self {
            auth,
            tenant_id,
            client_id,
            secret_env: source
                .config_str("secret_env")
                .unwrap_or_else(|| DEFAULT_SECRET_ENV.to_string()),
            az_path: source
                .config_str("az_path")
                .unwrap_or_else(|| "az".to_string()),
            drive_id: source.config_str("drive_id"),
            site_hostname,
            site_path: source.config_str("site_path"),
            folder_path: source.config_str("folder_path"),
            delta_link: source.config_str("delta_link"),
            graph_base,
            oauth_base,
            scopes: source.config_str("scopes").unwrap_or(default_scopes),
            cookie_env: source
                .config_str("cookie_env")
                .unwrap_or_else(|| "CRAWL_SHAREPOINT_COOKIE".to_string()),
            rest_base: source.config_str("rest_base"),
            all_sites: source
                .config
                .get("all_sites")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            sites_query: source
                .config_str("sites_query")
                .unwrap_or_else(|| "contentclass:STS_Site".to_string()),
            sites_filter: source.config_str("sites_filter"),
            max_sites: source
                .config
                .get("max_sites")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize,
        })
    }
}

fn fetch_token(cfg: &GraphConfig, cache_dir: &Path, source_name: &str) -> Result<String> {
    match cfg.auth {
        AuthMode::ClientCredentials => token_client_credentials(cfg),
        AuthMode::AzureCli => token_azure_cli(cfg),
        AuthMode::DeviceCode => token_device_code(cfg, cache_dir, source_name),
        AuthMode::AuthCode => token_auth_code(cfg, cache_dir, source_name),
        // These never mint a Graph token here; they are dispatched earlier.
        AuthMode::Cookie | AuthMode::BrowserRest => Err(Error::SharePoint(
            "this auth mode uses the REST path".into(),
        )),
    }
}

/// A still-valid cached access token, or one obtained by silently refreshing.
/// Returns `None` when there is no usable cache (caller falls back to sign-in).
fn cached_or_refresh(cfg: &GraphConfig, cache_path: &Path) -> Option<String> {
    let cached = read_cache(cache_path)?;
    let now = chrono::Utc::now().timestamp_millis();
    if cached.expires_at_ms > now + 60_000 {
        return Some(cached.access_token);
    }
    let rt = cached.refresh_token.as_ref()?;
    refresh_token_grant(cfg, rt, cache_path).ok()
}

fn token_client_credentials(cfg: &GraphConfig) -> Result<String> {
    let secret = std::env::var(&cfg.secret_env).map_err(|_| {
        Error::SharePoint(format!(
            "client secret not found in environment variable '{}' \
             (set it, or point `secret_env` at the variable that holds it)",
            cfg.secret_env
        ))
    })?;
    let url = format!("{}/token", cfg.oauth_base);
    let resp = ureq::post(&url)
        .send_form(&[
            ("client_id", cfg.client_id.as_str()),
            ("scope", "https://graph.microsoft.com/.default"),
            ("client_secret", secret.as_str()),
            ("grant_type", "client_credentials"),
        ])
        .map_err(|e| Error::SharePoint(format!("token request failed: {e}")))?;
    let body: Value = resp
        .into_json()
        .map_err(|e| Error::SharePoint(format!("token response not JSON: {e}")))?;
    access_token_from(&body)
}

/// Reuse an existing `az login` by shelling out to the Azure CLI.
fn token_azure_cli(cfg: &GraphConfig) -> Result<String> {
    let mut cmd = std::process::Command::new(&cfg.az_path);
    cmd.args([
        "account",
        "get-access-token",
        "--resource",
        "https://graph.microsoft.com",
        "--output",
        "json",
    ]);
    if !cfg.tenant_id.is_empty() && cfg.tenant_id != "organizations" {
        cmd.args(["--tenant", &cfg.tenant_id]);
    }
    let out = cmd.output().map_err(|e| {
        Error::SharePoint(format!(
            "failed to run '{}': {e} — is the Azure CLI installed and have you run `az login`?",
            cfg.az_path
        ))
    })?;
    if !out.status.success() {
        return Err(Error::SharePoint(format!(
            "`az account get-access-token` failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let body: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| Error::SharePoint(format!("az token output not JSON: {e}")))?;
    body.get("accessToken")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| Error::SharePoint("az token output had no accessToken".into()))
}

/// Interactive device-code sign-in, with a cached/refreshable token so repeat
/// crawls don't prompt again.
fn token_device_code(cfg: &GraphConfig, cache_dir: &Path, source_name: &str) -> Result<String> {
    let cache_path = token_cache_path(cache_dir, source_name, resource_tag(cfg));
    if let Some(tok) = cached_or_refresh(cfg, &cache_path) {
        return Ok(tok);
    }
    let dc = request_device_code(cfg)?;
    // The human-facing prompt goes to stderr so `--json` stdout stays clean.
    eprintln!("\n{}\n", dc.message);
    poll_device_token(cfg, &dc, &cache_path)
}

/// Interactive *browser* sign-in: OAuth2 authorization code + PKCE with a
/// loopback redirect. crawl opens the browser, you log in normally (username /
/// password / MFA — whatever your tenant requires), and the redirect is caught
/// on `http://localhost:<port>`. No device-code screen, no client secret. The
/// token (and refresh token) is cached so later crawls don't prompt.
fn token_auth_code(cfg: &GraphConfig, cache_dir: &Path, source_name: &str) -> Result<String> {
    let cache_path = token_cache_path(cache_dir, source_name, resource_tag(cfg));
    if let Some(tok) = cached_or_refresh(cfg, &cache_path) {
        return Ok(tok);
    }

    // PKCE: a random verifier and its S256 challenge guard the code exchange.
    let verifier = random_url_token(32);
    let challenge = pkce_challenge(&verifier);
    let state = random_url_token(16);

    // Loopback redirect on an ephemeral port (Azure AD allows any localhost port
    // for public clients).
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| Error::SharePoint(format!("could not open a local redirect port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error::SharePoint(e.to_string()))?
        .port();
    let redirect_uri = format!("http://localhost:{port}");

    let scope = cfg.scopes.as_str();
    let auth_url = format!(
        "{}/authorize?client_id={}&response_type=code&redirect_uri={}&response_mode=query\
         &scope={}&code_challenge={}&code_challenge_method=S256&state={}&prompt=select_account",
        cfg.oauth_base,
        pct(&cfg.client_id),
        pct(&redirect_uri),
        pct(scope),
        pct(&challenge),
        pct(&state),
    );

    eprintln!("\nOpening your browser to sign in to SharePoint…");
    eprintln!("If it doesn't open automatically, paste this URL into your browser:\n{auth_url}\n");
    let _ = open_browser(&auth_url);

    let code = wait_for_redirect_code(&listener, &state)?;

    let body = post_form_json(
        &format!("{}/token", cfg.oauth_base),
        &[
            ("grant_type", "authorization_code"),
            ("client_id", cfg.client_id.as_str()),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_verifier", verifier.as_str()),
            ("scope", scope),
        ],
    )?;
    cache_from_token_response(&body, &cache_path);
    eprintln!("Signed in. Token cached for future crawls.");
    access_token_from(&body)
}

/// Block until the OAuth redirect hits the loopback listener, returning the
/// authorization code. Verifies the `state` to defeat CSRF.
fn wait_for_redirect_code(listener: &std::net::TcpListener, expect_state: &str) -> Result<String> {
    use std::io::{BufRead, BufReader, Write};
    let (mut stream, _) = listener
        .accept()
        .map_err(|e| Error::SharePoint(format!("waiting for sign-in redirect failed: {e}")))?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .map_err(|e| Error::SharePoint(e.to_string()))?,
    );
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|e| Error::SharePoint(e.to_string()))?;

    // request_line: "GET /?code=...&state=... HTTP/1.1"
    let target = request_line.split_whitespace().nth(1).unwrap_or("");
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut err: Option<String> = None;
    for (k, v) in query.split('&').filter_map(|p| p.split_once('=')) {
        let val = pct_decode(v);
        match k {
            "code" => code = Some(val),
            "state" => state = Some(val),
            "error_description" => err = Some(val),
            "error" if err.is_none() => err = Some(val),
            _ => {}
        }
    }

    let (status, page) = if let Some(e) = &err {
        ("400 Bad Request", format!("Sign-in failed: {e}"))
    } else if state.as_deref() != Some(expect_state) {
        ("400 Bad Request", "Sign-in state mismatch.".to_string())
    } else if code.is_some() {
        (
            "200 OK",
            "Signed in to SharePoint. You can close this tab and return to the terminal."
                .to_string(),
        )
    } else {
        (
            "400 Bad Request",
            "No authorization code received.".to_string(),
        )
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nConnection: close\r\n\r\n\
         <html><body style=\"font-family:sans-serif\"><h3>{page}</h3></body></html>"
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();

    if let Some(e) = err {
        return Err(Error::SharePoint(format!("browser sign-in failed: {e}")));
    }
    if state.as_deref() != Some(expect_state) {
        return Err(Error::SharePoint(
            "browser sign-in state mismatch (possible CSRF)".into(),
        ));
    }
    code.ok_or_else(|| Error::SharePoint("no authorization code in redirect".into()))
}

/// Best-effort: open `url` in the user's default browser.
fn open_browser(url: &str) -> std::io::Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|_| ())
}

struct DeviceCode {
    device_code: String,
    interval: u64,
    expires_in: i64,
    message: String,
}

fn request_device_code(cfg: &GraphConfig) -> Result<DeviceCode> {
    let url = format!("{}/devicecode", cfg.oauth_base);
    let body = post_form_json(
        &url,
        &[
            ("client_id", cfg.client_id.as_str()),
            ("scope", cfg.scopes.as_str()),
        ],
    )?;
    let device_code = body
        .get("device_code")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            Error::SharePoint(format!("device code response missing device_code: {body}"))
        })?
        .to_string();
    let user_code = body.get("user_code").and_then(|v| v.as_str()).unwrap_or("");
    let verification_uri = body
        .get("verification_uri")
        .and_then(|v| v.as_str())
        .unwrap_or("https://microsoft.com/devicelogin");
    let message = body
        .get("message")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| {
            format!("To sign in, open {verification_uri} and enter the code {user_code}")
        });
    Ok(DeviceCode {
        device_code,
        interval: body.get("interval").and_then(|v| v.as_u64()).unwrap_or(5),
        expires_in: body
            .get("expires_in")
            .and_then(|v| v.as_i64())
            .unwrap_or(900),
        message,
    })
}

fn poll_device_token(cfg: &GraphConfig, dc: &DeviceCode, cache_path: &Path) -> Result<String> {
    let url = format!("{}/token", cfg.oauth_base);
    let deadline = chrono::Utc::now().timestamp_millis() + dc.expires_in * 1000;
    let mut interval = dc.interval;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(interval));
        let (status, body) = post_form_status(
            &url,
            &[
                ("grant_type", DEVICE_GRANT),
                ("client_id", cfg.client_id.as_str()),
                ("device_code", dc.device_code.as_str()),
            ],
        )?;
        if status == 200 {
            cache_from_token_response(&body, cache_path);
            return access_token_from(&body);
        }
        match body.get("error").and_then(|v| v.as_str()) {
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            Some(other) => {
                let desc = body
                    .get("error_description")
                    .and_then(|v| v.as_str())
                    .unwrap_or(other);
                return Err(Error::SharePoint(format!("device sign-in failed: {desc}")));
            }
            None => {
                return Err(Error::SharePoint(format!(
                    "unexpected token response: {body}"
                )))
            }
        }
        if chrono::Utc::now().timestamp_millis() >= deadline {
            return Err(Error::SharePoint("device sign-in timed out".into()));
        }
    }
}

fn refresh_token_grant(
    cfg: &GraphConfig,
    refresh_token: &str,
    cache_path: &Path,
) -> Result<String> {
    let url = format!("{}/token", cfg.oauth_base);
    let body = post_form_json(
        &url,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", cfg.client_id.as_str()),
            ("refresh_token", refresh_token),
            ("scope", cfg.scopes.as_str()),
        ],
    )?;
    cache_from_token_response(&body, cache_path);
    access_token_from(&body)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedToken {
    access_token: String,
    refresh_token: Option<String>,
    expires_at_ms: i64,
}

fn token_cache_path(cache_dir: &Path, source_name: &str, tag: &str) -> std::path::PathBuf {
    let safe: String = source_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cache_dir.join(format!("sharepoint-{safe}-{tag}.token.json"))
}

/// A short discriminator so a Graph token and a SharePoint-resource token for
/// the same source never share a cache file (different audiences).
fn resource_tag(cfg: &GraphConfig) -> &'static str {
    if cfg.scopes.contains("graph.microsoft.com") {
        "graph"
    } else {
        "spo"
    }
}

fn read_cache(path: &Path) -> Option<CachedToken> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn cache_from_token_response(body: &Value, cache_path: &Path) {
    let access_token = match body.get("access_token").and_then(|v| v.as_str()) {
        Some(t) => t.to_string(),
        None => return,
    };
    let expires_in = body
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .unwrap_or(3600);
    let cached = CachedToken {
        access_token,
        refresh_token: body
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(String::from),
        expires_at_ms: chrono::Utc::now().timestamp_millis() + expires_in * 1000,
    };
    if let Ok(json) = serde_json::to_vec(&cached) {
        let _ = std::fs::write(cache_path, json);
    }
}

fn access_token_from(body: &Value) -> Result<String> {
    body.get("access_token")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| Error::SharePoint(format!("no access_token in token response: {body}")))
}

/// POST a form, returning the parsed JSON body and treating any non-2xx with a
/// JSON body as a value too (so OAuth error payloads are inspectable).
fn post_form_status(url: &str, fields: &[(&str, &str)]) -> Result<(u16, Value)> {
    match ureq::post(url).send_form(fields) {
        Ok(resp) => {
            let status = resp.status();
            let body = resp
                .into_json()
                .map_err(|e| Error::SharePoint(format!("token response not JSON: {e}")))?;
            Ok((status, body))
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp
                .into_json()
                .map_err(|e| Error::SharePoint(format!("error response not JSON: {e}")))?;
            Ok((code, body))
        }
        Err(e) => Err(Error::SharePoint(format!("request to {url} failed: {e}"))),
    }
}

fn post_form_json(url: &str, fields: &[(&str, &str)]) -> Result<Value> {
    let (status, body) = post_form_status(url, fields)?;
    if status == 200 {
        Ok(body)
    } else {
        let desc = body
            .get("error_description")
            .and_then(|v| v.as_str())
            .or_else(|| body.get("error").and_then(|v| v.as_str()))
            .unwrap_or("request failed");
        Err(Error::SharePoint(format!("{url}: {desc}")))
    }
}

/// A URL-safe random token of `n_bytes` entropy, base64url-encoded (no padding).
/// Used for the PKCE verifier and the CSRF `state`.
fn random_url_token(n_bytes: usize) -> String {
    // /dev/urandom is the right entropy source on the platforms this targets.
    let mut buf = vec![0u8; n_bytes];
    match std::fs::File::open("/dev/urandom").and_then(|mut f| {
        use std::io::Read;
        f.read_exact(&mut buf)
    }) {
        Ok(()) => base64url(&buf),
        // Last-resort fallback so sign-in still works; uuid v7 carries a random
        // component. Two of them give ample unguessable entropy for one flow.
        Err(_) => base64url(
            format!(
                "{}{}",
                uuid::Uuid::now_v7().as_simple(),
                uuid::Uuid::now_v7().as_simple()
            )
            .as_bytes(),
        ),
    }
}

/// PKCE S256 challenge: base64url(sha256(verifier)), no padding (RFC 7636).
fn pkce_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(verifier.as_bytes());
    base64url(&h.finalize())
}

/// base64url encoding without padding (RFC 4648 §5).
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6 & 63) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        }
    }
    out
}

/// Decode base64url (no padding) back to bytes. Returns None on invalid input.
fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        let v = val(c)?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Decode the claims (payload) of a JWT without verifying its signature — for
/// diagnostics only (e.g. showing a token's `aud`/`scp` after a 403).
fn decode_jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64url_decode(payload)?;
    serde_json::from_slice(&bytes).ok()
}

/// A short human summary of a SharePoint token's audience and scopes, for
/// explaining a 403.
fn token_diagnostics(token: &str) -> Option<String> {
    let claims = decode_jwt_claims(token)?;
    let aud = claims.get("aud").and_then(|v| v.as_str()).unwrap_or("?");
    let scp = claims
        .get("scp")
        .and_then(|v| v.as_str())
        .unwrap_or("(none)");
    let roles = claims
        .get("roles")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|r| r.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    Some(format!(
        "token audience: {aud}\n  token scopes:   {scp}{}",
        if roles.is_empty() {
            String::new()
        } else {
            format!("\n  token roles:    {roles}")
        }
    ))
}

/// Percent-encode a query-string component (unreserved chars pass through).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// Decode a percent-encoded query component (`+` → space).
fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// One document library (Graph drive) to crawl.
struct DriveRef {
    id: String,
    name: Option<String>,
}

/// Resolve the libraries to crawl. With an explicit `drive_id`, that one. Else
/// resolve the site and list *all* its document libraries, so a restricted or
/// empty default library never hides the rest of the site's documents.
fn resolve_drives(token: &str, cfg: &GraphConfig) -> Result<Vec<DriveRef>> {
    if let Some(id) = &cfg.drive_id {
        return Ok(vec![DriveRef {
            id: id.clone(),
            name: None,
        }]);
    }
    let base = &cfg.graph_base;
    let site_id = resolve_site_id(token, cfg)?;

    // List every document library in the site.
    if let Ok(v) = graph_get(token, &format!("{base}/sites/{site_id}/drives")) {
        let drives: Vec<DriveRef> = v
            .get("value")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|d| {
                        Some(DriveRef {
                            id: d.get("id")?.as_str()?.to_string(),
                            name: d.get("name").and_then(|n| n.as_str()).map(String::from),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !drives.is_empty() {
            return Ok(drives);
        }
    }

    // Fall back to the default library.
    let drive = graph_get(token, &format!("{base}/sites/{site_id}/drive")).map_err(|e| {
        Error::SharePoint(format!(
            "could not list document libraries for this site, and the default library is \
             inaccessible ({e}). If the documents are in a specific library, pass its id with \
             `--set drive_id=<id>`."
        ))
    })?;
    let id = drive.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
        Error::SharePoint(format!("default drive lookup returned no id: {drive}"))
    })?;
    Ok(vec![DriveRef {
        id: id.to_string(),
        name: None,
    }])
}

/// Resolve a site's id from `site_hostname` + `site_path`.
fn resolve_site_id(token: &str, cfg: &GraphConfig) -> Result<String> {
    let (host, path) = match (&cfg.site_hostname, &cfg.site_path) {
        (Some(h), Some(p)) => (h, p),
        _ => return Err(Error::SharePoint(
            "specify either `drive_id` or both `site_hostname` and `site_path` in source config"
                .into(),
        )),
    };
    let path = if path.starts_with('/') {
        path.clone()
    } else {
        format!("/{path}")
    };
    let site = graph_get(token, &format!("{}/sites/{host}:{path}", cfg.graph_base))?;
    site.get("id")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| Error::SharePoint(format!("site lookup returned no id: {site}")))
}

fn graph_get(token: &str, url: &str) -> Result<Value> {
    let resp = ureq::get(url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| Error::SharePoint(format!("GET {url} failed: {e}")))?;
    resp.into_json()
        .map_err(|e| Error::SharePoint(format!("GET {url} returned non-JSON: {e}")))
}

/// Depth-bounded children traversal. Root children are depth 1.
#[allow(clippy::too_many_arguments)]
fn traverse(
    base: &str,
    token: &str,
    drive_id: &str,
    folder_path: Option<&str>,
    max_depth: Option<usize>,
    lib_prefix: Option<&str>,
    sink: &mut dyn FnMut(DiscoveredItem),
    stats: &mut CrawlStats,
) -> Result<()> {
    let start = match folder_path {
        Some(f) if !f.is_empty() => {
            format!(
                "{base}/drives/{drive_id}/root:/{}:/children",
                f.trim_matches('/')
            )
        }
        _ => format!("{base}/drives/{drive_id}/root/children"),
    };

    // Stack of (children-listing URL, depth).
    let mut stack: Vec<(String, usize)> = vec![(start, 1)];
    while let Some((url, depth)) = stack.pop() {
        let mut next: Option<String> = Some(url);
        while let Some(page_url) = next.take() {
            let page = graph_get(token, &page_url)?;
            if let Some(items) = page.get("value").and_then(|v| v.as_array()) {
                for item in items {
                    if item.get("folder").is_some() {
                        let recurse = max_depth.map(|m| depth < m).unwrap_or(true);
                        if recurse {
                            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                                stack.push((
                                    format!("{base}/drives/{drive_id}/items/{id}/children"),
                                    depth + 1,
                                ));
                            }
                        }
                    } else if item.get("file").is_some() {
                        match to_item(item, lib_prefix) {
                            Some(di) => sink(di),
                            None => stats.item_errors += 1,
                        }
                    }
                }
            }
            next = page
                .get("@odata.nextLink")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
    }
    Ok(())
}

/// Delta traversal for incremental crawls. Returns the new delta link to store.
fn crawl_delta(
    base: &str,
    token: &str,
    drive_id: &str,
    delta_link: Option<&str>,
    sink: &mut dyn FnMut(DiscoveredItem),
    stats: &mut CrawlStats,
) -> Result<String> {
    let mut url = delta_link
        .map(String::from)
        .unwrap_or_else(|| format!("{base}/drives/{drive_id}/root/delta"));
    loop {
        let page = graph_get(token, &url)?;
        if let Some(items) = page.get("value").and_then(|v| v.as_array()) {
            for item in items {
                // Deletions surface as a `deleted` facet; incremental crawls
                // don't reconcile removals (a recursive run does).
                if item.get("deleted").is_some() {
                    continue;
                }
                if item.get("file").is_some() {
                    match to_item(item, None) {
                        Some(di) => sink(di),
                        None => stats.item_errors += 1,
                    }
                }
            }
        }
        if let Some(next) = page.get("@odata.nextLink").and_then(|v| v.as_str()) {
            url = next.to_string();
            continue;
        }
        if let Some(delta) = page.get("@odata.deltaLink").and_then(|v| v.as_str()) {
            return Ok(delta.to_string());
        }
        // No further pages and no delta link: hand back what we started from.
        return Ok(delta_link
            .map(String::from)
            .unwrap_or_else(|| format!("{base}/drives/{drive_id}/root/delta")));
    }
}

/// Convert a Graph `driveItem` (with a `file` facet) into a DiscoveredItem.
/// `lib_prefix`, when set, is prepended to the relative path so documents from
/// different libraries in the same site stay distinct.
fn to_item(item: &Value, lib_prefix: Option<&str>) -> Option<DiscoveredItem> {
    let name = item.get("name")?.as_str()?.to_string();
    let web_url = item
        .get("webUrl")
        .and_then(|v| v.as_str())
        .map(String::from);
    let id = item.get("id").and_then(|v| v.as_str()).map(String::from);
    // Prefer webUrl as the canonical uri (stable, user-clickable); fall back to id.
    let uri = web_url.clone().or_else(|| id.clone())?;

    let mut di = DiscoveredItem::new(uri, name.clone());
    di.size = item.get("size").and_then(|v| v.as_i64());
    di.modified_ms = item
        .get("lastModifiedDateTime")
        .and_then(|v| v.as_str())
        .and_then(iso8601_to_ms);
    di.provider_hash = item
        .get("file")
        .and_then(|f| f.get("hashes"))
        .and_then(|h| {
            h.get("quickXorHash")
                .or_else(|| h.get("sha256Hash"))
                .or_else(|| h.get("sha1Hash"))
        })
        .and_then(|v| v.as_str())
        .map(String::from);

    let parent_path = item
        .get("parentReference")
        .and_then(|p| p.get("path"))
        .and_then(|v| v.as_str());
    let rel = rel_path(parent_path, &name);
    di.rel_path = Some(match lib_prefix {
        Some(lib) => format!("{lib}/{rel}"),
        None => rel,
    });
    di.metadata = json!({
        "id": id,
        "web_url": web_url,
        "created": item.get("createdDateTime").and_then(|v| v.as_str()),
        "last_modified_by": item
            .get("lastModifiedBy")
            .and_then(|b| b.get("user"))
            .and_then(|u| u.get("displayName"))
            .and_then(|v| v.as_str()),
    });
    Some(di)
}

/// Derive a drive-relative path from a Graph `parentReference.path`
/// (e.g. `/drive/root:/Reports/2024`) plus the item name.
fn rel_path(parent_path: Option<&str>, name: &str) -> String {
    let prefix = match parent_path {
        Some(p) => match p.find("root:") {
            Some(idx) => p[idx + "root:".len()..].trim_matches('/').to_string(),
            None => String::new(),
        },
        None => String::new(),
    };
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn iso8601_to_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Cookie-based crawling via the SharePoint REST API (`/_api/...`), for when
/// Graph access is blocked by admin-consent policy. Authenticates with the
/// user's browser session cookies; read-only (GET), so no request digest.
mod rest {
    use super::{iso8601_to_ms, pct, AuthMode, GraphConfig};
    use crate::crawl::{CrawlContext, CrawlStats, DiscoveredItem};
    use crate::error::{Error, Result};
    use serde_json::Value;

    /// Libraries whose contents are site chrome, not user documents.
    const SYSTEM_LIBRARIES: &[&str] = &[
        "Form Templates",
        "Style Library",
        "Site Assets",
        "Preservation Hold Library",
        "Site Collection Documents",
        "Site Collection Images",
        "Master Page Gallery",
        "Theme Gallery",
        "Web Part Gallery",
        "List Template Gallery",
        "Solution Gallery",
        "Converted Forms",
    ];

    struct Library {
        title: String,
        root: String,
    }

    /// How a REST request authenticates: a browser session cookie, or an OAuth
    /// bearer token for the SharePoint resource.
    enum Auth {
        Cookie(String),
        Bearer(String),
    }

    /// Cookie auth (`auth=cookie`): read the session cookie from the env var.
    pub(super) fn crawl_with_cookie(
        cfg: &GraphConfig,
        ctx: &CrawlContext,
        sink: &mut dyn FnMut(DiscoveredItem),
    ) -> Result<CrawlStats> {
        debug_assert_eq!(cfg.auth, AuthMode::Cookie);
        crawl_inner(cfg, ctx, &Auth::Cookie(resolve_cookie(cfg)?), sink)
    }

    /// browser_rest auth: an OAuth token already obtained for the SP resource.
    pub(super) fn crawl_with_token(
        cfg: &GraphConfig,
        ctx: &CrawlContext,
        token: &str,
        sink: &mut dyn FnMut(DiscoveredItem),
    ) -> Result<CrawlStats> {
        crawl_inner(cfg, ctx, &Auth::Bearer(token.to_string()), sink)
    }

    fn crawl_inner(
        cfg: &GraphConfig,
        ctx: &CrawlContext,
        auth: &Auth,
        sink: &mut dyn FnMut(DiscoveredItem),
    ) -> Result<CrawlStats> {
        let host = cfg.site_hostname.as_deref().ok_or_else(|| {
            Error::SharePoint("SharePoint REST auth needs `site_hostname` in source config".into())
        })?;
        let base = cfg
            .rest_base
            .clone()
            .unwrap_or_else(|| format!("https://{host}"))
            .trim_end_matches('/')
            .to_string();

        let mut stats = CrawlStats::default();

        if cfg.all_sites {
            // Tenant-wide: discover every accessible site, crawl each (soft per
            // site, so one inaccessible site never sinks the run).
            let sites = enumerate_sites(
                &base,
                auth,
                &cfg.sites_query,
                cfg.sites_filter.as_deref(),
                cfg.max_sites,
            )?;
            eprintln!("Discovered {} site(s) to crawl", sites.len());
            for site in &sites {
                let web = format!("{base}{}", site.server_rel);
                // Keep the full server-relative path so docs from different sites
                // don't collide (uri is unique regardless, but paths stay clear).
                if let Err(e) = crawl_one_web(&web, auth, host, "", ctx, &mut stats, sink) {
                    eprintln!("warning: skipping site '{}': {e}", site.server_rel);
                    stats.item_errors += 1;
                }
            }
            return Ok(stats);
        }

        // Single site.
        let site_path = cfg.site_path.as_deref().ok_or_else(|| {
            Error::SharePoint("SharePoint REST auth needs `site_path` in source config".into())
        })?;
        let site_rel = format!("/{}", site_path.trim_matches('/'));
        let web = format!("{base}{site_rel}");
        // Propagate (so a 403 surfaces with diagnostics). Strip the site prefix
        // so paths are relative to the site (e.g. "Project Documents/...").
        crawl_one_web(&web, auth, host, &site_rel, ctx, &mut stats, sink)?;
        Ok(stats)
    }

    /// Crawl one web: all its non-system document libraries, recursing folders.
    /// `path_prefix_strip` is removed from each doc's rel_path (use "" to keep
    /// the full server-relative path, which disambiguates across sites).
    fn crawl_one_web(
        web: &str,
        auth: &Auth,
        host: &str,
        path_prefix_strip: &str,
        ctx: &CrawlContext,
        stats: &mut CrawlStats,
        sink: &mut dyn FnMut(DiscoveredItem),
    ) -> Result<()> {
        for lib in list_libraries(web, auth)? {
            if SYSTEM_LIBRARIES
                .iter()
                .any(|s| s.eq_ignore_ascii_case(&lib.title))
            {
                continue;
            }
            if let Err(e) = walk_folder(web, &lib.root, auth, host, path_prefix_strip, ctx, 1, sink)
            {
                eprintln!("warning: skipping library '{}': {e}", lib.title);
                stats.item_errors += 1;
            }
        }
        Ok(())
    }

    /// A site discovered via Search, identified by its server-relative URL.
    struct SiteRef {
        server_rel: String,
    }

    /// Enumerate sites accessible to the caller via the SharePoint Search API.
    /// Pages through results, applies the optional URL filter, and caps at `max`.
    fn enumerate_sites(
        base: &str,
        auth: &Auth,
        query: &str,
        filter: Option<&str>,
        max: usize,
    ) -> Result<Vec<SiteRef>> {
        let mut out: Vec<SiteRef> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let row_limit = 100usize;
        let mut start = 0usize;
        loop {
            let url = format!(
                "{base}/_api/search/query?querytext='{}'&rowlimit={row_limit}&startrow={start}\
                 &selectproperties='Path'&trimduplicates=true",
                pct(query)
            );
            let body = rest_get(&url, auth)?;
            let (total, rows) = parse_search_paths(&body);
            if rows.is_empty() {
                break;
            }
            for path in rows {
                if let Some(f) = filter {
                    if !path.contains(f) {
                        continue;
                    }
                }
                if let Some(server_rel) = url_to_same_host_rel(base, &path) {
                    if seen.insert(server_rel.clone()) {
                        out.push(SiteRef { server_rel });
                        if out.len() >= max {
                            return Ok(out);
                        }
                    }
                }
            }
            start += row_limit;
            if start >= total {
                break;
            }
        }
        Ok(out)
    }

    /// Pull `(TotalRows, [Path,...])` out of a Search query response.
    fn parse_search_paths(body: &Value) -> (usize, Vec<String>) {
        let rel = body
            .get("PrimaryQueryResult")
            .and_then(|p| p.get("RelevantResults"));
        let total = rel
            .and_then(|r| r.get("TotalRows"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let mut paths = Vec::new();
        if let Some(rows) = rel
            .and_then(|r| r.get("Table"))
            .and_then(|t| t.get("Rows"))
            .and_then(|v| v.as_array())
        {
            for row in rows {
                if let Some(cells) = row.get("Cells").and_then(|v| v.as_array()) {
                    for cell in cells {
                        if cell.get("Key").and_then(|v| v.as_str()) == Some("Path") {
                            if let Some(p) = cell.get("Value").and_then(|v| v.as_str()) {
                                paths.push(p.to_string());
                            }
                        }
                    }
                }
            }
        }
        (total, paths)
    }

    /// If `url` is on the same host as `base`, return its server-relative path
    /// (e.g. "/sites/Engineering"); otherwise None (a cookie/token is host-scoped, so
    /// other hosts — like a `-my` OneDrive host — are skipped).
    fn url_to_same_host_rel(base: &str, url: &str) -> Option<String> {
        let strip = |u: &str| -> Option<(String, String)> {
            let rest = u.split_once("://").map(|(_, r)| r).unwrap_or(u);
            let (host, path) = match rest.split_once('/') {
                Some((h, p)) => (h.to_string(), format!("/{}", p.trim_end_matches('/'))),
                None => (rest.to_string(), String::new()),
            };
            Some((host, path))
        };
        let (base_host, _) = strip(base)?;
        let (url_host, url_path) = strip(url)?;
        if url_host.eq_ignore_ascii_case(&base_host) {
            Some(if url_path.is_empty() {
                "/".to_string()
            } else {
                url_path
            })
        } else {
            None
        }
    }

    fn list_libraries(web: &str, auth: &Auth) -> Result<Vec<Library>> {
        let url = format!(
            "{web}/_api/web/lists?$select=Title,Hidden,BaseTemplate,RootFolder/ServerRelativeUrl\
             &$expand=RootFolder&$top=500"
        );
        let body = rest_get(&url, auth)?;
        let mut out = Vec::new();
        if let Some(arr) = body.get("value").and_then(|v| v.as_array()) {
            for l in arr {
                let template = l.get("BaseTemplate").and_then(|v| v.as_i64()).unwrap_or(0);
                let hidden = l.get("Hidden").and_then(|v| v.as_bool()).unwrap_or(false);
                if template != 101 || hidden {
                    continue; // 101 = document library
                }
                let root = l
                    .get("RootFolder")
                    .and_then(|r| r.get("ServerRelativeUrl"))
                    .and_then(|v| v.as_str());
                if let Some(root) = root {
                    out.push(Library {
                        title: l
                            .get("Title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        root: root.to_string(),
                    });
                }
            }
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_folder(
        web: &str,
        folder: &str,
        auth: &Auth,
        host: &str,
        site_rel: &str,
        ctx: &CrawlContext,
        depth: usize,
        sink: &mut dyn FnMut(DiscoveredItem),
    ) -> Result<()> {
        let files_url = format!(
            "{web}/_api/web/GetFolderByServerRelativeUrl('{}')/Files\
             ?$select=Name,ServerRelativeUrl,Length,TimeLastModified&$top=5000",
            odata_escape(folder)
        );
        let files = rest_get(&files_url, auth)?;
        if let Some(arr) = files.get("value").and_then(|v| v.as_array()) {
            for f in arr {
                if let Some(item) = to_item(f, host, site_rel, ctx) {
                    sink(item);
                }
            }
        }

        let recurse = ctx.params.max_depth.map(|m| depth < m).unwrap_or(true);
        if !recurse {
            return Ok(());
        }
        let folders_url = format!(
            "{web}/_api/web/GetFolderByServerRelativeUrl('{}')/Folders\
             ?$select=Name,ServerRelativeUrl&$top=5000",
            odata_escape(folder)
        );
        let folders = rest_get(&folders_url, auth)?;
        if let Some(arr) = folders.get("value").and_then(|v| v.as_array()) {
            for sub in arr {
                let name = sub.get("Name").and_then(|v| v.as_str()).unwrap_or("");
                if name.is_empty() || name == "Forms" {
                    continue; // skip the system Forms folder
                }
                if let Some(srv) = sub.get("ServerRelativeUrl").and_then(|v| v.as_str()) {
                    walk_folder(web, srv, auth, host, site_rel, ctx, depth + 1, sink)?;
                }
            }
        }
        Ok(())
    }

    fn to_item(
        f: &Value,
        host: &str,
        site_rel: &str,
        ctx: &CrawlContext,
    ) -> Option<DiscoveredItem> {
        let name = f.get("Name")?.as_str()?.to_string();
        let srv = f.get("ServerRelativeUrl")?.as_str()?;
        let modified_ms = f
            .get("TimeLastModified")
            .and_then(|v| v.as_str())
            .and_then(iso8601_to_ms);
        if let (Some(since), Some(m)) = (ctx.params.since_ms, modified_ms) {
            if m < since {
                return None;
            }
        }
        let mut di = DiscoveredItem::new(format!("https://{host}{srv}"), name);
        di.size = f.get("Length").and_then(parse_len);
        di.modified_ms = modified_ms;
        // Path relative to the site web (drops the /sites/.../ prefix, keeps the
        // library folder), e.g. "Shared Documents/Plans/policy.pdf".
        let rel = srv
            .strip_prefix(site_rel)
            .unwrap_or(srv)
            .trim_start_matches('/');
        di.rel_path = Some(rel.to_string());
        Some(di)
    }

    fn parse_len(v: &Value) -> Option<i64> {
        // SharePoint REST returns Length as a string.
        v.as_str()
            .and_then(|s| s.parse().ok())
            .or_else(|| v.as_i64())
    }

    fn odata_escape(path: &str) -> String {
        path.replace('\'', "''")
    }

    fn resolve_cookie(cfg: &GraphConfig) -> Result<String> {
        if let Ok(v) = std::env::var(&cfg.cookie_env) {
            if !v.trim().is_empty() {
                return Ok(v);
            }
        }
        Err(Error::SharePoint(format!(
            "no SharePoint cookie found. Copy the `FedAuth` and `rtFa` cookies from your browser \
             (DevTools → Application → Cookies → your sharepoint.com site) and export them:\n  \
             export {}='FedAuth=...; rtFa=...'",
            cfg.cookie_env
        )))
    }

    fn rest_get(url: &str, auth: &Auth) -> Result<Value> {
        let req = ureq::get(url).set("Accept", "application/json;odata=nometadata");
        let req = match auth {
            Auth::Cookie(c) => req.set("Cookie", c),
            Auth::Bearer(t) => req.set("Authorization", &format!("Bearer {t}")),
        };
        match req.call() {
            Ok(resp) => resp
                .into_json()
                .map_err(|e| Error::SharePoint(format!("REST {url} returned non-JSON: {e}"))),
            Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => {
                let hint = match auth {
                    Auth::Cookie(_) => "your session cookies are missing, expired, or lack access. \
                                        Copy fresh FedAuth/rtFa cookies from your browser and re-export.",
                    Auth::Bearer(_) => "the sign-in token lacks access to this site, or expired. \
                                        Re-run with `--reauth` to sign in again.",
                };
                Err(Error::SharePoint(format!(
                    "SharePoint returned 401/403 — {hint}"
                )))
            }
            Err(e) => Err(Error::SharePoint(format!("REST {url} failed: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_path_from_parent_reference() {
        assert_eq!(
            rel_path(Some("/drive/root:/Reports/2024"), "q1.pdf"),
            "Reports/2024/q1.pdf"
        );
        assert_eq!(rel_path(Some("/drive/root:"), "top.docx"), "top.docx");
        assert_eq!(rel_path(None, "loose.txt"), "loose.txt");
    }

    #[test]
    fn iso_dates_parse_to_ms() {
        assert_eq!(iso8601_to_ms("1970-01-01T00:00:01Z"), Some(1000));
        assert!(iso8601_to_ms("not-a-date").is_none());
    }

    #[test]
    fn to_item_extracts_core_fields() {
        let v = json!({
            "name": "plan.docx",
            "webUrl": "https://x.sharepoint.com/plan.docx",
            "id": "01ABC",
            "size": 1234,
            "lastModifiedDateTime": "2024-01-02T03:04:05Z",
            "file": { "hashes": { "quickXorHash": "ABC123" } },
            "parentReference": { "path": "/drive/root:/Plans" }
        });
        let di = to_item(&v, None).unwrap();
        assert_eq!(di.name, "plan.docx");
        assert_eq!(di.uri, "https://x.sharepoint.com/plan.docx");
        assert_eq!(di.extension.as_deref(), Some("docx"));
        assert_eq!(di.size, Some(1234));
        assert_eq!(di.provider_hash.as_deref(), Some("ABC123"));
        assert_eq!(di.rel_path.as_deref(), Some("Plans/plan.docx"));
    }

    fn source_with(config: serde_json::Value) -> Source {
        Source {
            id: 1,
            name: "sp".into(),
            kind: SourceKind::SharePoint,
            uri: "x".into(),
            strategy: Strategy::Recursive,
            config,
            enabled: true,
            added_at: 0,
            last_crawled: None,
            last_run_id: None,
            last_status: None,
            last_error: None,
        }
    }

    #[test]
    fn interactive_modes_default_to_public_client() {
        // Regression for AADSTS900144: browser/device_code must supply a
        // client_id even when the user configures none.
        for mode in ["browser", "device_code"] {
            let cfg = GraphConfig::from_source(&source_with(json!({ "auth": mode }))).unwrap();
            assert_eq!(
                cfg.client_id, DEFAULT_PUBLIC_CLIENT_ID,
                "auth={mode} should default client_id"
            );
        }
    }

    #[test]
    fn client_credentials_requires_explicit_ids() {
        let err = GraphConfig::from_source(&source_with(json!({ "auth": "client_credentials" })))
            .unwrap_err();
        assert!(err.to_string().contains("tenant_id"), "{err}");
    }

    #[test]
    fn pkce_matches_rfc7636_vector() {
        // RFC 7636 Appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn jwt_claims_decode_for_diagnostics() {
        // header.payload.signature with a payload of {"aud":"https://x.sharepoint.com","scp":"AllSites.Read"}
        let payload = base64url(br#"{"aud":"https://x.sharepoint.com","scp":"AllSites.Read"}"#);
        let token = format!("aaa.{payload}.bbb");
        let claims = decode_jwt_claims(&token).unwrap();
        assert_eq!(claims["aud"], "https://x.sharepoint.com");
        let diag = token_diagnostics(&token).unwrap();
        assert!(diag.contains("https://x.sharepoint.com"));
        assert!(diag.contains("AllSites.Read"));
    }

    #[test]
    fn base64url_decode_round_trips() {
        for s in [&b""[..], b"f", b"fo", b"foobar", &[0xfb, 0xff, 0x00, 0x10]] {
            assert_eq!(base64url_decode(&base64url(s)).unwrap(), s);
        }
    }

    #[test]
    fn base64url_no_padding() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foobar"), "Zm9vYmFy");
        // URL-safe alphabet: 0xff,0xff produces '_' / '-', never '+' '/'.
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }

    #[test]
    fn percent_encode_round_trips() {
        let s = "offline_access https://graph.microsoft.com/.default";
        let enc = pct(s);
        assert!(!enc.contains(' ') && !enc.contains('/'));
        assert_eq!(pct_decode(&enc), s);
        assert_eq!(pct_decode("a%20b%2Fc"), "a b/c");
    }

    #[test]
    fn random_tokens_are_unique_and_url_safe() {
        let a = random_url_token(32);
        let b = random_url_token(32);
        assert_ne!(a, b);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    /// Drive the loopback redirect catcher as a browser would, without a browser.
    fn simulate_redirect(query: &str) -> Result<String> {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let q = query.to_string();
        let browser = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET /?{q} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
            buf
        });
        let result = wait_for_redirect_code(&listener, "STATE123");
        let _ = browser.join();
        result
    }

    #[test]
    fn redirect_catcher_extracts_code() {
        let code = simulate_redirect("code=AUTH_CODE_42&state=STATE123").unwrap();
        assert_eq!(code, "AUTH_CODE_42");
    }

    #[test]
    fn redirect_catcher_rejects_bad_state() {
        let err = simulate_redirect("code=x&state=WRONG").unwrap_err();
        assert!(err.to_string().contains("state mismatch"), "{err}");
    }

    #[test]
    fn redirect_catcher_surfaces_oauth_error() {
        let err = simulate_redirect("error=access_denied&error_description=Approval+required")
            .unwrap_err();
        assert!(err.to_string().contains("Approval required"), "{err}");
    }

    #[test]
    fn browser_rest_default_scope_targets_sharepoint_resource() {
        // browser_rest must request a token for the SharePoint resource (not
        // Graph) and default to the Azure CLI public client.
        let cfg = GraphConfig::from_source(&source_with(json!({
            "auth": "browser_rest",
            "site_hostname": "contoso.sharepoint.com",
            "site_path": "/sites/Marketing",
        })))
        .unwrap();
        assert!(
            cfg.scopes
                .contains("https://contoso.sharepoint.com/.default"),
            "scopes: {}",
            cfg.scopes
        );
        assert!(!cfg.scopes.contains("graph.microsoft.com"));
        assert_eq!(cfg.client_id, AZURE_CLI_CLIENT_ID);
        assert_eq!(resource_tag(&cfg), "spo");
    }

    #[test]
    fn browser_rest_sends_bearer_token_to_rest_api() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::sync::{Arc, Mutex};

        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let base = format!("http://127.0.0.1:{port}");
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut s = stream;
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut req = String::new();
                let _ = r.read_line(&mut req);
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if line.to_ascii_lowercase().starts_with("authorization:") {
                        seen2
                            .lock()
                            .unwrap()
                            .push(line["authorization:".len()..].trim().to_string());
                    }
                }
                let path = req.split_whitespace().nth(1).unwrap_or("");
                let body = if path.contains("/_api/web/lists") {
                    r#"{"value":[{"Title":"Documents","Hidden":false,"BaseTemplate":101,"RootFolder":{"ServerRelativeUrl":"/sites/X/Shared Documents"}}]}"#
                } else if path.contains("/Files") {
                    r#"{"value":[{"Name":"a.pdf","ServerRelativeUrl":"/sites/X/Shared Documents/a.pdf","Length":"5","TimeLastModified":"2024-01-02T03:04:05Z"}]}"#
                } else {
                    r#"{"value":[]}"#
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = s.write_all(resp.as_bytes());
            }
        });

        let cfg = GraphConfig::from_source(&source_with(json!({
            "auth": "browser_rest",
            "site_hostname": "x.sharepoint.com",
            "site_path": "/sites/X",
            "rest_base": base,
        })))
        .unwrap();

        let config = crate::config::Config {
            vault_name: String::new(),
            documents: crate::config::DocumentsConfig {
                extensions: vec![],
                excluded_extensions: vec![],
                size_cap_bytes: 0,
            },
            crawl: crate::config::CrawlConfig {
                hash: false,
                follow_symlinks: false,
                respect_crawlignore: true,
                default_strategy: "recursive".into(),
                default_max_depth: 0,
                concurrency: 4,
            },
        };
        let cache = std::path::PathBuf::from(".");
        let ctx = CrawlContext {
            params: crate::source::StrategyParams::default(),
            config: &config,
            cache_dir: &cache,
        };

        let mut items = Vec::new();
        rest::crawl_with_token(&cfg, &ctx, "FAKE-SP-TOKEN", &mut |i| items.push(i)).unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "a.pdf");
        assert_eq!(
            items[0].uri,
            "https://x.sharepoint.com/sites/X/Shared Documents/a.pdf"
        );
        let auths = seen.lock().unwrap();
        assert!(
            auths.iter().any(|a| a == "Bearer FAKE-SP-TOKEN"),
            "expected bearer token on REST requests, saw: {auths:?}"
        );
    }
}
