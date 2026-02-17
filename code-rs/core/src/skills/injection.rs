use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;

use code_protocol::models::ResponseItem;
use tokio::fs;

use crate::skills::model::SkillMetadata;
use crate::user_instructions::SkillInstructions;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MentionedSkill {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
}

pub(crate) struct SkillMentionOutcome {
    pub(crate) mentioned: Vec<MentionedSkill>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Default)]
pub(crate) struct SkillInjections {
    pub(crate) items: Vec<ResponseItem>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Default)]
struct ToolMentions<'a> {
    names: HashSet<&'a str>,
    paths: HashSet<&'a str>,
    plain_names: HashSet<&'a str>,
}

impl<'a> ToolMentions<'a> {
    fn is_empty(&self) -> bool {
        self.names.is_empty() && self.paths.is_empty()
    }
}

pub(crate) fn collect_explicit_skill_mentions(
    messages: &[String],
    skills: &[SkillMetadata],
) -> SkillMentionOutcome {
    if messages.is_empty() || skills.is_empty() {
        return SkillMentionOutcome {
            mentioned: Vec::new(),
            warnings: Vec::new(),
        };
    }

    let mentions = collect_tool_mentions_from_messages(messages);
    if mentions.is_empty() {
        return SkillMentionOutcome {
            mentioned: Vec::new(),
            warnings: Vec::new(),
        };
    }

    let mut skill_name_counts: HashMap<String, usize> = HashMap::new();
    for skill in skills {
        *skill_name_counts
            .entry(skill.name.to_ascii_lowercase())
            .or_insert(0) += 1;
    }

    let mention_skill_paths: HashSet<String> = mentions
        .paths
        .iter()
        .copied()
        .filter(|path| is_skill_path_like(path))
        .map(|path| normalize_skill_path(path).replace('\\', "/"))
        .collect();

    let mut selected: Vec<MentionedSkill> = Vec::new();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    let mut warned_ambiguous: HashSet<String> = HashSet::new();
    let mut warnings: Vec<String> = Vec::new();

    // Prefer explicit path mentions first.
    if !mention_skill_paths.is_empty() {
        for skill in skills {
            let path_str = normalize_path_for_compare(skill.path.as_path());
            if mention_skill_paths.contains(path_str.as_str()) && seen_paths.insert(skill.path.clone()) {
                selected.push(MentionedSkill {
                    name: skill.name.clone(),
                    path: skill.path.clone(),
                });
            }
        }
    }

    let mention_plain_names_lower: HashSet<String> = mentions
        .plain_names
        .iter()
        .copied()
        .map(|name| name.to_ascii_lowercase())
        .collect();

    if mention_plain_names_lower.is_empty() {
        return SkillMentionOutcome {
            mentioned: selected,
            warnings: Vec::new(),
        };
    }

    for skill in skills {
        if seen_paths.contains(&skill.path) {
            continue;
        }

        let skill_lower = skill.name.to_ascii_lowercase();
        if !mention_plain_names_lower.contains(skill_lower.as_str()) {
            continue;
        }

        let count = skill_name_counts.get(skill_lower.as_str()).copied().unwrap_or(0);
        if count != 1 {
            if warned_ambiguous.insert(skill_lower.clone()) {
                let mut paths = skills
                    .iter()
                    .filter(|candidate| candidate.name.to_ascii_lowercase() == skill_lower)
                    .map(|candidate| candidate.path.to_string_lossy().into_owned())
                    .collect::<Vec<_>>();
                paths.sort();
                let joined = paths.join(", ");
                warnings.push(format!(
                    "Ambiguous skill mention `${skill_lower}` matched multiple skills: {joined}. Use a linked mention to disambiguate: `[$skill_lower](skill://<full path>)`."
                ));
            }
            continue;
        }

        if seen_paths.insert(skill.path.clone()) {
            selected.push(MentionedSkill {
                name: skill.name.clone(),
                path: skill.path.clone(),
            });
        }
    }

    SkillMentionOutcome {
        mentioned: selected,
        warnings,
    }
}

pub(crate) async fn build_skill_injections(skills: &[MentionedSkill]) -> SkillInjections {
    if skills.is_empty() {
        return SkillInjections::default();
    }

    let mut items: Vec<ResponseItem> = Vec::with_capacity(skills.len());
    let mut warnings: Vec<String> = Vec::new();

    for skill in skills {
        match fs::read_to_string(&skill.path).await {
            Ok(contents) => {
                let path = skill.path.to_string_lossy().replace('\\', "/");
                items.push(
                    SkillInstructions {
                        name: skill.name.clone(),
                        path,
                        contents,
                    }
                    .into(),
                );
            }
            Err(err) => {
                warnings.push(format!(
                    "Failed to load skill `{}` at {}: {err:#}",
                    skill.name,
                    skill.path.display()
                ));
            }
        }
    }

    SkillInjections { items, warnings }
}

fn collect_tool_mentions_from_messages<'a>(messages: &'a [String]) -> ToolMentions<'a> {
    let mut out = ToolMentions::default();
    for message in messages {
        let mentions = extract_tool_mentions(message);
        out.names.extend(mentions.names);
        out.paths.extend(mentions.paths);
        out.plain_names.extend(mentions.plain_names);
    }
    out
}

/// Extract `$tool-name` mentions from a single text input.
///
/// Supports explicit resource links in the form `[$tool-name](resource path)`.
fn extract_tool_mentions(text: &str) -> ToolMentions<'_> {
    let text_bytes = text.as_bytes();
    let mut mentioned_names: HashSet<&str> = HashSet::new();
    let mut mentioned_paths: HashSet<&str> = HashSet::new();
    let mut plain_names: HashSet<&str> = HashSet::new();

    let mut index = 0;
    while index < text_bytes.len() {
        let byte = text_bytes[index];
        if byte == b'['
            && let Some((name, path, end_index)) =
                parse_linked_tool_mention(text, text_bytes, index)
        {
            if !is_common_env_var(name) {
                mentioned_names.insert(name);
                mentioned_paths.insert(path);
            }
            index = end_index;
            continue;
        }

        if byte != b'$' {
            index += 1;
            continue;
        }

        let name_start = index + 1;
        let Some(first_name_byte) = text_bytes.get(name_start) else {
            index += 1;
            continue;
        };
        if !is_mention_name_char(*first_name_byte) {
            index += 1;
            continue;
        }

        let mut name_end = name_start + 1;
        while let Some(next_byte) = text_bytes.get(name_end)
            && is_mention_name_char(*next_byte)
        {
            name_end += 1;
        }

        let name = &text[name_start..name_end];
        if !is_common_env_var(name) {
            mentioned_names.insert(name);
            plain_names.insert(name);
        }
        index = name_end;
    }

    ToolMentions {
        names: mentioned_names,
        paths: mentioned_paths,
        plain_names,
    }
}

fn parse_linked_tool_mention<'a>(
    text: &'a str,
    text_bytes: &[u8],
    start: usize,
) -> Option<(&'a str, &'a str, usize)> {
    let dollar_index = start + 1;
    if text_bytes.get(dollar_index) != Some(&b'$') {
        return None;
    }

    let name_start = dollar_index + 1;
    let first_name_byte = text_bytes.get(name_start)?;
    if !is_mention_name_char(*first_name_byte) {
        return None;
    }

    let mut name_end = name_start + 1;
    while let Some(next_byte) = text_bytes.get(name_end)
        && is_mention_name_char(*next_byte)
    {
        name_end += 1;
    }

    if text_bytes.get(name_end) != Some(&b']') {
        return None;
    }

    let mut path_start = name_end + 1;
    while let Some(next_byte) = text_bytes.get(path_start)
        && next_byte.is_ascii_whitespace()
    {
        path_start += 1;
    }
    if text_bytes.get(path_start) != Some(&b'(') {
        return None;
    }

    let mut path_end = path_start + 1;
    while let Some(next_byte) = text_bytes.get(path_end) && *next_byte != b')' {
        path_end += 1;
    }
    if text_bytes.get(path_end) != Some(&b')') {
        return None;
    }

    let path = text[path_start + 1..path_end].trim();
    if path.is_empty() {
        return None;
    }

    let name = &text[name_start..name_end];
    Some((name, path, path_end + 1))
}

fn is_common_env_var(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "PATH"
            | "HOME"
            | "USER"
            | "SHELL"
            | "PWD"
            | "TMPDIR"
            | "TEMP"
            | "TMP"
            | "LANG"
            | "TERM"
            | "XDG_CONFIG_HOME"
    )
}

fn is_mention_name_char(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-')
}

fn normalize_skill_path(path: &str) -> &str {
    path.strip_prefix("skill://").unwrap_or(path)
}

fn normalize_path_for_compare(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_skill_path_like(path: &str) -> bool {
    path.starts_with("skill://") || path.ends_with("SKILL.md") || path.ends_with("skill.md")
}
