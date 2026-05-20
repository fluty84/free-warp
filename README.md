<a href="https://www.warp.dev">
    <img width="1024" alt="Warp Agentic Development Environment product preview" src="https://github.com/user-attachments/assets/9976b2da-2edd-4604-a36c-8fd53719c6d4" />
</a>

<h1 align="center">free-warp</h1>

<p align="center">
  A fork of the <a href="https://github.com/warpdotdev/warp">Warp open-source client</a> that replaces
  Warp's cloud backend with a personal <a href="https://github.com/BerriAI/litellm">LiteLLM</a> key,
  so you can run the full Warp agent without a Warp account or credits.
</p>

---

## How it works

This fork adds a `direct_bedrock` Cargo feature that, when enabled, bypasses Warp's
servers entirely and routes every agent request straight to a LiteLLM-compatible
gateway using your own API key as a Bearer token.

The reference gateway is Cabify's internal BYOK endpoint (`llm-byok.cabify.tools`),
but any OpenAI-compatible LiteLLM instance works — override it with
`WARP_LLM_BYOK_BASE_URL`.

## Features

### No login required

The `direct_bedrock` feature implies `skip_login`. The app starts without
requiring a Warp account.

### Dynamic model discovery

On the first request, the client calls `GET /v1/models` with your API key and
caches the result for 24 hours. Model resolution works like this:

1. **Static alias** — a built-in mapping translates Warp's internal model IDs
   (`claude-4-6-sonnet-high`, etc.) to LiteLLM model names.
2. **Direct match** — if the Warp model ID matches a live model name literally,
   it is used as-is.
3. **Auto / unknown model** — when Warp requests `"auto"` or an unrecognised ID,
   the best available model is selected automatically by scoring the live list:
   `opus` (3) > `sonnet` (2) > `haiku / mini / lite` (1) > anything else (0).

No model names or versions are hardcoded beyond the family-level scoring heuristic.

### Vision / image attachments

Images attached in the Warp chat are forwarded to the model as base64 data URIs
in the OpenAI multimodal format (`content: [{type: "image_url", ...}]`).

### Multi-turn conversations

The protocol correctly handles both first messages (sends `CreateTask` to
establish the root task) and follow-up messages (sends `AddMessagesToTask` to
append a new response to the existing task). A follow-up is detected when the
request's `task_context` already contains an `AgentOutput` message, meaning the
server has responded at least once.

### BYOK settings always unlocked

The "Upgrade to Build plan" gate on the API Keys settings page is bypassed — you
can enter your personal `sk-...` key directly in **Settings → AI → OpenAI API Key**.

---

## Getting a personal key (Cabify employees)

Follow the [personal keys user guide](https://backstage.cabify.tools/catalog/dev-x/component/llm-ssot/docs/personal-keys-user-guide/)
to request a key from `llm-byok.cabify.tools`.

Once you have your `sk-...`, set it as the **OpenAI API Key** in Warp Settings → AI.

---

## Building

```bash
# Install Rust via rustup (the project pins its toolchain in rust-toolchain.toml)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build the free-warp binary
SKIP_METAL_SHADERS=1 cargo build --bin warp-oss --features direct_bedrock

# Run it
./target/debug/warp-oss
```

`SKIP_METAL_SHADERS=1` skips Metal shader compilation, which requires full Xcode.
The resulting binary works for development and testing; a production build should
omit this flag.

---

## Configuration

| Environment variable     | Default                        | Description                              |
|--------------------------|--------------------------------|------------------------------------------|
| `WARP_LLM_BYOK_BASE_URL` | `https://llm-byok.cabify.tools`| LiteLLM gateway base URL                 |

---

## Upstream

This repository tracks [warpdotdev/warp](https://github.com/warpdotdev/warp).
All upstream licensing terms apply — see [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-AGPL](LICENSE-AGPL).

The changes introduced by this fork live in:

| File | Change |
|------|--------|
| `app/src/ai/bedrock_direct.rs` | New module — LiteLLM gateway integration |
| `app/src/workspaces/user_workspaces.rs` | Bypass workspace plan gates under `direct_bedrock` |
| `app/Cargo.toml` | Add `direct_bedrock` feature |
| `crates/warpui/build.rs` | Support `SKIP_METAL_SHADERS` for dev builds |
