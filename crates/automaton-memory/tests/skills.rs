use automaton_memory::SkillIndex;

fn skill_dir(name: &str) -> std::path::PathBuf {
    // 테스트별 고유 경로 — 병렬 실행 경합 방지
    let dir = std::env::temp_dir().join(format!("automaton-skills-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let s1 = dir.join("organize-downloads");
    std::fs::create_dir_all(&s1).unwrap();
    std::fs::write(s1.join("SKILL.md"), "---\nname: organize-downloads\ndescription: 다운로드 폴더를 분류 정리한다\n---\n# 정리 절차\n1. 확장자별 분류\n2. 30일 경과 파일 삭제 제안\n").unwrap();
    dir
}

#[test]
fn scan_extracts_frontmatter_only() {
    let idx = SkillIndex::scan(&skill_dir("scan")).unwrap();
    assert_eq!(idx.len(), 1);
    assert_eq!(idx[0].name, "organize-downloads");
    assert!(idx[0].description.contains("다운로드"));
}

#[test]
fn body_loaded_on_demand_not_in_index() {
    let idx = SkillIndex::scan(&skill_dir("body")).unwrap();
    assert!(!format!("{idx:?}").contains("정리 절차")); // 인덱스에는 본문 없음
    let body = idx[0].load_body().unwrap();
    assert!(body.contains("정리 절차"));
}

#[test]
fn system_prompt_lines_are_compact() {
    let idx = SkillIndex::scan(&skill_dir("prompt")).unwrap();
    let lines = idx.system_prompt_lines();
    assert!(lines[0].contains("organize-downloads"));
    assert!(lines[0].contains("다운로드 폴더를 분류"));
}
