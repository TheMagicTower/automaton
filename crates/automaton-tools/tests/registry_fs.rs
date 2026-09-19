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
    let (app, target) = automaton_tools::action_context("shell.exec", &json!({"command": "echo test"}));
    assert_eq!(app, None);
    assert_eq!(target, Some("echo test".to_string()));
}

#[test]
fn action_context_prioritizes_operational_args_over_decoy_target() {
    // 보안 회귀 방지: 적대적 모델이 decoy target 또는 decoy path를 보내도 도구 선언 피연산자만 추출되어야 함
    let (_, target_cmd) = automaton_tools::action_context("shell.exec", &json!({"command": "rm -rf ~", "target": "cargo test", "path": "notes.txt"}));
    assert_eq!(target_cmd, Some("rm -rf ~".to_string()));

    let (_, target_path) = automaton_tools::action_context("fs.read", &json!({"path": "Passwords.kdbx", "target": "notes.txt", "command": "echo fake"}));
    assert_eq!(target_path, Some("Passwords.kdbx".to_string()));

    let (_, target_input) = automaton_tools::action_context("input.type", &json!({"target": "SecureTextField", "path": "notes.txt"}));
    assert_eq!(target_input, Some("SecureTextField".to_string()));

    // 타입 혼동 방지: 비문자열 path가 있어도 정상 추출
    let (_, target_type) = automaton_tools::action_context("shell.exec", &json!({"command": "cat secret", "path": 123}));
    assert_eq!(target_type, Some("cat secret".to_string()));
}

#[test]
fn fs_read_lines_prefixes_line_numbers() {
    let p = tmp("lines.txt");
    std::fs::write(&p, "alpha\nbrass line\ngamma\n").unwrap();
    let out = FsReadLines.execute(&json!({"path": p})).unwrap();
    assert!(out.contains("1:alpha"), "실제 출력: {out}");
    assert!(out.contains("2:brass line"), "실제 출력: {out}");
    assert!(out.contains("3:gamma"), "실제 출력: {out}");
}

#[test]
fn fs_read_lines_declares_read_and_missing_file_is_error() {
    assert_eq!(FsReadLines.category(&json!({})), automaton_policy::Category::Read);
    assert!(FsReadLines.execute(&json!({"path": tmp("없는파일-줄읽기")})).is_err());
}

#[test]
fn edit_replace_lines_replaces_inclusive_range() {
    let p = tmp("replace.txt");
    std::fs::write(&p, "one\ntwo\nthree\nfour\n").unwrap();
    EditReplaceLines.execute(&json!({"path": p, "start": 2, "end": 3, "content": "B\nc"})).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\nB\nc\nfour\n");
}

#[test]
fn edit_replace_lines_empty_content_deletes_range() {
    let p = tmp("del-range.txt");
    std::fs::write(&p, "a\nb\nc\n").unwrap();
    EditReplaceLines.execute(&json!({"path": p, "start": 2, "end": 2, "content": ""})).unwrap();
    assert_eq!(std::fs::read_to_string(&p).unwrap(), "a\nc\n");
}

#[test]
fn edit_replace_lines_rejects_invalid_range_and_declares_write() {
    let p = tmp("bad-range.txt");
    std::fs::write(&p, "a\nb\n").unwrap();
    assert_eq!(EditReplaceLines.category(&json!({"path": p})), automaton_policy::Category::Write);
    assert!(EditReplaceLines.execute(&json!({"path": p, "start": 0, "end": 1, "content": "x"})).is_err()); // 1-based 위반
    assert!(EditReplaceLines.execute(&json!({"path": p, "start": 2, "end": 1, "content": "x"})).is_err()); // start>end
    assert!(EditReplaceLines.execute(&json!({"path": p, "start": 1, "end": 99, "content": "x"})).is_err()); // 범위 초과
}

#[test]
fn fs_mkdir_creates_nested_dirs_and_declares_write() {
    let p = tmp("nested/a/b");
    assert_eq!(FsMkdir.category(&json!({"path": p})), automaton_policy::Category::Write);
    FsMkdir.execute(&json!({"path": p})).unwrap();
    assert!(p.is_dir());
    FsMkdir.execute(&json!({"path": p})).unwrap(); // 이미 존재해도 멱등
}

#[test]
fn fs_move_relocates_file_with_content() {
    let src = tmp("mv-src.txt");
    let dst = tmp("mv-dst.txt");
    std::fs::write(&src, "payload").unwrap();
    assert_eq!(FsMove.category(&json!({"path": src, "dest": dst})), automaton_policy::Category::Write);
    FsMove.execute(&json!({"path": src, "dest": dst})).unwrap();
    assert!(!src.exists(), "원본이 남아 있으면 안 됨");
    assert_eq!(std::fs::read_to_string(&dst).unwrap(), "payload");
    assert!(FsMove.execute(&json!({"path": tmp("없는원본"), "dest": dst})).is_err());
}

#[test]
fn coding_set_registers_line_editing_tools() {
    let r = Registry::coding_set();
    for n in ["fs.read_lines", "edit.replace_lines", "fs.mkdir", "fs.move"] {
        assert!(r.get(n).is_some(), "{n} 등록 필요");
    }
}
