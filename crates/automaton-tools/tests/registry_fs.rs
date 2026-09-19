use automaton_tools::*;
use serde_json::json;

fn tmp(name: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("automaton-tools-{}/", std::process::id())).join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    p
}

#[test]
fn registry_registers_and_look_up() {
    let mut r = Registry::new();
    r.register(Box::new(FsRead));
    assert_eq!(r.get("fs.read").unwrap().description(), "파일을 읽어 반환");
    assert!(r.get("fs.write").is_none());
    assert_eq!(r.names(), vec!["fs.read"]);
}

#[test]
fn fs_write_then_read_roundtrip() {
    let p = tmp("round.txt");
    FsWrite.execute(&json!({"path": p, "content": "홍브라스 시대"})).unwrap();
    let out = FsRead.execute(&json!({"path": p})).unwrap();
    assert!(out.contains("홍브라스"));
}

#[test]
fn fs_grep_reports_line_numbers() {
    let p = tmp("grep.txt");
    std::fs::write(&p, "alpha\nbeta brass\ngamma\n").unwrap();
    let out = FsGrep.execute(&json!({"path": p, "pattern": "brass"})).unwrap();
    assert!(out.contains("2:"), "실제 출력: {out}");
}

#[test]
fn fs_delete_removes_file_and_declares_destructive() {
    let p = tmp("del.txt");
    std::fs::write(&p, "x").unwrap();
    assert_eq!(FsDelete.category(&json!({"path": p})), automaton_policy::Category::Destructive);
    FsDelete.execute(&json!({"path": p})).unwrap();
    assert!(!p.exists());
}

#[test]
fn fs_read_declares_read_category_and_missing_file_is_error() {
    assert_eq!(FsRead.category(&json!({})), automaton_policy::Category::Read);
    assert!(FsRead.execute(&json!({"path": tmp("없는파일")})).is_err());
}

#[test]
fn action_context_extracts_command_as_target() {
    let (app, target) = automaton_tools::action_context(&json!({"command": "echo test"}));
    assert_eq!(app, None);
    assert_eq!(target, Some("echo test".to_string()));
}

#[test]
fn action_context_prioritizes_operational_args_over_decoy_target() {
    // 보안 회귀 방지: 적대적 모델이 decoy target을 보내도 실제 command 또는 path가 타깃으로 추출되어야 함
    let (_, target_cmd) = automaton_tools::action_context(&json!({"command": "rm -rf ~", "target": "cargo test"}));
    assert_eq!(target_cmd, Some("rm -rf ~".to_string()));

    let (_, target_path) = automaton_tools::action_context(&json!({"path": "Passwords.kdbx", "target": "notes.txt"}));
    assert_eq!(target_path, Some("Passwords.kdbx".to_string()));
}
