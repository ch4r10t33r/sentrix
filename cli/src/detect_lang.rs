use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lang {
    TypeScript,
    Python,
    Rust,
    Zig,
}

impl Lang {
    /// Parse from a CLI flag value (case-insensitive).
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "typescript" | "ts" => Some(Self::TypeScript),
            "python" | "py" => Some(Self::Python),
            "rust" | "rs" => Some(Self::Rust),
            "zig" => Some(Self::Zig),
            _ => None,
        }
    }

    /// The subfolder name under `templates/`.
    pub fn template_dir(&self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Python => "python",
            Self::Rust => "rust",
            Self::Zig => "zig",
        }
    }

    pub fn as_str(&self) -> &'static str {
        self.template_dir()
    }
}

impl std::fmt::Display for Lang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Deserialize)]
struct BorgkitConfig {
    language: Option<String>,
}

/// Detect the project language for the given directory.
///
/// Priority:
///   1. Explicit `--lang` flag (caller passes `Some`)
///   2. `borgkit.config.json` → `language` field
///   3. Sniff for `tsconfig.json` / `requirements.txt` / `Cargo.toml` / `build.zig`
///   4. Default: TypeScript
pub fn detect(explicit: Option<&str>, dir: &Path) -> Result<Lang> {
    if let Some(s) = explicit {
        return Lang::from_str(s).with_context(|| {
            format!(
                "Unknown language '{}'. Valid: typescript, python, rust, zig",
                s
            )
        });
    }

    // 2. borgkit.config.json
    let config_path = dir.join("borgkit.config.json");
    if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)?;
        if let Ok(cfg) = serde_json::from_str::<BorgkitConfig>(&raw) {
            if let Some(lang_str) = cfg.language {
                if let Some(lang) = Lang::from_str(&lang_str) {
                    return Ok(lang);
                }
            }
        }
    }

    // 3. Sniff
    let sniff: &[(&str, Lang)] = &[
        ("tsconfig.json", Lang::TypeScript),
        ("requirements.txt", Lang::Python),
        ("Cargo.toml", Lang::Rust),
        ("build.zig", Lang::Zig),
    ];
    for (file, lang) in sniff {
        if dir.join(file).exists() {
            return Ok(lang.clone());
        }
    }

    Ok(Lang::TypeScript)
}

/// Find the project root by walking up from `start` looking for `borgkit.config.json`.
/// Falls back to `start` if not found.
pub fn find_project_root(start: &Path) -> PathBuf {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join("borgkit.config.json").exists() {
            return dir;
        }
        if !dir.pop() {
            break;
        }
    }
    start.to_path_buf()
}
