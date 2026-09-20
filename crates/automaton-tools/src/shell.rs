//! shell.exec — 스펙 §5가 구현 계획에 위임한 '허용 명령 분류' 정의.
//! 분류 가이드(결정론적):
//!   READ   — 읽기 전용: ls, cat, head, tail, pwd, which, file, wc, git status/diff/log/show(옵션 화이트리스트)
//!   EXTERNAL — 그 외 전부(빌드·테스트 러너 포함, 네트워크·시스템 변경 가능): 항상 ASK. granted 규칙으로만 해제.
//! code 모드 자율성 = READ가 Allow(§5), 위험 셸 = EXTERNAL이 ASK.

use crate::{Tool, ToolError};
use automaton_policy::Category;
use serde_json::Value;

const READ_PREFIXES: &[&str] = &[
    "ls", "cat", "head", "tail", "pwd", "which", "file", "wc",
    // 진단·조사 명령 — 부수효과 없는 출력 전용
    "du", "df", "ps", "uname", "date", "uptime", "sw_vers", "sysctl", "hostname", "id",
    "whoami", "groups", "stat", "realpath", "basename", "dirname", "md5sum", "shasum", "shasum5",
    "grep", "find", "diff", "sort", "uniq", "wc", "column", "jq", "plutil", "defaults read",
    "system_profiler", "ioreg", "lsof", "netstat", "ifconfig", "ping", "dig", "nslookup", "host",
    "cargo --version", "cargo --list", "rustc --version", "rustup show",
    "node --version", "npm --version", "python3 --version", "swift --version",
];
// F-01: 빌드·테스트 러너(make, pytest, npm test, cargo build/test/check, swift build/test)는
// 정의상 임의 코드 실행기다(make 레시피 = /bin/sh -c, conftest.py, package.json scripts, build.rs, SPM 플러그인).
// fs.write(Write=자동 Allow)와 체이닝하면 무승인 RCE가 되므로 Write 자동허용 목록에서 제거해
// 전부 External(항상 ASK)로 강등한다. 워크스페이스 디렉터리 제한으로는 -f 절대경로·conftest 자동 탐색
// 등을 접두사 분류 수준에서 전수 차단할 수 없어 ASK 강등이 유일하게 완전한 즉시 조치다.

/// git 읽기 서브커맨드 허용 목록 (F-02). 그 외 서브커맨드는 전부 External.
const GIT_READ_SUBCOMMANDS: &[&str] = &["status", "diff", "log", "show"];
/// git 읽기 서브커맨드에 허용되는 표시 전용 옵션 화이트리스트 (F-02, 포지티브 필터).
/// `-`로 시작하는 미등록 토큰은 `--output`의 약어(--outpu=)를 포함해 전부 즉시 External —
/// 블록리스트와 달리 알려지지 않은 옵션·약어를 구조적으로 기각한다.
/// 값 결합은 "옵션명=" 접두사 형태만 허용(--stat=200 O / --outpu=x X).
const GIT_ALLOWED_OPTS: &[&str] = &[
    "--stat", "--name-only", "--name-status", "--shortstat", "--numstat", "--oneline",
    "--short", "--branch", "--porcelain", "--long", "--graph", "--decorate", "--no-decorate",
    "--color", "--no-color", "--abbrev-commit", "--summary", "--patch", "--no-patch",
    "--cached", "--staged", "--merges", "--no-merges", "--all", "--first-parent", "--reverse",
    "--follow", "--find-renames", "--find-copies", "--pretty", "--format", "--date",
    "-p", "-s", "-w", "-a", "-n", "-M", "-C",
];

fn first_word_classify(cmd: &str) -> Category {
    const METACHARS: &[&str] = &[";", "&&", "||", "|", ">", ">>", "<", "&", "`", "$(", "\n", "\r"];
    let c = cmd.trim_start();
    // 복합 명령 우회 방지: 메타문자 포함 시 무조건 External (항상 ASK)
    if METACHARS.iter().any(|m| c.contains(m)) { return Category::External; }
    // git은 서브커맨드·옵션 화이트리스트로 별도 분류 (F-02)
    if c == "git" || c.starts_with("git ") {
        classify_git(c)
    } else if READ_PREFIXES.iter().any(|p| c == *p || c.starts_with(&format!("{p} "))) {
        Category::Read
    } else {
        Category::External
    }
}

/// git 명령 분류 (F-02):
/// - 허용 서브커맨드는 status/diff/log/show뿐 — config/checkout 등 전부 External
/// - 옵션은 포지티브 화이트리스트: 미등록 `-` 토큰(약어 `--outpu=` 포함) 즉시 External
/// - `$`/`${` 환경변수 전개 문자 포함 토큰은 셸 실행 시점 전개로 경로가 예측 불가하므로 External
/// - 서브커맨드 앞 글로벌 옵션(-C, -c 등)도 미등록 토큰으로 취급해 External
fn classify_git(c: &str) -> Category {
    let mut toks = c.split_whitespace();
    toks.next(); // "git"
    let Some(sub) = toks.next() else { return Category::External };
    if !GIT_READ_SUBCOMMANDS.contains(&sub) { return Category::External; }
    for t in toks {
        if t.contains('$') { return Category::External; }
        if t == "--" { continue; } // 옵션 종결자 — 이후 토큰은 전부 경로·피연산자로 취급
        let Some(rest) = t.strip_prefix('-') else { continue }; // 피연산자(리비전·경로) 통과
        if rest.is_empty() { continue; } // "-" 단독 토큰은 옵션이 아님
        let allowed = GIT_ALLOWED_OPTS.contains(&t)
            || GIT_ALLOWED_OPTS.iter().any(|o| t.starts_with(&format!("{o}=")));
        if !allowed { return Category::External; }
    }
    Category::Read
}

pub struct ShellExec;

impl Tool for ShellExec {
    fn name(&self) -> &'static str { "shell.exec" }
    fn description(&self) -> &'static str { "셸 명령을 실행하고 출력을 반환" }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]})
    }
    fn category(&self, args: &Value) -> Category {
        args.get("command").and_then(|v| v.as_str()).map(first_word_classify).unwrap_or(Category::External)
    }
    fn execute(&self, args: &Value) -> Result<String, ToolError> {
        let command = args.get("command").and_then(|v| v.as_str()).ok_or_else(|| ToolError::Message("command 인자 누락".into()))?;
        let out = std::process::Command::new("/bin/sh").arg("-c").arg(command).output()
            .map_err(|e| ToolError::Message(format!("실행 실패: {e}")))?;
        let text = String::from_utf8_lossy(&out.stdout);
        let err = String::from_utf8_lossy(&out.stderr);
        Ok(format!("exit={} stdout:\n{} stderr:\n{}", out.status, text, err))
    }
}
