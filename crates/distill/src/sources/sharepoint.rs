//! SharePoint source — a fresh, independent REST traversal (not crawl's).
//!
//! Auth: **cookie** mode. Paste a browser session cookie (the `FedAuth` and
//! `rtFa` pair) into an env var; we call SharePoint's own REST API with it.
//! This deliberately bypasses the Microsoft Graph admin-consent wall that
//! blocks non-admin users on locked-down tenants.
//!
//! Traversal: list document libraries (`/_api/web/lists`, BaseTemplate 101),
//! then walk each library's folders/files recursively. Each file's clickable
//! `webUrl` (`https://{host}{ServerRelativeUrl}`) becomes its locator; bytes
//! are streamed lazily via `GetFileByServerRelativeUrl(...)/$value` only when a
//! document actually needs (re)indexing.
//!
//! `browser` (interactive OAuth) auth is a future step; cookie mode is the
//! working path today.

use super::{is_supported, SourceDoc};
use anyhow::{anyhow, bail, Context, Result};
use kb_core::SourceConfig;
use serde_json::Value;

pub fn enumerate(src: &SourceConfig) -> Result<Vec<SourceDoc>> {
    let SourceConfig::Sharepoint {
        site,
        auth,
        cookie_env,
        ..
    } = src
    else {
        return Ok(Vec::new());
    };

    match auth.as_str() {
        "cookie" => {
            let var = cookie_env
                .clone()
                .unwrap_or_else(|| "KB_SP_COOKIE".to_string());
            let cookie = std::env::var(&var).map_err(|_| {
                anyhow!(
                    "sharepoint auth=cookie but env `{var}` is unset — paste your FedAuth/rtFa \
                     session cookies into it (export {var}='FedAuth=...; rtFa=...')"
                )
            })?;
            if cookie.trim().is_empty() {
                bail!("env `{var}` is empty");
            }
            Client::new(site, cookie)?.enumerate()
        }
        other => bail!(
            "sharepoint auth `{other}` is not implemented yet — use auth = \"cookie\" with a \
             session cookie in cookie_env (browser OAuth is a future step)"
        ),
    }
}

/// A minimal SharePoint REST client scoped to one site.
struct Client {
    host: String,
    /// REST base, e.g. `https://host/sites/Eng/_api`.
    api: String,
    cookie: String,
}

impl Client {
    fn new(site: &str, cookie: String) -> Result<Self> {
        // Accept `host/sites/X`, `https://host/sites/X`, trailing slashes, etc.
        let trimmed = site
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');
        let (host, site_path) = match trimmed.split_once('/') {
            Some((h, rest)) => (h.to_string(), format!("/{}", rest.trim_matches('/'))),
            None => (trimmed.to_string(), String::new()),
        };
        if host.is_empty() {
            bail!("invalid sharepoint site: {site}");
        }
        let api = format!("https://{host}{site_path}/_api");
        Ok(Self { host, api, cookie })
    }

    fn enumerate(&self) -> Result<Vec<SourceDoc>> {
        let mut docs = Vec::new();
        for root in self.document_library_roots()? {
            self.walk(&root, &mut docs)?;
        }
        Ok(docs)
    }

    /// Server-relative roots of every non-hidden document library.
    fn document_library_roots(&self) -> Result<Vec<String>> {
        let url = format!(
            "{}/web/lists?$filter=BaseTemplate eq 101 and Hidden eq false\
             &$select=Title,RootFolder/ServerRelativeUrl&$expand=RootFolder",
            self.api
        );
        let body = self.get_json(&url).context("listing document libraries")?;
        let mut roots = Vec::new();
        for lib in array(&body) {
            if let Some(rel) = lib
                .get("RootFolder")
                .and_then(|rf| rf.get("ServerRelativeUrl"))
                .and_then(|v| v.as_str())
            {
                roots.push(rel.to_string());
            }
        }
        Ok(roots)
    }

    /// Recurse a folder, appending file docs and descending into subfolders.
    fn walk(&self, folder: &str, out: &mut Vec<SourceDoc>) -> Result<()> {
        // Files in this folder.
        let files_url = format!(
            "{}/web/GetFolderByServerRelativeUrl('{}')/Files\
             ?$select=Name,ServerRelativeUrl,Length,TimeLastModified",
            self.api,
            quote(folder)
        );
        for f in self.get_all(&files_url)? {
            let name = f.get("Name").and_then(|v| v.as_str()).unwrap_or("");
            let rel = f
                .get("ServerRelativeUrl")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if name.is_empty() || rel.is_empty() {
                continue;
            }
            let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
            if !is_supported(&ext) {
                continue;
            }
            let size = f
                .get("Length")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| f.get("Length").and_then(|v| v.as_u64()));
            let modified_at = f
                .get("TimeLastModified")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let download = format!(
                "{}/web/GetFileByServerRelativeUrl('{}')/$value",
                self.api,
                quote(rel)
            );
            let cookie = self.cookie.clone();
            out.push(SourceDoc {
                locator: format!("https://{}{}", self.host, rel),
                title: name.to_string(),
                ext,
                modified_at,
                size,
                read: Box::new(move || get_bytes(&download, &cookie)),
            });
        }

        // Subfolders (skip the system "Forms" folder and hidden ones).
        let folders_url = format!(
            "{}/web/GetFolderByServerRelativeUrl('{}')/Folders?$select=Name,ServerRelativeUrl",
            self.api,
            quote(folder)
        );
        for sub in self.get_all(&folders_url)? {
            let name = sub.get("Name").and_then(|v| v.as_str()).unwrap_or("");
            let rel = sub
                .get("ServerRelativeUrl")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if name.is_empty() || rel.is_empty() || name.eq_ignore_ascii_case("Forms") {
                continue;
            }
            self.walk(rel, out)?;
        }
        Ok(())
    }

    /// GET a collection, following `odata.nextLink`/`__next` pagination.
    fn get_all(&self, url: &str) -> Result<Vec<Value>> {
        let mut items = Vec::new();
        let mut next = Some(url.to_string());
        while let Some(u) = next {
            let body = self.get_json(&u)?;
            items.extend(array(&body));
            next = body
                .get("odata.nextLink")
                .or_else(|| body.get("@odata.nextLink"))
                .or_else(|| body.get("__next"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
        Ok(items)
    }

    fn get_json(&self, url: &str) -> Result<Value> {
        let resp = ureq::get(url)
            .set("Cookie", &self.cookie)
            .set("Accept", "application/json;odata=nometadata")
            .call()
            .map_err(|e| map_err(e, url))?;
        resp.into_json::<Value>()
            .with_context(|| format!("parsing JSON from {url}"))
    }
}

fn get_bytes(url: &str, cookie: &str) -> Result<Vec<u8>> {
    let resp = ureq::get(url)
        .set("Cookie", cookie)
        .call()
        .map_err(|e| map_err(e, url))?;
    let mut buf = Vec::new();
    let mut reader = resp.into_reader();
    std::io::Read::read_to_end(&mut reader, &mut buf)
        .with_context(|| format!("downloading {url}"))?;
    Ok(buf)
}

/// The `value` array of an OData collection response (odata=nometadata).
fn array(body: &Value) -> Vec<Value> {
    body.get("value")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Encode a server-relative URL for an OData function argument: double single
/// quotes (OData string escaping), then percent-encode reserved characters.
/// The doubled single quotes are kept literal — SharePoint expects `''`.
fn quote(s: &str) -> String {
    let doubled = s.replace('\'', "''");
    let mut out = String::with_capacity(doubled.len());
    for b in doubled.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b'\'' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn map_err(e: ureq::Error, url: &str) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let hint = match code {
                401 | 403 => "  (cookie expired or insufficient rights — refresh FedAuth/rtFa)",
                404 => "  (check the site path)",
                _ => "",
            };
            anyhow!(
                "{url} → {code}{hint}: {}",
                body.chars().take(200).collect::<String>()
            )
        }
        ureq::Error::Transport(t) => anyhow!("cannot reach {url}: {t}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_site_forms() {
        let c = Client::new("tenant.sharepoint.com/sites/Eng", "ck".into()).unwrap();
        assert_eq!(c.host, "tenant.sharepoint.com");
        assert_eq!(c.api, "https://tenant.sharepoint.com/sites/Eng/_api");

        let c2 = Client::new("https://tenant.sharepoint.com/sites/Eng/", "ck".into()).unwrap();
        assert_eq!(c2.api, "https://tenant.sharepoint.com/sites/Eng/_api");

        let root = Client::new("tenant.sharepoint.com", "ck".into()).unwrap();
        assert_eq!(root.api, "https://tenant.sharepoint.com/_api");
    }

    #[test]
    fn quote_doubles_and_encodes() {
        assert_eq!(
            quote("/sites/Eng/Shared Documents"),
            "/sites/Eng/Shared%20Documents"
        );
        assert_eq!(quote("/a/o'brien"), "/a/o''brien");
    }
}
