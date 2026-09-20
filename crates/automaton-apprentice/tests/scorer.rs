use automaton_apprentice::Apprentice;
use automaton_proto::ActionInfo;

fn tmp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "automaton-apprentice-scorer-{}-{name}",
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
fn four_approvals_one_deny_is_exactly_threshold_not_default() {
    let a = Apprentice::open(&tmp("4a1d")).unwrap();
    for i in 0..4 {
        a.note_decision(
            "s1",
            &info("fs.delete", &format!("~/old-{i}.zip")),
            "ask",
            "approve",
        )
        .unwrap();
    }
    a.note_decision("s2", &info("fs.delete", "~/old-9.zip"), "ask", "deny")
        .unwrap();
    let s = a.preference_score("fs.delete").unwrap();
    assert_eq!(s.sample_count, 5);
    assert!((s.probability - 0.8).abs() < 1e-9); // 0.8은 >0.8이 아니므로 기본값 아님
    assert!(!s.suggested_default);
}

#[test]
fn five_approvals_suggest_default() {
    let a = Apprentice::open(&tmp("5a")).unwrap();
    for i in 0..5 {
        a.note_decision(
            "s1",
            &info("fs.write", &format!("~/note-{i}.md")),
            "ask",
            "approve",
        )
        .unwrap();
    }
    let s = a.preference_score("fs.write").unwrap();
    assert_eq!(s.sample_count, 5);
    assert_eq!(s.probability, 1.0);
    assert_eq!(s.confidence, 1.0);
    assert!(s.suggested_default);
}

#[test]
fn no_history_is_neutral_low_confidence() {
    let a = Apprentice::open(&tmp("none")).unwrap();
    let s = a.preference_score("net.fetch").unwrap();
    assert_eq!(s.sample_count, 0);
    assert_eq!(s.probability, 0.5);
    assert_eq!(s.confidence, 0.0);
    assert!(!s.suggested_default);
}

#[test]
fn other_tool_history_does_not_leak() {
    let a = Apprentice::open(&tmp("leak")).unwrap();
    for i in 0..3 {
        a.note_decision(
            "s1",
            &info("fs.write", &format!("~/a-{i}.txt")),
            "ask",
            "approve",
        )
        .unwrap();
    }
    let s = a.preference_score("fs.delete").unwrap();
    assert_eq!(s.sample_count, 0);
    assert_eq!(s.probability, 0.5);
    assert!(!s.suggested_default);
}
