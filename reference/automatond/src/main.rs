//! automatond — 참조 데몬 (§2). 개인 하네스는 이 구조를 베이스 크레이트 조립으로 대체한다.

use automatond::daemon::{Daemon, Paths}; // lib.rs 경유 — bin 전용 크레이트는 통합 테스트에 노출되지 않음(실측 E0433 반영)

use automaton_core::Provider;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| "serve".into());
    let paths = Paths::default_dirs();
    match cmd.as_str() {
        "doctor" => doctor(&paths),
        "serve" => {
            let socket = PathBuf::from(args.next().unwrap_or_else(|| paths.data_dir.join("automatond.sock").to_string_lossy().into()));
            // 프로바이더: AUTOMATON_API_KEY 있으면 OpenAI 호환, 없으면 안내 후 종료 (§3 키 재사용)
            let provider: Box<dyn Provider> = match automaton_core::OpenAiCompat::from_env() {
                Some(p) => Box::new(p),
                None => { eprintln!("AUTOMATON_API_KEY 미설정 — .env 또는 키체인 설정 후 재시도"); std::process::exit(2); }
            };
            let d = Arc::new(Daemon::new(provider, paths));
            d.serve(socket).await.expect("데몬 서빙 실패");
        }
        other => { eprintln!("모름: {other} · 사용법: automatond [serve [소켓경로]|doctor]"); std::process::exit(2); }
    }
}

/// §9 자가진단 — 권한·경로·키 상태 보고
fn doctor(paths: &Paths) {
    println!("== automaton doctor ==");
    println!("데이터 디렉터리: {} ({})", paths.data_dir.display(), if paths.data_dir.exists() { "존재" } else { "미생성 — 첫 실행 시 생성" });
    println!("정책 파일: {} ({})", paths.policy().display(), if paths.policy().exists() { "존재" } else { "기본 builtin 정책 사용" });
    let ax = std::process::Command::new("osascript").arg("-e").arg("tell application \"System Events\" to name of first process").output();
    println!("접근성 권한: {}", match ax { Ok(o) if o.status.success() => "정상", Ok(_) => "거부됨 — 시스템 설정>개인정보>접근성에서 automatond 허용", Err(_) => "osascript 없음" });
    // 오탐 주의: 스크린 레코딩 권한 거부 상태에서도 screencapture는 exit 0으로 배경화면만 캡처할 수 있음 —
    // exit 0은 "정상"의 필요조건일 뿐이며, 실제 창 캡처 확인은 capture.screen 툴 실행으로 별도 검증해야 한다.
    let cap = std::process::Command::new("screencapture").arg("-x").arg("/tmp/automaton-doctor.png").output();
    println!("스크린 레코딩: {}", match cap { Ok(o) if o.status.success() => "정상(권한 거부 시에도 exit 0일 수 있음 — /tmp/automaton-doctor.png 내용으로 확인)", _ => "거부됨 — 시스템 설정>개인정보>화면 기록에서 허용" });
    println!("AUTOMATON_API_KEY: {}", if std::env::var("AUTOMATON_API_KEY").is_ok() { "설정됨" } else { "미설정" });
}
