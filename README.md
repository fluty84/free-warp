<a href="https://www.warp.dev">
    <img width="1024" alt="Warp Agentic Development Environment product preview" src="https://github.com/user-attachments/assets/9976b2da-2edd-4604-a36c-8fd53719c6d4" />
</a>

<h1 align="center">free-warp</h1>

<p align="center">
  A fork of the <a href="https://github.com/warpdotdev/warp">Warp open-source client</a> that replaces
  Warp's cloud backend with any <a href="https://github.com/BerriAI/litellm">LiteLLM</a>-compatible
  gateway, so you can run the full Warp agent without a Warp account or credits.
</p>

---

## Quick start

### 1 — Get a key from your LiteLLM gateway

free-warp uses the **OpenAI API Key** field in Settings as the Bearer token sent
to your LiteLLM instance. It does **not** call OpenAI — the field is simply how
the key reaches the app.

Get your key from your LiteLLM instance:

- **Self-hosted LiteLLM** — create a virtual key in your LiteLLM dashboard
  (`/ui` → Virtual Keys) or via the API:
  ```bash
  curl -X POST https://your-litellm.example.com/key/generate \
    -H "Authorization: Bearer <master-key>" \
    -H "Content-Type: application/json" \
    -d '{"duration": "30d"}'
  ```
- **Managed LiteLLM** — obtain a `sk-...` key from your provider's dashboard
  or admin.

### 2 — Build and run

```bash
# Prerequisites: Rust (rustup), Xcode Command Line Tools (macOS)
git clone https://github.com/fluty84/free-warp.git
cd free-warp

# Build (SKIP_METAL_SHADERS avoids requiring full Xcode)
SKIP_METAL_SHADERS=1 cargo build --bin warp-oss --features litellm_gateway

# Run
./target/debug/warp-oss
```

> **Production build** (better performance): omit `SKIP_METAL_SHADERS=1` and
> make sure full Xcode is installed.

### 3 — Enter your key in Settings

1. Open **Settings** (⌘,) → **AI**
2. Scroll to **API Keys**
3. Paste your `sk-...` token into the **OpenAI API Key** field

   > This field is repurposed as the LiteLLM Bearer token. The key is stored
   > locally and never sent to any Warp server.

4. Close Settings and start a conversation with `/agent`

### 4 — Point to your gateway

Set the environment variable to your LiteLLM instance before running:

```bash
# Copy the example and fill in your values
cp .env.example .env
# edit .env, then:
source .env && ./target/debug/warp-oss
```

The default when `WARP_LLM_BYOK_BASE_URL` is not set is `http://localhost:4000`
(standard LiteLLM local dev port).

---

## How it works

This fork adds a `litellm_gateway` Cargo feature that, when enabled, bypasses
Warp's servers entirely and routes every agent request to a LiteLLM-compatible
gateway using your key as a Bearer token.

### No login required

`litellm_gateway` implies `skip_login` — the app starts without a Warp account.

### Dynamic model discovery

On the first request, the client calls `GET /v1/models` with your API key and
caches the result for 24 hours. Model resolution works as follows:

1. **Static alias** — a built-in mapping translates Warp's internal model IDs
   (`claude-4-6-sonnet-high`, etc.) to LiteLLM model names.
2. **Direct match** — if the Warp model ID matches a model name in the live
   list, it is used as-is.
3. **Auto / unknown model** — when Warp requests `"auto"` or an unrecognised
   ID, the best available model is chosen by scoring the live list:
   `opus` (3) > `sonnet` (2) > `haiku / mini / lite` (1) > anything else (0).

No model names or versions are hardcoded beyond these family-level tiers.

### Vision / image attachments

Images attached in the Warp chat are forwarded to the model as base64 data URIs
in the standard OpenAI multimodal format (`content: [{type: "image_url", ...}]`).

### Multi-turn conversations

The protocol correctly distinguishes first messages (sends `CreateTask`) from
follow-ups (sends `AddMessagesToTask`). A follow-up is detected when the
request's `task_context` already contains an `AgentOutput`, meaning the server
has responded at least once.

### BYOK settings always unlocked

The "Upgrade to Build plan" gate on the API Keys page is bypassed — you can
enter your key directly in **Settings → AI → OpenAI API Key**.

---

## Configuration

| Environment variable     | Default | Description |
|--------------------------|---------|-------------|
| `WARP_LLM_BYOK_BASE_URL` | `http://localhost:4000` | LiteLLM gateway base URL |

---

## Distributable build (macOS .app / .dmg)

The quick-start above produces a debug binary that reads assets from the source
tree at runtime. For a self-contained build you can share or install like a
normal app, add the `standalone` and `release_bundle` features:

```bash
# Full Xcode required (no SKIP_METAL_SHADERS)
cargo build --release --bin warp-oss \
  --features litellm_gateway,standalone,release_bundle
```

The `release_bundle` feature embeds all assets directly into the binary so it
runs from any location without the source tree.

### Creating a .app bundle

Use [`cargo-bundle`](https://github.com/burtonageo/cargo-bundle) to wrap the
binary in a macOS `.app`:

```bash
cargo install cargo-bundle
cargo bundle --release --bin warp-oss \
  --features litellm_gateway,standalone,release_bundle
# Output: target/release/bundle/osx/WarpOss.app
```

### Code-signing and notarization (optional)

To distribute outside your own machine, macOS requires the app to be signed and
notarized with an Apple Developer ID certificate. The repo ships a full bundle
script at `script/macos/bundle` that handles signing, `.dmg` creation, and
Apple notarization — it expects Warp's internal signing secrets, so you need to
adapt it with your own Apple Developer ID credentials:

```bash
# With your own certs (set env vars first — see script/macos/bundle for details)
./script/bundle --channel oss --release-tag v0.1.0
```

For personal or team-internal use without notarization, you can ad-hoc sign to
suppress the Gatekeeper "unidentified developer" prompt:

```bash
codesign --force --deep --sign - target/release/bundle/osx/WarpOss.app
# Then right-click → Open the first time to bypass Gatekeeper
```

---

## Upstream

This repository tracks [warpdotdev/warp](https://github.com/warpdotdev/warp).
All upstream licensing terms apply — see [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-AGPL](LICENSE-AGPL).

Changes introduced by this fork:

| File | Change |
|------|--------|
| `app/src/ai/litellm_gateway.rs` | New module — LiteLLM gateway integration |
| `app/src/workspaces/user_workspaces.rs` | Bypass workspace plan gates under `litellm_gateway` |
| `app/Cargo.toml` | Add `litellm_gateway` feature |
| `crates/warpui/build.rs` | `SKIP_METAL_SHADERS` support for dev builds |
