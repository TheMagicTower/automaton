//! 모드 프로파일 (§5) — 모든 모드가 동일한 전체 툴셋을 가진다.
//! 모드는 성격(프롬프트)만 차별화하고, 안전은 Policy Engine이 모드별로 차등 제어한다:
//!   Read → 모든 모드 자동 허용
//!   Write → code 모드 자동 허용, 타 모드 승인 요청
//!   Input → 모든 모드 승인 요청 (target 필수)
//!   External → 모든 모드 승인 요청
//! 이 설계는 "chat 모드에서 cargo test하려면 code로 전환"하는 마찰을 제거한다.

use automaton_proto::Mode;

pub struct ModeProfile { pub mode: Mode, pub tools: Vec<&'static str>, pub system_prompt: String }

/// 전 모드 공통 툴셋 — 코딩 + mac + 진단 도구 전부
const ALL_TOOLS: &[&str] = &[
    // 파일·코드 도구
    "fs.read", "fs.read_lines", "fs.write", "fs.grep", "fs.delete", "fs.mkdir", "fs.move",
    "shell.exec", "edit.apply", "edit.replace_lines",
    // mac 화면·조작 도구
    "capture.screen", "ax.read", "ax.list_elements",
    "input.click", "input.click_element", "input.type",
];

impl ModeProfile {
    pub fn builtin(mode: Mode) -> Self {
        let prompt = match mode {
            Mode::Code =>
                "당신은 automaton의 code 모드입니다. 정확한 엔지니어로서 파일을 읽고·쓰고·빌드해 작업을 완수하세요. 모든 도구 결과를 근거로 보고하세요.",
            Mode::Mac =>
                "당신은 automaton의 mac 모드입니다. 신중한 조작수로서 화면을 관찰하고 필요한 최소 동작만 수행하세요. 위험 동작은 승인 절차를 따릅니다.",
            Mode::Chat =>
                "당신은 automaton의 chat 모드입니다. 대화 파트너로서 질문에 답하고 필요하면 도구를 사용해 정보를 수집하세요. 파일 읽기·검색·진단 명령은 자유롭게 사용할 수 있습니다.",
        };
        ModeProfile { mode, tools: ALL_TOOLS.to_vec(), system_prompt: prompt.into() }
    }
}
