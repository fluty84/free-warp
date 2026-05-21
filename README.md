<a href="https://www.warp.dev">
    <img width="1024" alt="Warp Agentic Development Environment product preview" src="https://github.com/user-attachments/assets/9976b2da-2edd-4604-a36c-8fd53719c6d4" />
</a>

<h1 align="center">free-warp</h1>

<p align="center">
  A fork of the <a href="https://github.com/warpdotdev/warp">Warp open-source client</a> that lets you
  route the full Warp agent through any <a href="https://github.com/BerriAI/litellm">LiteLLM</a>-compatible
  gateway — no Warp account or credits required.
</p>

---

## Quick start

### 1 — Build and run

```bash
# Prerequisites: Rust (rustup), Xcode Command Line Tools (macOS)
git clone https://github.com/fluty84/free-warp.git
cd free-warp

# Build (SKIP_METAL_SHADERS avoids requiring full Xcode)
SKIP_METAL_SHADERS=1 cargo build --bin dev

# Run
./target/debug/dev
```

> **Production build**: omit `SKIP_METAL_SHADERS=1` and make sure full Xcode is installed.

### 2 — Choose your mode on first run

On the very first launch a mode picker appears:

- **Warp Cloud** — standard Warp experience, requires a Warp account.
- **LiteLLM Gateway** — routes all AI requests to your own gateway. No login needed.

Your choice is remembered. You can switch at any time in **Settings → AI → LiteLLM Gateway Mode**.

### 3 — Configure the gateway

Once LiteLLM Gateway mode is active, open **Settings → AI** and fill in:

| Field | What to enter |
|-------|--------------|
| **LiteLLM Gateway URL** | Your gateway base URL (default: `http://localhost:4000`) |
| **OpenAI API Key** | Your LiteLLM virtual key (`sk-...`) used as the Bearer token |

> The key is stored locally in an encrypted file and never sent to any Warp server.

**Get your virtual key** from your LiteLLM instance:

- **Self-hosted** — create a key in the LiteLLM dashboard (`/ui` → Virtual Keys) or via the API:
  ```bash
  curl -X POST https://your-litellm.example.com/key/generate \
    -H "Authorization: Bearer <master-key>" \
    -H "Content-Type: application/json" \
    -d '{"duration": "30d"}'
  ```
- **Managed** — obtain a `sk-...` key from your provider's dashboard.

### 4 — Start chatting

Open a terminal tab and start a conversation with `/agent`. Use `/model` to pick from the models your gateway is currently serving.

---

## How it works

### Runtime mode toggle

LiteLLM Gateway mode is a **runtime setting** — no special compile flags needed. Toggle it at any time in **Settings → AI → LiteLLM Gateway Mode**. The `--features litellm_gateway` Cargo flag is still accepted as a shortcut that pre-enables the mode in builds where you always want it on.

### No login required

When LiteLLM Gateway mode is active the app skips Firebase authentication entirely and goes straight to the workspace.

### Dynamic model discovery

`/model` fetches the live model list from `GET /v1/models` on your gateway and shows only models that are actually available. The list refreshes automatically when you change the gateway URL or toggle the mode.

When routing a request, model resolution works as follows:

1. **Static alias** — a built-in mapping translates Warp's internal model IDs (`claude-4-6-sonnet-high`, etc.) to LiteLLM aliases.
2. **Direct match** — if the Warp model ID matches a name in the live list, it is used as-is.
3. **Auto / unknown model** — the best available model is chosen by scoring the live list: `opus` (3) > `sonnet` (2) > `haiku / mini / lite` (1) > anything else (0).

### Vision / image attachments

Images attached in the chat are forwarded as base64 data URIs in the standard OpenAI multimodal format.

### Multi-turn conversations

First messages send `CreateTask`; follow-ups reuse the existing task. A follow-up is detected when the task context already contains an `AgentOutput`.

### No Keychain prompts in dev builds

Every `cargo build` changes the binary hash, which macOS treats as a new app identity — causing repeated Keychain prompts. Dev builds (`debug_assertions`) store secrets in AES-256-GCM encrypted files in the state directory instead of the macOS Keychain. Release builds still use the Keychain.

---

## Configuration

| Environment variable | Default | Description |
|----------------------|---------|-------------|
| `WARP_LLM_BYOK_BASE_URL` | `http://localhost:4000` | Gateway base URL fallback when the Settings field is empty |

You can also set the URL in a `.env` file at the repo root:

```bash
cp .env.example .env
# edit .env with your values
```

---

## Distributable build (macOS .app / .dmg)

The quick-start above produces a debug binary that reads assets from the source tree at runtime. For a self-contained build:

```bash
# Full Xcode required (no SKIP_METAL_SHADERS)
cargo build --release --bin dev --features standalone,release_bundle
```

### Creating a .app bundle

```bash
cargo install cargo-bundle
cargo bundle --release --bin dev --features standalone,release_bundle
# Output: target/release/bundle/osx/WarpOss.app
```

### Code-signing and notarization (optional)

For distribution outside your own machine, macOS requires signing and notarization. The repo ships a bundle script at `script/macos/bundle` that handles signing, `.dmg` creation, and Apple notarization — adapt it with your own Apple Developer ID credentials.

For personal use without notarization, ad-hoc sign to suppress the Gatekeeper prompt:

```bash
codesign --force --deep --sign - target/release/bundle/osx/WarpOss.app
# Right-click → Open the first time to bypass Gatekeeper
```

---

## Upstream

This repository tracks [warpdotdev/warp](https://github.com/warpdotdev/warp).
All upstream licensing terms apply — see [LICENSE-MIT](LICENSE-MIT) and [LICENSE-AGPL](LICENSE-AGPL).

Changes introduced by this fork:

| File / path | Change |
|-------------|--------|
| `app/src/ai/litellm_gateway.rs` | LiteLLM gateway integration (streaming, model discovery) |
| `app/src/ai/llms.rs` | `/model` picker shows only gateway models in LiteLLM mode |
| `app/src/auth/mode_picker.rs` | First-run mode selection screen (Warp Cloud vs LiteLLM Gateway) |
| `app/src/root_view.rs` | Mode picker wired into the startup flow |
| `app/src/settings/ai.rs` | `litellm_mode_enabled` and `litellm_gateway_url` runtime settings |
| `app/src/settings_view/ai_page.rs` | LiteLLM toggle + URL field in Settings → AI |
| `app/src/server/server_api.rs` | Runtime routing to gateway when mode is active |
| `app/src/workspaces/user_workspaces.rs` | BYO API key gate bypassed in LiteLLM mode |
| `app/src/ai/agent/api.rs` | `litellm_gateway_url` always compiled; URL fallback logic |
| `crates/warpui_extras/src/secure_storage/mac.rs` | File-based encrypted storage for macOS dev builds |
| `app/src/lib.rs` | Dev builds use file storage instead of Keychain |
| `app/Cargo.toml` | `litellm_gateway` feature; `dotenvy` always enabled |
| `crates/warpui/build.rs` | `SKIP_METAL_SHADERS` support for dev builds |
