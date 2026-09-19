//! 모드 프로파일 (§5) — 툴셋+프롬프트 조합. 개인 조립은 이 정의를 데이터/코드로 재정의 (§2).

use automaton_proto::Mode;

pub struct ModeProfile { pub mode: Mode, pub tools: Vec<&'static str>, pub system_prompt: String }

impl ModeProfile {
    pub fn builtin(mode: Mode) -> Self {
        match mode {
            Mode::Code => ModeProfile {
                mode, tools: vec!["fs.read", "fs.read_lines", "fs.write", "fs.grep", "fs.delete", "fs.mkdir", "fs.move", "shell.exec", "edit.apply", "edit.replace_lines"],
                system_prompt: "당신은 automaton의 code 모드입니다. 정확한 엔지니어로서 파일을 읽고·쓰고·빌드해 작업을 완수하세요. 모든 도구 결과를 근거로 보고하세요.".into(),
            },
            Mode::Mac => ModeProfile {
                mode, tools: vec!["capture.screen", "ax.read", "ax.list_elements", "input.click", "input.click_element", "input.type", "shell.exec"],
                system_prompt: "당신은 automaton의 mac 모드입니다. 신중한 조작수로서 화면을 관찰하고 필요한 최소 동작만 수행하세요. 위험 동작은 승인 절차를 따릅니다.".into(),
            },
            Mode::Chat => ModeProfile {
                mode, tools: vec!["fs.read", "fs.grep"],
                system_prompt: "당신은 automaton의 chat 모드입니다. 대화 파트너로서 읽기 도구만으로 답변을 구성하세요.".into(),
            },
        }
    }
}
