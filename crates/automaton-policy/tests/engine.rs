use automaton_policy::*;

fn action(tool: &str, category: Category) -> Action {
    Action { tool: tool.into(), category, app: None, target: None }
}

#[test]
fn builtin_allows_reads_in_every_mode() {
    for mode in [Mode::Code, Mode::Mac, Mode::Chat] {
        let e = Engine::builtin();
        assert!(matches!(e.evaluate(&action("fs.read", Category::Read), mode), Verdict::Allow));
        assert!(matches!(e.evaluate(&action("capture.screen", Category::Read), mode), Verdict::Allow));
    }
}

#[test]
fn builtin_allows_code_mode_edits_but_asks_in_other_modes() {
    let e = Engine::builtin();
    assert!(matches!(e.evaluate(&action("fs.write", Category::Write), Mode::Code), Verdict::Allow));
    assert!(matches!(e.evaluate(&action("fs.write", Category::Write), Mode::Mac), Verdict::Ask { .. }));
}

#[test]
fn destructive_and_external_actions_always_ask() {
    for mode in [Mode::Code, Mode::Mac, Mode::Chat] {
        let e = Engine::builtin();
        assert!(matches!(e.evaluate(&action("fs.delete", Category::Destructive), mode), Verdict::Ask { .. }));
        assert!(matches!(e.evaluate(&action("shell.exec", Category::External), mode), Verdict::Ask { .. }));
    }
}

#[test]
fn input_defaults_to_ask_even_in_mac_mode_until_granted() {
    // 스펙 §5: 미등록 앱 클릭·타이핑 → ASK. 등록(=grant_always) 이후에만 Allow.
    // F-04 반영: Allow 판정은 target을 자기선언한 입력에만 성립 — target 미포함 회귀는 별도 테스트.
    let mut e = Engine::builtin();
    let a = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.apple.finder".into()), target: Some("search field".into()) };
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Ask { .. }));
    e.grant_always(Rule { name: "finder-input".into(), tool: Some("input.type".into()), app: Some("com.apple.finder".into()), category: None, verdict: VerdictTemplate::Allow });
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Allow));
}

#[test]
fn input_without_target_never_allows_even_when_granted_f04() {
    // F-04 회귀: target 미포함 input.*은 granted 규칙보다 우선하는 항상 ASK —
    // 소유자가 앱 입력을 '항상 허용'으로 등록해도 자기선언 누락 호출은 자동 Allow 불가.
    let mut e = Engine::builtin();
    e.grant_always(Rule { name: "finder-input".into(), tool: Some("input.type".into()), app: Some("com.apple.finder".into()), category: None, verdict: VerdictTemplate::Allow });
    for category in [Category::Input, Category::External] { // 툴 분류와 무관하게 정책이 최종 방어
        let a = Action { tool: "input.type".into(), category, app: Some("com.apple.finder".into()), target: None };
        for mode in [Mode::Code, Mode::Mac, Mode::Chat] {
            assert!(matches!(e.evaluate(&a, mode), Verdict::Ask { .. }), "target 미포함 input은 granted로도 Allow 불가 ({category:?}/{mode:?})");
        }
    }
}

#[test]
fn sensitive_targets_are_denied_regardless_of_rules() {
    let e = Engine::builtin();
    let a = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.some.bank".into()), target: Some("SecureTextField".into()) };
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Deny { .. }));
}

#[test]
fn sensitive_targets_denied_case_insensitive() {
    let e = Engine::builtin();
    for target in ["password", "PASSWORD", "user_password_field", "passwd", "my_passphrase", "my_passcode", "secret_token", "user_credential", "api_key", "my-otp", "card_cvv", "user_pin"] {
        let a = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.apple.finder".into()), target: Some(target.into()) };
        assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Deny { .. }), "should deny target: {target}");
    }
    // pin/otp 등이 단어의 일부로 포함된 일반 단어(typing, shipping, spinner)는 Deny되지 않아야 함 (위양성 방지)
    for benign in ["typing_area", "shipping_address", "spinner_control"] {
        let a = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.apple.finder".into()), target: Some(benign.into()) };
        assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Ask { .. }), "should not deny benign: {benign}");
    }
}

#[test]
fn wildcard_app_grant_does_not_override_deny_app_rules() {
    let mut e = Engine::builtin();
    // input.type 툴 전체에 대해 app: None으로 일반 허용 등록
    e.grant_always(Rule { name: "grant-all-input".into(), tool: Some("input.type".into()), app: None, category: None, verdict: VerdictTemplate::Allow });
    
    // 일반 앱은 허용됨 (F-04 이후 Allow는 target 자기선언이 있는 입력에만 성립)
    let normal = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.example.app".into()), target: Some("search field".into()) };
    assert!(matches!(e.evaluate(&normal, Mode::Mac), Verdict::Allow));

    // 하지만 금지 앱(com.some.bank)은 여전히 DENY되어야 함 (일반 와일드카드 그랜트로 뚫리지 않음)
    let bank = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.some.bank".into()), target: Some("search field".into()) };
    assert!(matches!(e.evaluate(&bank, Mode::Mac), Verdict::Deny { .. }));
}

#[test]
fn deny_takes_precedence_over_allow_regardless_of_order() {
    let e = Engine::with_rules(vec![
        Rule { name: "allow-all-input".into(), tool: Some("input.type".into()), app: None, category: None, verdict: VerdictTemplate::Allow },
        Rule { name: "deny-example".into(), tool: None, app: Some("com.example.app".into()), category: None, verdict: VerdictTemplate::Deny },
    ]);
    let a = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.example.app".into()), target: Some("search field".into()) };
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Deny { .. }));
}

#[test]
fn mode_switch_always_asks_even_when_granted() {
    let e = Engine::builtin();
    assert!(matches!(e.evaluate(&action("mode.switch", Category::ModeSwitch), Mode::Chat), Verdict::Ask { .. }));
    // §5 원천 차단 불변식: 소유자 granted Allow 규칙으로도 모드 전환은 우회 불가
    let mut g = Engine::builtin();
    g.grant_always(Rule { name: "always-switch".into(), tool: Some("mode.switch".into()), app: None, category: None, verdict: VerdictTemplate::Allow });
    assert!(matches!(g.evaluate(&action("mode.switch", Category::ModeSwitch), Mode::Chat), Verdict::Ask { .. }));
}

#[test]
fn owner_grant_overrides_builtin_deny_app_but_never_password_fields() {
    // 스펙 §5: 민감 영역은 소유자가 명시적으로 등록(화이트리스트)한 경우에만 허용.
    let mut e = Engine::builtin();
    let bank = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.some.bank".into()), target: Some("search field".into()) };
    assert!(matches!(e.evaluate(&bank, Mode::Mac), Verdict::Deny { .. })); // builtin deny-app 규칙
    e.grant_always(Rule { name: "owner-trusts-bank".into(), tool: Some("input.type".into()), app: Some("com.some.bank".into()), category: None, verdict: VerdictTemplate::Allow });
    assert!(matches!(e.evaluate(&bank, Mode::Mac), Verdict::Allow)); // 명시 등록 → 허용 (target 선언된 입력)
    let pw = Action { tool: "input.type".into(), category: Category::Input, app: Some("com.some.bank".into()), target: Some("SecureTextField".into()) };
    assert!(matches!(e.evaluate(&pw, Mode::Mac), Verdict::Deny { .. })); // 비밀번호 필드는 절대 불가
}

#[test]
fn grant_always_persists_and_allows_future_matching() {
    let mut e = Engine::builtin();
    let a = action("fs.delete", Category::Destructive);
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Ask { .. }));
    e.grant_always(Rule { name: "trash-downloads".into(), tool: Some("fs.delete".into()), app: None, category: Some(Category::Destructive), verdict: VerdictTemplate::Allow });
    assert!(matches!(e.evaluate(&a, Mode::Mac), Verdict::Allow));
}

#[test]
fn policy_file_roundtrip_persists_granted_rules() {
    let mut e = Engine::builtin();
    e.grant_always(Rule { name: "trash-downloads".into(), tool: Some("fs.delete".into()), app: None, category: Some(Category::Destructive), verdict: VerdictTemplate::Allow });
    let dir = std::env::temp_dir().join(format!("automaton-policy-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("policy.toml");
    e.save(&path).unwrap();
    let loaded = Engine::from_file(&path).unwrap();
    assert!(matches!(loaded.evaluate(&action("fs.delete", Category::Destructive), Mode::Mac), Verdict::Allow));
}
