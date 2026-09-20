# automaton ⚙

**A steampunk-flavored personal GUI agent for macOS.** Built in Rust with a SwiftUI Brass & Glass shell.

> Your assistant, on your machine. Your data stays local.

## What it does

automaton is a personal AI agent that lives in your macOS menu bar. It can:

- 💬 **Chat** — answer questions, analyze files, run diagnostics
- ⚙️ **Code** — read, write, edit files; run builds; manage projects
- 🔭 **Mac** — capture screens, read UI elements, click and type

All three modes share the same toolset. Safety is enforced by a deterministic policy engine, not by hiding tools.

## Quick Start

### Prerequisites

- macOS 14+ (Apple Silicon)
- Rust 1.75+ (`rustup`)
- Xcode Command Line Tools
- An OpenAI-compatible API key (e.g., [Z.AI](https://z.ai))

### Build & Run

```bash
git clone https://github.com/TheMagicTower/automaton.git
cd automaton

# Start the daemon
export AUTOMATON_API_KEY="your-key"
export AUTOMATON_BASE_URL="https://api.z.ai/api/coding/paas/v4"
export AUTOMATON_MODEL="glm-4.6"
cargo run -p automatond -- serve

# In another terminal, start the menu bar app
cd apps/Automaton
swift run
```

Or build a proper .app bundle:

```bash
./scripts/build-app.sh
open build/Automaton.app
```

### First Run

1. A ⚙ gear icon appears in your menu bar
2. Click it to open the chat popover
3. Grant **Accessibility** and **Screen Recording** permissions when prompted (for mac mode)
4. Start chatting — automaton learns about you and remembers across sessions

## Architecture

```
┌─────────────────────────────────────────────┐
│  SwiftUI Shell (menu bar + window)          │
│  Brass & Glass · Chat Bubbles · Sidebar     │
└──────────────────┬──────────────────────────┘
                   │ JSON-RPC over Unix Socket
┌──────────────────┴──────────────────────────┐
│  automatond (Rust daemon)                   │
│  ┌─────────┐ ┌──────────┐ ┌─────────────┐ │
│  │ Agent   │ │ Policy   │ │ Apprentice  │ │
│  │ Loop    │ │ Engine   │ │ (learning)  │ │
│  └────┬────┘ └────┬─────┘ └──────┬──────┘ │
│       │           │              │          │
│  ┌────┴────┐ ┌────┴─────┐ ┌─────┴──────┐ │
│  │Provider │ │ Tools    │ │ Memory     │ │
│  │(OpenAI) │ │(fs/shell/│ │(SQLite/FTS)│ │
│  │         │ │ mac/AX)  │ │            │ │
│  └─────────┘ └──────────┘ └────────────┘ │
└─────────────────────────────────────────────┘
```

### Crates

| Crate | Purpose |
|-------|---------|
| `automaton-proto` | JSON-RPC wire types (requests, events) |
| `automaton-policy` | Deterministic permission engine |
| `automaton-tools` | Tool registry + filesystem/shell/mac tools |
| `automaton-core` | Agent loop, providers, mode profiles |
| `automaton-memory` | SQLite storage + FTS5 + skill loader |
| `automaton-apprentice` | Decision journal + similarity hints |
| `reference/automatond` | Reference daemon binary |

## Safety Model

All tool calls pass through a deterministic policy engine:

1. **Password fields** → hard deny (cannot be overridden)
2. **Mode switching** → always requires approval
3. **Owner grants** → whitelist for "always allow" (tool-scoped)
4. **Explicit deny rules** → banking apps, sensitive areas
5. **Shell classification** → read-only commands auto-allow; build tools require approval; everything else requires approval

## Memory & Learning

automaton learns about you through conversation:

- Agent outputs `[기억: fact]` markers → automatically saved
- Facts are injected into system prompts (cache-friendly: end of history)
- Session summaries generated after idle periods
- Decision journal tracks your approvals for similarity hints

Edit `~/.config/automaton/persona.md` to customize the agent's personality.

## Skills

Skills are markdown files in `~/.config/automaton/skills/`:

```
~/.config/automaton/skills/
├── organize-downloads/SKILL.md
├── find-and-open/SKILL.md
└── git-commit/SKILL.md
```

Compatible with the [agentskills.io](https://agentskills.io) standard.

## Voice (Experimental)

Push-to-talk with `⌘⇧Space`:
- Speech recognition (on-device when available)
- Text-to-speech responses
- Interrupt TTS by typing

## Configuration

| File | Purpose |
|------|---------|
| `~/.config/automaton/persona.md` | Agent personality |
| `~/.config/automaton/policy.toml` | Permission rules |
| `~/.config/automaton/skills/` | Skill definitions |
| `~/.local/share/automaton/memory.db` | Memory (SQLite) |
| `~/.local/share/automaton/audit/` | Audit logs (JSONL) |

## Diagnostics

```bash
cargo run -p automatond -- doctor
```

Checks accessibility permissions, screen recording, API key, and directory setup.

## License

MIT OR Apache-2.0

## Contributing

PRs welcome. Run `cargo test --workspace` before submitting.
