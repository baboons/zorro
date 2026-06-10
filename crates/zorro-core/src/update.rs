//! A tiny, dependency-free "is there a newer release?" check.
//!
//! Queries the GitHub Releases API via `curl` (so we avoid pulling in an HTTP
//! stack), extracts the latest tag, and compares it to the running version. The
//! app runs [`check_latest`] on a background thread and shows a banner if a
//! newer version exists — it never updates anything itself.

/// The latest released version (tag with any leading `v` stripped), or `None`
/// if the check fails (offline, rate-limited, no releases yet, …).
pub fn check_latest(repo: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let output = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            "Accept: application/vnd.github+json",
            "-H",
            "User-Agent: zorro",
            &url,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_latest_tag(&String::from_utf8_lossy(&output.stdout))
}

/// Pull `tag_name` out of the GitHub release JSON, with any leading `v` removed.
pub fn parse_latest_tag(json: &str) -> Option<String> {
    let key = "\"tag_name\"";
    let after_key = &json[json.find(key)? + key.len()..];
    let after_colon = &after_key[after_key.find(':')? + 1..];
    let open = after_colon.find('"')? + 1;
    let close = after_colon[open..].find('"')? + open;
    let tag = after_colon[open..close].trim().trim_start_matches('v');
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

/// Whether `latest` is a strictly newer version than `current`. Compares the
/// dotted numeric components (ignoring any `-prerelease` suffix).
pub fn is_newer(latest: &str, current: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.split('-')
            .next()
            .unwrap_or(v)
            .split('.')
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect()
    }
    let (l, c) = (parts(latest), parts(current));
    for i in 0..l.len().max(c.len()) {
        let a = l.get(i).copied().unwrap_or(0);
        let b = c.get(i).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}
