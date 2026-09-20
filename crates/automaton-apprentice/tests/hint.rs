use automaton_apprentice::Apprentice;
use automaton_proto::ActionInfo;

fn tmp(name: &str) -> std::path::PathBuf {
    // 테스트별 고유 서브디렉터리 — 병렬 실행 경합 방지(실측: database is locked)
    let dir = std::env::temp_dir().join(format!(
        "automaton-apprentice-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("memory.db")
}

fn info(tool: &str, target: &str) -> ActionInfo {
    ActionInfo {
        tool: tool.into(),
        target: target.into(),
        risk: String::new(),
    }
}

#[test]
fn no_history_yields_no_hint() {
    let a = Apprentice::open(&tmp("empty")).unwrap();
    assert!(
        a.hint_for(&info("fs.delete", "~/Downloads/a.zip"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn three_similar_approvals_yield_hint_with_count() {
    let a = Apprentice::open(&tmp("three")).unwrap();
    for i in 0..3 {
        a.note_decision(
            "s1",
            &info("fs.delete", &format!("~/Downloads/old-{i}.zip")),
            "ask",
            "approve",
        )
        .unwrap();
    }
    let h = a
        .hint_for(&info("fs.delete", "~/Downloads/new.zip"))
        .unwrap();
    let h = h.expect("히스토리가 있으면 힌트 필요");
    assert_eq!(h.similar_count, 3);
    assert!(h.text.contains("승인(3회)"));
}

#[test]
fn denials_do_not_count_as_approvals() {
    let a = Apprentice::open(&tmp("denials")).unwrap();
    a.note_decision("s1", &info("fs.delete", "~/Documents/x.md"), "ask", "deny")
        .unwrap();
    a.note_decision("s2", &info("fs.delete", "~/Documents/y.md"), "ask", "deny")
        .unwrap();
    assert!(
        a.hint_for(&info("fs.delete", "~/Documents/z.md"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn different_tool_does_not_match() {
    let a = Apprentice::open(&tmp("tools")).unwrap();
    a.note_decision(
        "s1",
        &info("fs.write", "~/Downloads/a.txt"),
        "ask",
        "approve",
    )
    .unwrap();
    assert!(
        a.hint_for(&info("fs.delete", "~/Downloads/b.txt"))
            .unwrap()
            .is_none()
    );
}
