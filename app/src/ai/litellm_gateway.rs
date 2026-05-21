/// LiteLLM gateway integration.
///
/// Translates `warp_multi_agent_api::Request` → LiteLLM `/v1/chat/completions`
/// (OpenAI-compatible streaming) and maps SSE chunks back to
/// `warp_multi_agent_api::ResponseEvent`.
///
/// Auth: the OpenAI API Key field in Settings is used as the LiteLLM Bearer
/// token. Set `WARP_LLM_BYOK_BASE_URL` to point to your LiteLLM instance, or
/// configure the URL in Settings → AI → LiteLLM Gateway URL.
///
/// Available models are discovered at runtime via `GET /v1/models` and cached
/// for [`MODELS_CACHE_TTL`]. The static mapping in [`warp_model_to_litellm_id`]
/// translates Warp's internal model IDs to LiteLLM aliases; if the mapped alias
/// is not present in the live model list the request fails with a clear error.
pub mod litellm_gateway {
    use anyhow::{Context as _, Result};
    use futures::Stream;
    use serde::{Deserialize, Serialize};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};
    use tokio::sync::RwLock;
    use uuid::Uuid;
    use warp_multi_agent_api::{
        client_action::{AppendToMessageContent, BeginTransaction, CommitTransaction, CreateTask},
        message as msg,
        response_event::{
            stream_finished, ClientActions, StreamFinished, StreamInit, Type as ResponseEventType,
        },
        ClientAction, Message, Request, ResponseEvent, Task,
    };

    const DEFAULT_LLM_BYOK_BASE_URL: &str = "http://localhost:4000";

    /// How long to reuse a cached model list before re-fetching.
    const MODELS_CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

    /// Resolves the gateway URL from env vars / .env file, falling back to the compiled default.
    /// The Settings UI override is applied earlier, in `stream_litellm_response`.
    fn llm_byok_base_url() -> String {
        // Load .env on first call; silently ignores a missing file.
        let _ = dotenvy::dotenv();

        std::env::var("WARP_LLM_BYOK_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_LLM_BYOK_BASE_URL.to_string())
    }

    // ── Models cache ──────────────────────────────────────────────────────────

    struct ModelsCache {
        models: Vec<String>,
        fetched_at: Instant,
    }

    static MODELS_CACHE: OnceLock<RwLock<Option<ModelsCache>>> = OnceLock::new();

    fn models_cache() -> &'static RwLock<Option<ModelsCache>> {
        MODELS_CACHE.get_or_init(|| RwLock::new(None))
    }

    #[derive(Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelObject>,
    }

    #[derive(Deserialize)]
    struct ModelObject {
        id: String,
    }

    /// Fetches the available model IDs from `GET /v1/models`.
    async fn fetch_available_models(
        client: &reqwest::Client,
        base_url: &str,
        api_key: &str,
    ) -> Result<Vec<String>> {
        let url = format!("{base_url}/v1/models");
        let resp = client
            .get(&url)
            .bearer_auth(api_key)
            .send()
            .await
            .context("Failed to connect to LLM BYOK gateway to list models")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("GET /v1/models returned {status}: {body}");
        }

        let models_resp: ModelsResponse = resp
            .json()
            .await
            .context("Failed to parse /v1/models response")?;

        let ids: Vec<String> = models_resp.data.into_iter().map(|m| m.id).collect();
        log::debug!("LLM BYOK available models: {ids:?}");
        Ok(ids)
    }

    /// Returns cached model list, refreshing it if the TTL has elapsed.
    async fn available_models(
        client: &reqwest::Client,
        base_url: &str,
        api_key: &str,
    ) -> Result<Vec<String>> {
        // Fast path: valid cache exists.
        {
            let guard = models_cache().read().await;
            if let Some(cache) = guard.as_ref() {
                if cache.fetched_at.elapsed() < MODELS_CACHE_TTL {
                    return Ok(cache.models.clone());
                }
            }
        }

        // Slow path: fetch and store.
        let models = fetch_available_models(client, base_url, api_key).await?;
        let mut guard = models_cache().write().await;
        *guard = Some(ModelsCache {
            models: models.clone(),
            fetched_at: Instant::now(),
        });
        Ok(models)
    }

    // ── Model resolution ──────────────────────────────────────────────────────

    /// Best-effort mapping from Warp model IDs to LiteLLM BYOK aliases.
    /// Used as a hint; the resolved alias is validated against the live model list.
    fn warp_model_to_litellm_id(warp_model_id: &str) -> Option<&'static str> {
        match warp_model_id {
            "claude-4-7-opus-high" | "claude-4-7-opus-xhigh" | "claude-4-7-opus-max" => {
                Some("us.anthropic.claude-opus-4-7-20251101-v1:0")
            }
            "claude-4-6-opus-high" | "claude-4-6-opus-max" => {
                Some("us.anthropic.claude-opus-4-6-v1")
            }
            "claude-4-6-sonnet-high" | "claude-4-6-sonnet-max" => {
                Some("us.anthropic.claude-sonnet-4-6")
            }
            "claude-4-5-haiku" => Some("us.anthropic.claude-haiku-4-5-20251001-v1:0"),
            _ => None,
        }
    }

    /// Scores a model ID by capability tier so we can pick the best available
    /// without hardcoding specific version strings.
    ///
    /// Higher score = preferred. Returns 0 for unknown models (they still
    /// participate as last-resort candidates).
    fn model_preference_score(id: &str) -> u8 {
        let id = id.to_lowercase();
        if id.contains("opus") {
            // Opus is the most capable but slowest; good default for agent mode.
            3
        } else if id.contains("sonnet") {
            2
        } else if id.contains("haiku") || id.contains("mini") || id.contains("lite") {
            1
        } else {
            0
        }
    }

    /// Resolves a Warp model ID to a LiteLLM model alias confirmed to exist in
    /// the live model list.
    ///
    /// Resolution order:
    /// 1. Static mapping (`warp_model_to_litellm_id`) → validated against live list.
    /// 2. Direct match of the Warp ID in the live list.
    /// 3. For `"auto"` or any unrecognised ID: pick the highest-scoring model
    ///    from the live list using `model_preference_score`.
    /// 4. If the list is empty, error.
    async fn resolve_model(
        warp_model_id: &str,
        client: &reqwest::Client,
        base_url: &str,
        api_key: &str,
    ) -> Result<String> {
        let models = available_models(client, base_url, api_key).await?;

        // 1. Try static mapping.
        if let Some(alias) = warp_model_to_litellm_id(warp_model_id) {
            if models.contains(&alias.to_string()) {
                log::info!("Routing to LLM BYOK: {warp_model_id} → {alias}");
                return Ok(alias.to_string());
            }
            log::warn!(
                "Static alias '{alias}' for '{warp_model_id}' not in live model list; \
                 falling through"
            );
        }

        // 2. Direct match.
        if models.contains(&warp_model_id.to_string()) {
            log::info!("Routing to LLM BYOK: {warp_model_id} (direct match)");
            return Ok(warp_model_id.to_string());
        }

        // 3. Pick best available by capability tier.
        let best = models
            .iter()
            .max_by_key(|m| model_preference_score(m))
            .ok_or_else(|| anyhow::anyhow!("LLM BYOK gateway returned an empty model list"))?;

        log::info!(
            "Model '{warp_model_id}' not recognised; auto-selecting best available: '{best}'"
        );
        Ok(best.clone())
    }

    // ── OpenAI-compatible request/response types ──────────────────────────────

    #[derive(Serialize)]
    struct ChatRequest<'a> {
        model: &'a str,
        messages: Vec<ChatMessage>,
        stream: bool,
    }

    /// A single content block inside a multimodal message.
    #[derive(Serialize, Clone)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum ContentBlock {
        Text { text: String },
        ImageUrl { image_url: ImageUrlPayload },
    }

    #[derive(Serialize, Clone)]
    struct ImageUrlPayload {
        /// Data URI: `data:<mime>;base64,<data>`
        url: String,
    }

    /// A chat message whose content may be plain text (string) or multimodal
    /// (array of content blocks). LiteLLM accepts both forms.
    #[derive(Serialize)]
    #[serde(untagged)]
    enum ChatMessageContent {
        Text(String),
        Multimodal(Vec<ContentBlock>),
    }

    #[derive(Serialize)]
    struct ChatMessage {
        role: &'static str,
        content: ChatMessageContent,
    }

    impl ChatMessage {
        fn text(role: &'static str, text: String) -> Self {
            Self {
                role,
                content: ChatMessageContent::Text(text),
            }
        }

        fn multimodal(role: &'static str, blocks: Vec<ContentBlock>) -> Self {
            Self {
                role,
                content: ChatMessageContent::Multimodal(blocks),
            }
        }
    }

    #[derive(Deserialize)]
    struct StreamChunk {
        choices: Vec<StreamChoice>,
    }

    #[derive(Deserialize)]
    struct StreamChoice {
        delta: DeltaContent,
        finish_reason: Option<String>,
    }

    #[derive(Deserialize)]
    struct DeltaContent {
        content: Option<String>,
    }

    // ── Build messages from conversation history ──────────────────────────────

    fn build_chat_messages(request: &Request) -> Vec<ChatMessage> {
        let mut out: Vec<ChatMessage> = Vec::new();

        for task in request
            .task_context
            .as_ref()
            .map_or(&[][..], |tc| tc.tasks.as_slice())
        {
            for message in &task.messages {
                match &message.message {
                    Some(msg::Message::UserQuery(uq)) if !uq.query.is_empty() => {
                        out.push(ChatMessage::text("user", uq.query.clone()));
                    }
                    Some(msg::Message::AgentOutput(ao)) if !ao.text.is_empty() => {
                        out.push(ChatMessage::text("assistant", ao.text.clone()));
                    }
                    _ => {}
                }
            }
        }

        if let Some(input) = &request.input {
            use warp_multi_agent_api::request::input::{
                user_inputs::user_input::Input as UserInputVariant, Type as InputType,
            };
            if let Some(InputType::UserInputs(ui)) = &input.r#type {
                // Collect images from the top-level input context.
                let image_blocks: Vec<ContentBlock> = input
                    .context
                    .as_ref()
                    .map_or(&[][..], |ctx| ctx.images.as_slice())
                    .iter()
                    .filter(|img| !img.data.is_empty())
                    .map(|img| {
                        let mime = if img.mime_type.is_empty() {
                            "image/png"
                        } else {
                            img.mime_type.as_str()
                        };
                        // The Warp client currently base64-encodes the image bytes before
                        // stuffing them into the proto `bytes` field (see the TODO in the
                        // proto definition). Interpret the bytes as a UTF-8 base64 string
                        // directly instead of re-encoding them.
                        let b64 = String::from_utf8_lossy(&img.data);
                        ContentBlock::ImageUrl {
                            image_url: ImageUrlPayload {
                                url: format!("data:{mime};base64,{b64}"),
                            },
                        }
                    })
                    .collect();

                for ui_input in &ui.inputs {
                    if let Some(UserInputVariant::UserQuery(uq)) = &ui_input.input {
                        if uq.query.is_empty() && image_blocks.is_empty() {
                            continue;
                        }
                        if image_blocks.is_empty() {
                            out.push(ChatMessage::text("user", uq.query.clone()));
                        } else {
                            // Multimodal: text first, then images.
                            let mut blocks = Vec::with_capacity(1 + image_blocks.len());
                            if !uq.query.is_empty() {
                                blocks.push(ContentBlock::Text {
                                    text: uq.query.clone(),
                                });
                            }
                            blocks.extend(image_blocks.iter().cloned());
                            out.push(ChatMessage::multimodal("user", blocks));
                        }
                    }
                }
            }
        }

        out
    }

    // ── Main streaming function ───────────────────────────────────────────────

    /// Stream a LiteLLM BYOK response and yield `ResponseEvent`s that mirror what
    /// the Warp server would normally produce.
    ///
    /// The BYOK API key is read from `request.settings.api_keys.openai`.
    ///
    /// `gateway_url_override`: when non-empty it takes priority over env vars and the default.
    pub async fn stream_litellm_response(
        request: &Request,
        gateway_url_override: &str,
    ) -> Result<impl Stream<Item = Result<ResponseEvent>>> {
        let api_key = request
            .settings
            .as_ref()
            .and_then(|s| s.api_keys.as_ref())
            .map(|k| k.openai.as_str())
            .filter(|k| !k.is_empty())
            .context(
                "No LLM BYOK API key found. Paste your sk-... token into \
                 Warp Settings → AI → OpenAI API key.",
            )?
            .to_string();

        let warp_model_id = request
            .settings
            .as_ref()
            .and_then(|s| s.model_config.as_ref())
            .map(|mc| mc.base.as_str())
            .unwrap_or("claude-4-6-sonnet-high");

        let base_url = if !gateway_url_override.is_empty() {
            gateway_url_override.to_string()
        } else {
            llm_byok_base_url()
        };
        let client = reqwest::Client::new();

        let litellm_model = resolve_model(warp_model_id, &client, &base_url, &api_key).await?;

        let messages = build_chat_messages(request);
        if messages.is_empty() {
            anyhow::bail!("No messages to send to LLM BYOK gateway");
        }

        let url = format!("{base_url}/v1/chat/completions");
        let resp = client
            .post(&url)
            .bearer_auth(&api_key)
            .json(&ChatRequest {
                model: &litellm_model,
                messages,
                stream: true,
            })
            .send()
            .await
            .context("Failed to connect to LLM BYOK gateway")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("LLM BYOK gateway returned {status}: {body}");
        }

        let conversation_id = Uuid::new_v4().to_string();
        let request_id = Uuid::new_v4().to_string();

        // Determine if this is a follow-up in an established conversation.
        // We consider it established only when the task context already contains
        // at least one AgentOutput — meaning the server has responded at least
        // once. An optimistic task with only user queries (first message, even
        // with image) still counts as a new conversation.
        let existing_task_id = request
            .task_context
            .as_ref()
            .and_then(|tc| tc.tasks.first())
            .filter(|t| {
                t.messages
                    .iter()
                    .any(|m| matches!(m.message, Some(msg::Message::AgentOutput(_))))
            })
            .map(|t| t.id.clone());

        let task_id = existing_task_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let message_id = Uuid::new_v4().to_string();
        let is_new_conversation = existing_task_id.is_none();

        let byte_stream = resp.bytes_stream();

        let stream = async_stream::try_stream! {
            yield ResponseEvent {
                r#type: Some(ResponseEventType::Init(StreamInit {
                    conversation_id: conversation_id.clone(),
                    request_id: request_id.clone(),
                    run_id: String::new(),
                })),
            };

            yield ResponseEvent {
                r#type: Some(ResponseEventType::ClientActions(ClientActions {
                    actions: vec![ClientAction {
                        action: Some(warp_multi_agent_api::client_action::Action::BeginTransaction(
                            BeginTransaction {},
                        )),
                    }],
                })),
            };

            // Only send CreateTask on the first request in a conversation.
            // Follow-ups reuse the existing root task — sending CreateTask again
            // would attempt to re-upgrade an already-Server task (UnexpectedUpgrade).
            if is_new_conversation {
                let root_task = Task {
                    id: task_id.clone(),
                    description: String::new(),
                    messages: vec![Message {
                        id: message_id.clone(),
                        task_id: task_id.clone(),
                        message: Some(msg::Message::AgentOutput(msg::AgentOutput {
                            text: String::new(),
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                yield ResponseEvent {
                    r#type: Some(ResponseEventType::ClientActions(ClientActions {
                        actions: vec![ClientAction {
                            action: Some(warp_multi_agent_api::client_action::Action::CreateTask(
                                CreateTask { task: Some(root_task) },
                            )),
                        }],
                    })),
                };
            } else {
                // Add a new empty AgentOutput message to the existing task so
                // that AppendToMessageContent can find it by message_id.
                use warp_multi_agent_api::client_action::AddMessagesToTask;
                yield ResponseEvent {
                    r#type: Some(ResponseEventType::ClientActions(ClientActions {
                        actions: vec![ClientAction {
                            action: Some(warp_multi_agent_api::client_action::Action::AddMessagesToTask(
                                AddMessagesToTask {
                                    task_id: task_id.clone(),
                                    messages: vec![Message {
                                        id: message_id.clone(),
                                        task_id: task_id.clone(),
                                        message: Some(msg::Message::AgentOutput(msg::AgentOutput {
                                            text: String::new(),
                                        })),
                                        ..Default::default()
                                    }],
                                },
                            )),
                        }],
                    })),
                };
            }

            use futures::StreamExt as _;
            let mut byte_stream = byte_stream;
            let mut buffer = String::new();

            'outer: loop {
                match byte_stream.next().await {
                    None => break 'outer,
                    Some(Err(e)) => {
                        Err(anyhow::anyhow!("LLM BYOK stream error: {e}"))?;
                        break 'outer;
                    }
                    Some(Ok(bytes)) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));

                        while let Some(newline_pos) = buffer.find('\n') {
                            let line = buffer[..newline_pos].trim().to_string();
                            buffer = buffer[newline_pos + 1..].to_string();

                            if line == "data: [DONE]" {
                                break 'outer;
                            }

                            let data = match line.strip_prefix("data: ") {
                                Some(d) if !d.is_empty() => d,
                                _ => continue,
                            };

                            let chunk: StreamChunk = match serde_json::from_str(data) {
                                Ok(c) => c,
                                Err(_) => continue,
                            };

                            for choice in &chunk.choices {
                                if choice.finish_reason.as_deref() == Some("stop") {
                                    break 'outer;
                                }
                                if let Some(text) = &choice.delta.content {
                                    if !text.is_empty() {
                                        let append_msg = Message {
                                            id: message_id.clone(),
                                            task_id: task_id.clone(),
                                            message: Some(msg::Message::AgentOutput(
                                                msg::AgentOutput { text: text.clone() },
                                            )),
                                            ..Default::default()
                                        };
                                        yield ResponseEvent {
                                            r#type: Some(ResponseEventType::ClientActions(
                                                ClientActions {
                                                    actions: vec![ClientAction {
                                                        action: Some(warp_multi_agent_api::client_action::Action::AppendToMessageContent(
                                                            AppendToMessageContent {
                                                                task_id: task_id.clone(),
                                                                message: Some(append_msg),
                                                                mask: Some(prost_types::FieldMask {
                                                                    paths: vec![
                                                                        "agent_output.text".to_string(),
                                                                    ],
                                                                }),
                                                            },
                                                        )),
                                                    }],
                                                },
                                            )),
                                        };
                                    }
                                }
                            }
                        }
                    }
                }
            }

            yield ResponseEvent {
                r#type: Some(ResponseEventType::ClientActions(ClientActions {
                    actions: vec![ClientAction {
                        action: Some(warp_multi_agent_api::client_action::Action::CommitTransaction(
                            CommitTransaction { ..Default::default() },
                        )),
                    }],
                })),
            };

            yield ResponseEvent {
                r#type: Some(ResponseEventType::Finished(StreamFinished {
                    reason: Some(stream_finished::Reason::Done(stream_finished::Done {})),
                    ..Default::default()
                })),
            };
        };

        Ok(stream)
    }
}
