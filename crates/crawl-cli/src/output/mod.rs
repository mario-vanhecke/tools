use serde::Serialize;

pub fn emit_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    let s = serde_json::to_string_pretty(value)?;
    println!("{s}");
    Ok(())
}

/// Print a table of discovered documents.
pub fn print_documents(
    rows: &[crawl_core::registry::DocumentRow],
    names: &std::collections::HashMap<i64, String>,
) {
    if rows.is_empty() {
        println!("No matching documents.");
        return;
    }
    let (h_status, h_source, h_size, h_doc) = ("STATUS", "SOURCE", "SIZE", "DOCUMENT");
    println!("{h_status:<10} {h_source:<14} {h_size:<10} {h_doc}");
    for d in rows {
        let src = names
            .get(&d.source_id)
            .cloned()
            .unwrap_or_else(|| d.source_id.to_string());
        let loc = d.rel_path.clone().unwrap_or_else(|| d.uri.clone());
        println!(
            "{:<10} {:<14} {:<10} {}",
            d.status.as_str(),
            truncate(&src, 14),
            human_size(d.size),
            loc
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

fn human_size(bytes: Option<i64>) -> String {
    match bytes {
        None => "-".to_string(),
        Some(b) => {
            let b = b as f64;
            const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
            let mut v = b;
            let mut u = 0;
            while v >= 1024.0 && u < UNITS.len() - 1 {
                v /= 1024.0;
                u += 1;
            }
            if u == 0 {
                format!("{}{}", v as i64, UNITS[u])
            } else {
                format!("{:.1}{}", v, UNITS[u])
            }
        }
    }
}

/// Render an epoch-millis timestamp as a local datetime, or "never".
pub fn fmt_time(ms: Option<i64>) -> String {
    match ms {
        Some(ms) => chrono::DateTime::from_timestamp_millis(ms)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| ms.to_string()),
        None => "never".to_string(),
    }
}
