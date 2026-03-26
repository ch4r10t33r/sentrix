fn main() {
    // ── Version: read from root package.json (npm is the source of truth) ────
    //
    // Semantic-release bumps package.json on every merge to main.
    // Cargo.toml is kept in sync by the release CI, but reading package.json
    // here means the binary always reports the npm version even if Cargo.toml
    // lags by one commit.
    let npm_version = read_npm_version().unwrap_or_else(|| {
        // Fallback: use the version baked into Cargo.toml at workspace level.
        std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into())
    });
    println!("cargo:rustc-env=SENTRIX_VERSION={npm_version}");
    println!("cargo:rerun-if-changed=../package.json");

    // ── Build date ────────────────────────────────────────────────────────────
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    println!("cargo:rustc-env=SENTRIX_BUILD_DATE={date}");

    // ── Template files (debug-embed fast iteration) ───────────────────────────
    println!("cargo:rerun-if-changed=../templates");
}

/// Parse `"version": "X.Y.Z"` from `../package.json` without pulling in a
/// JSON library as a build dependency — a simple string scan is sufficient.
fn read_npm_version() -> Option<String> {
    let content = std::fs::read_to_string("../package.json").ok()?;
    // Find the first "version" key (it is always near the top of package.json).
    let key_pos = content.find("\"version\"")?;
    let after = &content[key_pos + "\"version\"".len()..];
    let colon = after.find(':')? + 1;
    let trimmed = after[colon..].trim_start();
    let q_start = trimmed.find('"')? + 1;
    let rest = &trimmed[q_start..];
    let q_end = rest.find('"')?;
    let version = rest[..q_end].trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}
