//! Project skills: a Markdown "map" of each project kept in `.ckdtr/skill.md`
//! (the same idea as `.claude/` + CLAUDE.md). The agent writes it once when a
//! project joins the office; every later task gets it in the system prompt, so
//! the agent starts oriented instead of spending tool calls exploring.

use std::path::{Path, PathBuf};

pub const DIR: &str = ".ckdtr";
pub const REL: &str = ".ckdtr/skill.md";
const MAX_SKILL: usize = 32 * 1024;
const MAX_INSTRUCTIONS: usize = 16 * 1024;

/// Marker the skill-writing system prompt starts with.
pub const MODE_MARKER: &str = "# Cokandetor skill mode";

/// What the user sees in the chat when a skill is (re)built.
pub const TASK: &str = "Pelajari project ini lalu tulis skill project ke .ckdtr/skill.md.";

pub fn path(root: &Path) -> PathBuf {
    root.join(DIR).join("skill.md")
}

pub fn exists(root: &Path) -> bool {
    path(root).is_file()
}

fn read_capped(path: &Path, max: usize) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return None;
    }
    if text.len() <= max {
        return Some(text.to_string());
    }
    let mut cut = max;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    Some(format!("{}\n\n[... dipotong, {} byte lagi]", &text[..cut], text.len() - cut))
}

/// The project's skill, if one was written.
pub fn read(root: &Path) -> Option<String> {
    read_capped(&path(root), MAX_SKILL)
}

/// Instruction files other tools keep in a project (Claude Code, AGENTS.md).
pub fn instructions(root: &Path) -> Vec<(String, String)> {
    ["CLAUDE.md", ".claude/CLAUDE.md", "AGENTS.md"]
        .iter()
        .filter_map(|name| read_capped(&root.join(name), MAX_INSTRUCTIONS).map(|t| (name.to_string(), t)))
        .collect()
}

/// A field from the skill's YAML front matter (`generated`, `description`, …).
pub fn front_matter(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some((k, v)) = line.split_once(':')
            && k.trim() == key {
                let v = v.trim().trim_matches('"').trim();
                return (!v.is_empty()).then(|| v.to_string());
            }
    }
    None
}

/// System-prompt section for the skill-writing run.
pub fn writing_instructions(project: &str, today: &str) -> String {
    format!(
        "{MODE_MARKER}\n\
         Your only job now is onboarding: explore this project with list_files, read_file and search_files, \
         then write a project skill to `{REL}` with write_file. Do not create or change any other file.\n\n\
         Every future task in this editor starts by reading this skill as the agent's map of the project, \
         so make it accurate, specific and short (about 60–150 lines). Only state facts you verified in the \
         files; never guess commands or versions. Use exactly this structure:\n\n\
         ---\n\
         name: {project}\n\
         description: <one sentence: what the project is and what it is built with>\n\
         generated: {today}\n\
         ---\n\
         # {project}\n\
         ## Ringkasan\n\
         ## Teknologi\n\
         ## Struktur\n\
         (important folders/files and what they hold)\n\
         ## Menjalankan, build & test\n\
         (exact commands found in the repo: package.json scripts, Cargo.toml, Makefile, README, …)\n\
         ## Konvensi\n\
         (code style, naming, patterns and frameworks to follow)\n\
         ## Titik masuk & file kunci\n\
         ## Catatan untuk agent\n\
         (gotchas, generated files not to edit, required env vars, anything a new coder must know)\n\n\
         Write the skill in Indonesian. When it is saved, reply with one short sentence."
    )
}

/// Is this tool call allowed while writing a skill? Only reads, and writes to the skill file.
pub fn allowed_in_skill_mode(root: &Path, tool: &str, input: &serde_json::Value) -> bool {
    match tool {
        "list_files" | "read_file" | "search_files" => true,
        "write_file" | "edit_file" => input
            .get("path")
            .and_then(|p| p.as_str())
            .and_then(|p| crate::agent::tools::resolve(root, p).ok())
            .is_some_and(|p| p == path(root)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_front_matter() {
        let text = "---\nname: toko\ndescription: \"Kasir Flutter\"\ngenerated: 2026-11-03\n---\n# toko\n";
        assert_eq!(front_matter(text, "generated").as_deref(), Some("2026-11-03"));
        assert_eq!(front_matter(text, "description").as_deref(), Some("Kasir Flutter"));
        assert_eq!(front_matter("# no front matter", "generated"), None);
    }

    #[test]
    fn skill_mode_only_writes_the_skill() {
        let root = Path::new("C:/proj");
        assert!(allowed_in_skill_mode(root, "read_file", &json!({"path": "src/main.rs"})));
        assert!(allowed_in_skill_mode(root, "write_file", &json!({"path": ".ckdtr/skill.md"})));
        assert!(allowed_in_skill_mode(root, "write_file", &json!({"path": "./.ckdtr//skill.md"})));
        assert!(!allowed_in_skill_mode(root, "write_file", &json!({"path": "src/main.rs"})));
        assert!(!allowed_in_skill_mode(root, "delete_file", &json!({"path": ".ckdtr/skill.md"})));
        assert!(!allowed_in_skill_mode(root, "run_command", &json!({"command": "dir"})));
    }

    #[test]
    fn long_skills_are_capped_on_a_char_boundary() {
        let dir = std::env::temp_dir().join(format!("cokandetor-skill-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(DIR)).unwrap();
        std::fs::write(path(&dir), "é".repeat(MAX_SKILL)).unwrap();
        let text = read(&dir).unwrap();
        assert!(text.contains("dipotong"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
