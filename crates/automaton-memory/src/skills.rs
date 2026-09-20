//! 스킬 로더 (§7) — agentskills.io 호환 폴더/SKILL.md, 점진적 로딩(인덱스엔 이름·설명만).

#[derive(Debug, Clone)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub path: std::path::PathBuf,
}

#[derive(Debug)]
pub struct SkillIndex {
    pub skills: Vec<SkillMeta>,
}

impl SkillIndex {
    pub fn scan(dir: &std::path::Path) -> std::io::Result<Self> {
        let mut skills = vec![];
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return Ok(SkillIndex { skills }), // 스킬 디렉터리 없음은 정상
        };
        for entry in entries.flatten() {
            let skill_md = entry.path().join("SKILL.md");
            let Ok(raw) = std::fs::read_to_string(&skill_md) else {
                continue;
            };
            let (name, description) = parse_frontmatter(&raw, &entry.file_name().to_string_lossy());
            skills.push(SkillMeta {
                name,
                description,
                path: skill_md,
            });
        }
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(SkillIndex { skills })
    }

    pub fn len(&self) -> usize {
        self.skills.len()
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
    pub fn system_prompt_lines(&self) -> Vec<String> {
        self.skills
            .iter()
            .map(|s| format!("- {}: {}", s.name, s.description))
            .collect()
    }
}

impl std::ops::Deref for SkillIndex {
    type Target = [SkillMeta];
    fn deref(&self) -> &[SkillMeta] {
        &self.skills
    }
}

impl SkillMeta {
    /// 필요할 때 본문 전체 로드 (점진적 로딩 — §7)
    pub fn load_body(&self) -> std::io::Result<String> {
        let raw = std::fs::read_to_string(&self.path)?;
        Ok(strip_frontmatter(&raw))
    }
}

/// 간단 frontmatter 파서: '---' 사이의 'key: value' 라인만 인식 (YAML 의존 없음, M1)
fn parse_frontmatter(raw: &str, fallback_name: &str) -> (String, String) {
    let mut name = fallback_name.to_string();
    let mut description = String::new();
    let mut in_fm = false;
    for line in raw.lines() {
        let t = line.trim();
        if t == "---" {
            if in_fm {
                break;
            } else {
                in_fm = true;
                continue;
            }
        }
        if in_fm {
            if let Some(v) = t.strip_prefix("name:") {
                name = v.trim().to_string();
            }
            if let Some(v) = t.strip_prefix("description:") {
                description = v.trim().to_string();
            }
        }
    }
    (name, description)
}

fn strip_frontmatter(raw: &str) -> String {
    let mut out = String::new();
    let mut in_fm = false;
    let mut seen_first = false;
    for line in raw.lines() {
        let t = line.trim();
        if t == "---" && !seen_first {
            in_fm = true;
            seen_first = true;
            continue;
        }
        if t == "---" && in_fm {
            in_fm = false;
            continue;
        }
        if !in_fm {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
