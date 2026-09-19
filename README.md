# automaton ⚙️

> **A modular macOS GUI & coding agent framework in Rust & SwiftUI.**

`automaton` is a local-first, on-device AI agent platform built for macOS (Apple Silicon). It bridges coding tasks in your terminal with direct GUI automation across native Mac applications, governed by a deterministic, security-hardened permission policy engine.

---

## Highlights

- **Dual-Track Architecture**: Pure library crates published as an open-source framework, designed to be imported and assembled into custom, private personal harnesses.
- **Tri-Mode Operation**:
  - `code`: Autonomous engineering tools (filesystem, grep, edit, safe shell execution).
  - `mac`: Careful desktop manipulation via ScreenCaptureKit, Accessibility (AX), and CGEvent mouse/keyboard injection.
  - `chat`: Read-only conversational partner.
- **Deterministic Policy Engine**: Strict 6-stage security evaluation guaranteeing that sensitive input fields (passwords, credentials) cannot be accessed, mode escalations require confirmation, and shell execution is strictly gated against command injection and flag bypasses.
- **On-Device Learning (Apprentice Engine Phase 1)**: SQLite + FTS5 decision journaling with sub-linear token-overlap ranking that surfaces intelligent past-decision hints in approval dialogs without telemetry.
- **Steampunk Aesthetic (Brass & Glass)**: Native SwiftUI menu bar popover styled in dark walnut, brushed brass, and gold typography.

---

## Architecture

```
SwiftUI Shell (apps/Automaton)
  Conversation stream · Mode switcher · Approval banner with hints · ndjson UDS client
        ↕ JSON-RPC over Unix domain socket (~/.local/share/automaton/automatond.sock)
automaton core (reference/automatond or your custom binary)
  ├─ automaton-proto      JSON-RPC requests, events, and contract types
  ├─ automaton-policy     Deterministic ALLOW / ASK / DENY policy engine
  ├─ automaton-tools      Tool trait, registry, coding tools, shell classifier, mac tools
  ├─ automaton-memory     SQLite store (WAL, FTS5 facts & decisions) + agentskills.io loader
  ├─ automaton-apprentice Decision journal & similarity hint provider
  └─ automaton-core       Provider abstraction (OpenAI-compatible / scripted), agent loop, mode profiles
        ↕ macOS APIs
Accessibility API (AXUIElement) · ScreenCaptureKit · CoreGraphics (CGEvent)
```

---

## Crates

| Crate | Description |
|---|---|
| [`automaton-proto`](crates/automaton-proto) | Wire contract types for JSON-RPC requests, streaming events, and approval dialogs. |
| [`automaton-policy`](crates/automaton-policy) | Deterministic rule engine enforcing fail-closed permission evaluation. |
| [`automaton-tools`](crates/automaton-tools) | Tool trait, registry, filesystem/edit tools, shell metacharacter defense, and macOS CGEvent/AX tools. |
| [`automaton-memory`](crates/automaton-memory) | SQLite persistence with WAL mode, FTS5 virtual tables, and progressive markdown skill indexing. |
| [`automaton-apprentice`](crates/automaton-apprentice) | Local decision learning loop and similarity hint provider. |
| [`automaton-core`](crates/automaton-core) | Multi-turn agent loop, provider abstraction, and mode profiles. |
| [`automatond`](reference/automatond) | Reference daemon providing the UDS ndjson server, doctor diagnostics, and session management. |
| [`Automaton`](apps/Automaton) | Native macOS SwiftUI menu bar application featuring the Brass & Glass theme. |

---

## Quick Start

### Prerequisites

- macOS 14.0+ (Apple Silicon recommended)
- Rust 1.90+ (`cargo`)
- Swift 6.0+ (`swift`) and Xcode Command Line Tools

### Build & Run Tests

```bash
# Clone the repository
git clone https://github.com/TheMagicTower/automaton.git
cd automaton

# Run the complete Rust test suite (58+ tests)
cargo test --workspace

# Build the SwiftUI Menu Bar app
cd apps/Automaton && swift build -Xswiftc -warnings-as-errors
```

### Self-Diagnostics (`doctor`)

Verify your macOS permissions (Accessibility, Screen Recording) and environment setup:

```bash
cargo run -p automatond -- doctor
```

### Start the Daemon

```bash
export AUTOMATON_API_KEY="your-api-key"
cargo run -p automatond -- serve
```

Then in another terminal:

```bash
cd apps/Automaton && swift run
```

---

## Security Invariants

1. **Hard-Denied Sensitive Inputs**: Fields matching passwords, passphrases, tokens, credentials, PINs, or OTPs are unconditionally denied. No user grant or model instruction can override this rule.
2. **Command Injection Defense**: Shell execution enforces a 12-metacharacter blacklist (`;`, `&&`, `|`, `<`, `>`, etc.) and output flag interception (`--output`, `-o`), guaranteeing complex or file-writing commands always require human approval.
3. **Decoy Argument Neutralization**: Tool parameter extraction strictly reads declared operands (e.g. `command` for `shell.exec`, `path` for filesystem tools), making it impossible for untrusted model outputs to spoof targets or bypass inspection with extra keys.
4. **Local Socket Protection**: The daemon's Unix domain socket is bound with `0o600` permissions immediately upon creation, restricting access strictly to the local owner.

---

## License

Dual-licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
