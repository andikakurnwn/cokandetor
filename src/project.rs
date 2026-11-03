//! Scanning a project folder: file tree, line counts and the derived "level"
//! that decides how big the project's pixel character is.

use std::ffi::OsStr;
use std::path::Path;

pub const MAX_FILES: usize = 15_000;
const MAX_COUNT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default)]
pub struct FileNode {
    pub name: String,
    /// Path relative to the project root, with forward slashes.
    pub rel: String,
    pub is_dir: bool,
    pub lines: usize,
    pub children: Vec<FileNode>,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub files: usize,
    pub lines: usize,
    pub bytes: u64,
    /// (language, lines), most lines first.
    pub langs: Vec<(String, usize)>,
}

#[derive(Clone, Debug, Default)]
pub struct Scan {
    pub root: FileNode,
    pub stats: Stats,
    pub truncated: bool,
}

pub fn is_junk_dir(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(
            "target"
                | "node_modules"
                | ".git"
                | "__pycache__"
                | ".venv"
                | "venv"
                | ".vs"
                | ".idea"
                | ".next"
                | ".gradle"
        )
    )
}

pub fn walker(dir: &Path, max_depth: Option<usize>) -> ignore::Walk {
    let mut b = ignore::WalkBuilder::new(dir);
    b.hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .require_git(false)
        .max_depth(max_depth)
        .filter_entry(|e| !is_junk_dir(e.file_name()))
        .sort_by_file_name(|a, b| a.cmp(b));
    b.build()
}

pub fn is_generated(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(
        lower.as_str(),
        "cargo.lock"
            | "package-lock.json"
            | "yarn.lock"
            | "pnpm-lock.yaml"
            | "poetry.lock"
            | "composer.lock"
            | "gemfile.lock"
            | "go.sum"
            | "uv.lock"
    ) || lower.ends_with(".min.js")
        || lower.ends_with(".min.css")
}

/// Returns `Some(line_count)` for text files, `None` for binary/huge files.
pub fn count_lines(path: &Path, size: u64) -> Option<usize> {
    if size > MAX_COUNT_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return None;
    }
    if bytes.is_empty() {
        return Some(0);
    }
    let nl = bytes.iter().filter(|&&b| b == b'\n').count();
    Some(if bytes.last() == Some(&b'\n') { nl } else { nl + 1 })
}

pub fn scan(root: &Path) -> Scan {
    let mut tree = FileNode {
        name: root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        is_dir: true,
        ..Default::default()
    };
    let mut stats = Stats::default();
    let mut langs: std::collections::HashMap<String, usize> = Default::default();
    let mut truncated = false;

    for entry in walker(root, None).flatten() {
        if entry.depth() == 0 {
            continue;
        }
        if stats.files >= MAX_FILES {
            truncated = true;
            break;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let parts: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        let mut lines = 0;
        if !is_dir {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            stats.files += 1;
            stats.bytes += size;
            if let Some(n) = count_lines(entry.path(), size) {
                lines = n;
                let name = &parts[parts.len() - 1];
                // Lock files are generated, not written: they don't grow the character.
                if !is_generated(name) {
                    stats.lines += n;
                    if let Some(lang) = language_name(name) {
                        *langs.entry(lang.to_string()).or_default() += n;
                    }
                }
            }
        }
        insert(&mut tree, &parts, is_dir, lines);
    }

    sort_tree(&mut tree);
    let mut langs: Vec<_> = langs.into_iter().filter(|(_, n)| *n > 0).collect();
    langs.sort_by_key(|l| std::cmp::Reverse(l.1));
    stats.langs = langs;
    Scan {
        root: tree,
        stats,
        truncated,
    }
}

fn insert(node: &mut FileNode, parts: &[String], is_dir: bool, lines: usize) {
    let mut cur = node;
    for (i, part) in parts.iter().enumerate() {
        let last = i + 1 == parts.len();
        let pos = cur.children.iter().position(|c| &c.name == part);
        let idx = match pos {
            Some(p) => p,
            None => {
                cur.children.push(FileNode {
                    name: part.clone(),
                    rel: parts[..=i].join("/"),
                    is_dir: !last || is_dir,
                    lines: 0,
                    children: vec![],
                });
                cur.children.len() - 1
            }
        };
        cur.lines += lines;
        cur = &mut cur.children[idx];
        if last {
            cur.lines = if is_dir { cur.lines } else { lines };
        }
    }
}

fn sort_tree(node: &mut FileNode) {
    node.children.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    for c in &mut node.children {
        sort_tree(c);
    }
}

/// Level 1..=12, growing roughly with log2 of the line count.
pub fn level_for_lines(lines: usize) -> u32 {
    let mut level = 1;
    let mut threshold = 50usize;
    while lines >= threshold && level < 12 {
        level += 1;
        threshold = threshold * 2 + 50;
    }
    level
}

pub fn fmt_count(n: usize) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=9_999 => format!("{:.1}k", n as f64 / 1e3),
        10_000..=999_999 => format!("{}k", n / 1000),
        _ => format!("{:.1}M", n as f64 / 1e6),
    }
}

pub fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

pub fn language_name(file_name: &str) -> Option<&'static str> {
    let lower = file_name.to_lowercase();
    if lower == "dockerfile" {
        return Some("Docker");
    }
    if lower == "makefile" {
        return Some("Makefile");
    }
    Some(match extension(file_name).as_str() {
        "rs" => "Rust",
        "py" | "pyw" => "Python",
        "js" | "mjs" | "cjs" => "JavaScript",
        "ts" | "mts" => "TypeScript",
        "tsx" | "jsx" => "React",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "go" => "Go",
        "c" | "h" => "C",
        "cpp" | "cc" | "cxx" | "hpp" | "hh" => "C++",
        "cs" => "C#",
        "rb" => "Ruby",
        "php" => "PHP",
        "swift" => "Swift",
        "dart" => "Dart",
        "lua" => "Lua",
        "html" | "htm" => "HTML",
        "css" | "scss" | "sass" | "less" => "CSS",
        "vue" => "Vue",
        "svelte" => "Svelte",
        "json" => "JSON",
        "toml" => "TOML",
        "yaml" | "yml" => "YAML",
        "xml" => "XML",
        "md" | "markdown" => "Markdown",
        "sh" | "bash" | "zsh" => "Shell",
        "ps1" => "PowerShell",
        "bat" | "cmd" => "Batch",
        "sql" => "SQL",
        "txt" => "Text",
        _ => return None,
    })
}

/// A short textual overview for the agent's system prompt.
pub fn overview(scan: &Scan, max_entries: usize) -> String {
    let mut out = format!(
        "{} files, {} lines of text. Languages: {}.\nTop of the file tree:\n",
        scan.stats.files,
        scan.stats.lines,
        if scan.stats.langs.is_empty() {
            "-".to_string()
        } else {
            scan.stats
                .langs
                .iter()
                .take(6)
                .map(|(l, n)| format!("{l} ({n})"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    let mut count = 0;
    fn walk(n: &FileNode, depth: usize, out: &mut String, count: &mut usize, max: usize) {
        for c in &n.children {
            if *count >= max {
                return;
            }
            *count += 1;
            out.push_str(&"  ".repeat(depth));
            out.push_str(&c.name);
            if c.is_dir {
                out.push('/');
            }
            out.push('\n');
            if c.is_dir && depth < 1 {
                walk(c, depth + 1, out, count, max);
            }
        }
    }
    walk(&scan.root, 0, &mut out, &mut count, max_entries);
    if count >= max_entries {
        out.push_str("  ...\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_grow() {
        assert_eq!(level_for_lines(0), 1);
        assert_eq!(level_for_lines(49), 1);
        assert_eq!(level_for_lines(50), 2);
        assert!(level_for_lines(10_000) > level_for_lines(1_000));
        assert_eq!(level_for_lines(10_000_000), 12);
    }
}
