//! The file-system tools the agent can call. Every path is resolved inside the
//! project root; anything that escapes it is rejected.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::project;

pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub struct Outcome {
    pub content: String,
    pub is_error: bool,
    pub summary: String,
    pub change: Option<FileChange>,
}

impl Outcome {
    fn ok(content: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
            summary: summary.into(),
            change: None,
        }
    }

    fn err(msg: impl Into<String>) -> Self {
        let msg = msg.into();
        Self {
            summary: msg.clone(),
            content: format!("Error: {msg}"),
            is_error: true,
            change: None,
        }
    }
}

pub fn specs(allow_commands: bool) -> Vec<ToolSpec> {
    let mut v = vec![
        ToolSpec {
            name: "list_files",
            description: "List files and folders in the project (respects .gitignore). Returns an indented tree with line counts.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Folder relative to the project root. Default: the root."},
                    "max_depth": {"type": "integer", "description": "How deep to descend. Default 4."}
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "read_file",
            description: "Read a text file. Output lines are prefixed with their 1-based line number and a tab; the prefix is not part of the file.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File path relative to the project root."},
                    "start_line": {"type": "integer", "description": "First line to return (1-based)."},
                    "end_line": {"type": "integer", "description": "Last line to return (inclusive)."}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "write_file",
            description: "Create a file or replace its whole content. Parent folders are created automatically.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File path relative to the project root."},
                    "content": {"type": "string", "description": "The complete new file content."}
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "edit_file",
            description: "Replace an exact snippet of an existing file. old_text must match the file exactly (including whitespace) and be unique unless replace_all is true.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File path relative to the project root."},
                    "old_text": {"type": "string", "description": "Exact text to find."},
                    "new_text": {"type": "string", "description": "Replacement text."},
                    "replace_all": {"type": "boolean", "description": "Replace every occurrence instead of requiring a unique match."}
                },
                "required": ["path", "old_text", "new_text"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "delete_file",
            description: "Delete a file (or an empty folder).",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path relative to the project root."}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "search_files",
            description: "Search text across project files. Returns matching lines as path:line: text.",
            schema: json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Plain text to search for."},
                    "path": {"type": "string", "description": "Folder to search in. Default: the root."},
                    "case_sensitive": {"type": "boolean", "description": "Default false."}
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
    ];
    if allow_commands {
        v.push(ToolSpec {
            name: "run_command",
            description: "Run a shell command (Windows cmd.exe) in the project root and return its output. Use for builds and tests.",
            schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "The command line to run."},
                    "timeout_secs": {"type": "integer", "description": "Timeout in seconds (default 120, max 600)."}
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        });
    }
    v
}

/// Resolve a model-supplied path inside `root`.
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim();
    if rel.is_empty() {
        return Err("path is empty".into());
    }
    let p = Path::new(rel);
    let rel_path = if p.is_absolute() || p.has_root() {
        strip_root(root, rel).ok_or_else(|| format!("'{rel}' is outside the project"))?
    } else {
        p.to_path_buf()
    };
    let mut out = root.to_path_buf();
    let mut depth = 0usize;
    for comp in rel_path.components() {
        match comp {
            Component::Normal(c) => {
                out.push(c);
                depth += 1;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if depth == 0 {
                    return Err(format!("'{rel}' is outside the project"));
                }
                out.pop();
                depth -= 1;
            }
            _ => return Err(format!("invalid path '{rel}'")),
        }
    }
    Ok(out)
}

fn strip_root(root: &Path, abs: &str) -> Option<PathBuf> {
    let r = root.to_string_lossy().replace('\\', "/");
    let r = r.trim_end_matches('/');
    let s = abs.replace('\\', "/");
    let sb = s.as_bytes();
    if sb.len() >= r.len() && sb[..r.len()].eq_ignore_ascii_case(r.as_bytes()) {
        let rest = s.get(r.len()..)?;
        if rest.is_empty() {
            return Some(PathBuf::from("."));
        }
        if let Some(rest) = rest.strip_prefix('/') {
            return Some(PathBuf::from(rest));
        }
    }
    None
}

pub fn rel_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn str_arg<'a>(input: &'a Value, key: &str) -> Result<&'a str, String> {
    match input.get(key) {
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(format!("'{key}' must be a string")),
        None => Err(format!("missing required field '{key}'")),
    }
}

fn opt_str<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(Value::as_str)
}

fn opt_u64(input: &Value, key: &str) -> Option<u64> {
    input.get(key).and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64)))
}

fn opt_bool(input: &Value, key: &str) -> bool {
    input.get(key).and_then(Value::as_bool).unwrap_or(false)
}

pub fn execute(root: &Path, name: &str, input: &Value, allow_commands: bool, cancel: &AtomicBool) -> Outcome {
    if !input.is_object() {
        return Outcome::err("tool input must be a JSON object");
    }
    let res = match name {
        "list_files" => list_files(root, input),
        "read_file" => read_file(root, input),
        "write_file" => write_file(root, input),
        "edit_file" => edit_file(root, input),
        "delete_file" => delete_file(root, input),
        "search_files" => search_files(root, input),
        "run_command" if allow_commands => run_command(root, input, cancel),
        "run_command" => Err("running commands is disabled in Cokandetor settings".into()),
        other => Err(format!("unknown tool '{other}'")),
    };
    res.unwrap_or_else(Outcome::err)
}

fn list_files(root: &Path, input: &Value) -> Result<Outcome, String> {
    let dir = resolve(root, opt_str(input, "path").unwrap_or("."))?;
    if !dir.is_dir() {
        return Err(format!("'{}' is not a folder", rel_display(root, &dir)));
    }
    let depth = opt_u64(input, "max_depth").unwrap_or(4).clamp(1, 12) as usize;
    let mut out = String::new();
    let mut n = 0;
    for e in project::walker(&dir, Some(depth)).flatten() {
        if e.depth() == 0 {
            continue;
        }
        if n >= 400 {
            out.push_str("... (truncated, narrow the path or depth)\n");
            break;
        }
        n += 1;
        let name = e.file_name().to_string_lossy();
        out.push_str(&"  ".repeat(e.depth() - 1));
        if e.file_type().is_some_and(|t| t.is_dir()) {
            out.push_str(&format!("{name}/\n"));
        } else {
            let size = e.metadata().map(|m| m.len()).unwrap_or(0);
            match project::count_lines(e.path(), size) {
                Some(l) => out.push_str(&format!("{name} ({l} lines)\n")),
                None => out.push_str(&format!("{name} (binary, {size} bytes)\n")),
            }
        }
    }
    if out.is_empty() {
        out = "(empty)".into();
    }
    Ok(Outcome::ok(out, format!("{n} entri")))
}

fn read_text(path: &Path) -> Result<String, String> {
    read_text_mode(path, false)
}

/// `strict` refuses non-UTF-8 files instead of decoding them lossily, so an
/// edit never replaces their bytes with U+FFFD.
fn read_text_mode(path: &Path, strict: bool) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot read: {e}"))?;
    if meta.is_dir() {
        return Err("that path is a folder; use list_files".into());
    }
    if meta.len() > 2 * 1024 * 1024 {
        return Err(format!("file is too large ({} bytes)", meta.len()));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read: {e}"))?;
    if bytes.iter().take(8000).any(|&b| b == 0) {
        return Err("file looks binary".into());
    }
    match String::from_utf8(bytes) {
        Ok(s) => Ok(s),
        Err(_) if strict => Err("file is not valid UTF-8; refusing to edit it to avoid corrupting it".into()),
        Err(e) => Ok(String::from_utf8_lossy(e.as_bytes()).into_owned()),
    }
}

fn read_file(root: &Path, input: &Value) -> Result<Outcome, String> {
    let rel = str_arg(input, "path")?;
    let path = resolve(root, rel)?;
    let text = read_text(&path)?;
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let start = opt_u64(input, "start_line").unwrap_or(1).max(1) as usize;
    let explicit_end = opt_u64(input, "end_line").map(|e| e as usize);
    let end = explicit_end.unwrap_or(start.saturating_add(1999)).min(total);
    let mut out = String::new();
    if total == 0 {
        out.push_str("(empty file)");
    }
    for (i, line) in lines.iter().enumerate().take(end).skip(start - 1) {
        out.push_str(&format!("{}\t{}\n", i + 1, line));
    }
    if end < total {
        out.push_str(&format!(
            "[... {} more lines; read again with start_line={}]\n",
            total - end,
            end + 1
        ));
    }
    Ok(Outcome::ok(out, format!("{total} baris")))
}

fn write_file(root: &Path, input: &Value) -> Result<Outcome, String> {
    let rel = str_arg(input, "path")?;
    let content = str_arg(input, "content")?;
    let path = resolve(root, rel)?;
    if path.is_dir() {
        return Err("that path is a folder".into());
    }
    let before = std::fs::read_to_string(&path).ok();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create folder: {e}"))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("cannot write: {e}"))?;
    let shown = rel_display(root, &path);
    let lines = content.lines().count();
    let verb = if before.is_some() { "Updated" } else { "Created" };
    Ok(Outcome {
        content: format!("{verb} {shown} ({lines} lines)."),
        is_error: false,
        summary: format!("{} · {lines} baris", if before.is_some() { "diperbarui" } else { "dibuat" }),
        change: Some(FileChange {
            path: shown,
            before,
            after: Some(content.to_string()),
        }),
    })
}

fn edit_file(root: &Path, input: &Value) -> Result<Outcome, String> {
    let rel = str_arg(input, "path")?;
    let old = str_arg(input, "old_text")?;
    let new = str_arg(input, "new_text")?;
    let all = opt_bool(input, "replace_all");
    if old.is_empty() {
        return Err("old_text must not be empty".into());
    }
    let path = resolve(root, rel)?;
    let before = read_text_mode(&path, true)?;

    let mut old_s = old.to_string();
    let mut new_s = new.to_string();
    let mut count = before.matches(&old_s).count();
    if count == 0 && before.contains("\r\n") && !old.contains("\r\n") {
        // The model usually writes LF; retry against a CRLF file.
        old_s = old.replace('\n', "\r\n");
        new_s = new.replace('\n', "\r\n");
        count = before.matches(&old_s).count();
    }
    if count == 0 {
        return Err("old_text was not found in the file; read the file again and copy the text exactly".into());
    }
    if count > 1 && !all {
        return Err(format!(
            "old_text matches {count} places; add more surrounding context or set replace_all"
        ));
    }
    let after = if all {
        before.replace(&old_s, &new_s)
    } else {
        before.replacen(&old_s, &new_s, 1)
    };
    std::fs::write(&path, &after).map_err(|e| format!("cannot write: {e}"))?;
    let shown = rel_display(root, &path);
    Ok(Outcome {
        content: format!("Edited {shown} ({count} replacement{}).", if count == 1 { "" } else { "s" }),
        is_error: false,
        summary: format!("{count} perubahan"),
        change: Some(FileChange {
            path: shown,
            before: Some(before),
            after: Some(after),
        }),
    })
}

fn delete_file(root: &Path, input: &Value) -> Result<Outcome, String> {
    let rel = str_arg(input, "path")?;
    let path = resolve(root, rel)?;
    if path == root {
        return Err("refusing to delete the project root".into());
    }
    let shown = rel_display(root, &path);
    if path.is_dir() {
        std::fs::remove_dir(&path).map_err(|e| format!("cannot delete folder (must be empty): {e}"))?;
        return Ok(Outcome::ok(format!("Deleted folder {shown}."), "folder dihapus"));
    }
    let before = std::fs::read_to_string(&path).ok();
    std::fs::remove_file(&path).map_err(|e| format!("cannot delete: {e}"))?;
    Ok(Outcome {
        content: format!("Deleted {shown}."),
        is_error: false,
        summary: "dihapus".into(),
        change: Some(FileChange {
            path: shown,
            before,
            after: None,
        }),
    })
}

fn search_files(root: &Path, input: &Value) -> Result<Outcome, String> {
    let query = str_arg(input, "query")?;
    if query.is_empty() {
        return Err("query must not be empty".into());
    }
    let dir = resolve(root, opt_str(input, "path").unwrap_or("."))?;
    let case = opt_bool(input, "case_sensitive");
    let needle = if case { query.to_string() } else { query.to_lowercase() };
    let mut out = String::new();
    let mut hits = 0;
    'outer: for e in project::walker(&dir, None).flatten() {
        if !e.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(text) = read_text(e.path()) else { continue };
        let shown = rel_display(root, e.path());
        for (i, line) in text.lines().enumerate() {
            let hay = if case { line.to_string() } else { line.to_lowercase() };
            if hay.contains(&needle) {
                hits += 1;
                let t: String = line.trim().chars().take(200).collect();
                out.push_str(&format!("{shown}:{}: {t}\n", i + 1));
                if hits >= 150 {
                    out.push_str("... (more matches truncated)\n");
                    break 'outer;
                }
            }
        }
    }
    if hits == 0 {
        out = "No matches.".into();
    }
    Ok(Outcome::ok(out, format!("{hits} hasil")))
}

fn run_command(root: &Path, input: &Value, cancel: &AtomicBool) -> Result<Outcome, String> {
    let command = str_arg(input, "command")?;
    let timeout = Duration::from_secs(opt_u64(input, "timeout_secs").unwrap_or(120).clamp(1, 600));

    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = std::process::Command::new("cmd");
        c.arg("/C").raw_arg(command);
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = std::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    cmd.current_dir(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("cannot start command: {e}"))?;

    // Readers stream chunks over channels: a grandchild that keeps the pipe
    // open (a dev server, a GUI app) must not block us after the command ends.
    let pipe = |r: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let Some(mut r) = r else { return };
            let mut buf = [0u8; 8192];
            while let Ok(n) = r.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        rx
    };
    let out_rx = pipe(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let err_rx = pipe(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));

    let started = Instant::now();
    let mut killed = None;
    let status = loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            break Some(st);
        }
        if cancel.load(Ordering::Relaxed) {
            killed = Some("cancelled by user");
        } else if started.elapsed() > timeout {
            killed = Some("timed out");
        }
        if killed.is_some() {
            kill_tree(&mut child);
            break None;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    // Collect what was written, giving stragglers a short grace period.
    let collect = |rx: std::sync::mpsc::Receiver<Vec<u8>>| {
        let deadline = Instant::now() + Duration::from_millis(1500);
        let mut buf = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(chunk) => buf.extend_from_slice(&chunk),
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&buf).into_owned()
    };
    let stdout = collect(out_rx);
    let stderr = collect(err_rx);
    let mut text = String::new();
    if !stdout.is_empty() {
        text.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push_str("\n--- stderr ---\n");
        }
        text.push_str(&stderr);
    }
    const LIMIT: usize = 20_000;
    if text.len() > LIMIT {
        let mut cut = text.len() - LIMIT;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        text = format!("[... output truncated ...]\n{}", &text[cut..]);
    }
    let code = status.and_then(|s| s.code());
    let head = match (killed, code) {
        (Some(why), _) => format!("Command {why}."),
        (None, Some(c)) => format!("Exit code {c}."),
        (None, None) => "Command ended.".to_string(),
    };
    let failed = killed.is_some() || code.is_some_and(|c| c != 0);
    Ok(Outcome {
        content: format!("{head}\n{text}"),
        is_error: failed,
        summary: head,
        change: None,
    })
}

/// Stops a command and everything it started (cmd.exe's children survive a
/// plain `kill`).
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_refuses_non_utf8() {
        let dir = std::env::temp_dir().join(format!("cokandetor-edit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("latin1.txt"), b"caf\xe9 = 1\n").unwrap();
        let out = execute(
            &dir,
            "edit_file",
            &json!({"path": "latin1.txt", "old_text": "1", "new_text": "2"}),
            false,
            &AtomicBool::new(false),
        );
        assert!(out.is_error);
        assert_eq!(std::fs::read(dir.join("latin1.txt")).unwrap(), b"caf\xe9 = 1\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn command_with_lingering_child_returns() {
        let dir = std::env::temp_dir();
        let started = Instant::now();
        // `start /b` leaves a child holding the output pipe after cmd exits.
        let out = execute(
            &dir,
            "run_command",
            &json!({"command": "echo hi & start /b ping -n 30 127.0.0.1 >nul", "timeout_secs": 20}),
            true,
            &AtomicBool::new(false),
        );
        assert!(out.content.contains("hi"), "{}", out.content);
        assert!(started.elapsed() < Duration::from_secs(15), "took {:?}", started.elapsed());
    }

    #[test]
    fn resolve_stays_inside() {
        let root = Path::new("C:/proj");
        assert!(resolve(root, "src/main.rs").is_ok());
        assert!(resolve(root, "./a/../b.txt").is_ok());
        assert!(resolve(root, "../escape.txt").is_err());
        assert!(resolve(root, "a/../../escape.txt").is_err());
        assert!(resolve(root, "C:/proj/src/x.rs").is_ok());
        assert!(resolve(root, "C:/other/x.rs").is_err());
    }
}
