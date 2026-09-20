use automaton_apprentice::{Apprentice, DraftComposer};
use automaton_memory::Decision;
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

fn approval(tool: &str) -> Decision {
    Decision {
        session: "s1".into(),
        tool: tool.into(),
        target: "~/Downloads/old.zip".into(),
        verdict: "ask".into(),
        decision: "approve".into(),
    }
}

#[test]
fn three_approvals_yield_go_ahead_draft() {
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
    let drafts = a
        .drafts_for(&info("fs.delete", "~/Downloads/new.zip"))
        .unwrap();
    assert!(
        drafts.iter().any(|d| d == "진행해"),
        "초안에 '진행해' 필요: {drafts:?}"
    );
}

#[test]
fn no_history_yields_no_drafts() {
    let a = Apprentice::open(&tmp("empty")).unwrap();
    assert!(
        a.drafts_for(&info("fs.delete", "~/Downloads/a.zip"))
            .unwrap()
            .is_empty()
    );
    assert!(
        DraftComposer
            .generate_drafts("fs.delete", "~/Downloads/a.zip", &[])
            .is_empty()
    );
}

#[test]
fn other_tool_history_is_not_used() {
    let past = vec![
        approval("fs.write"),
        approval("fs.write"),
        approval("fs.write"),
    ];
    assert!(
        DraftComposer
            .generate_drafts("fs.delete", "~/Downloads/new.zip", &past)
            .is_empty()
    );
    assert!(
        !DraftComposer
            .generate_drafts("fs.write", "~/notes.md", &past)
            .is_empty()
    );
}

#[test]
fn two_approvals_stay_below_threshold() {
    let past = vec![approval("fs.delete"), approval("fs.delete")];
    assert!(
        DraftComposer
            .generate_drafts("fs.delete", "~/Downloads/new.zip", &past)
            .is_empty()
    );
}
