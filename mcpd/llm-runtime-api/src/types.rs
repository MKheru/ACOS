//! Core LLM runtime API types.

use crate::budget::InferenceBudget;
use std::error::Error;
use std::fmt;

/// Stable identifier for a configured model.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ModelId(String);

impl ModelId {
    /// Create a model identifier from a non-empty string.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Borrow the raw model identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ModelId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for ModelId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Backend family used to route a request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LlmBackendKind {
    /// Local Rust-native backend, expected to wrap mistral.rs in later WS12 work.
    LocalMistralRs,
    /// Network proxy preserving the existing OpenAI-compatible dispatch path.
    NetOpenAiProxy,
    /// Deterministic backend for tests and boot smoke checks.
    Mock,
}

/// Role of one chat message in an inference request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageRole {
    /// System instruction.
    System,
    /// End-user request.
    User,
    /// Assistant output.
    Assistant,
    /// Tool result.
    Tool,
}

/// One chat message sent to a backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatMessage {
    /// Message role.
    pub role: MessageRole,
    /// Message content.
    pub content: String,
}

impl ChatMessage {
    /// Create a chat message.
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self::new(MessageRole::User, content)
    }

    /// Number of bytes in the message content.
    pub fn content_bytes(&self) -> usize {
        self.content.len()
    }
}

/// Request passed to an LLM backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferenceRequest {
    /// Target model.
    pub model: ModelId,
    /// Chat messages in backend order.
    pub messages: Vec<ChatMessage>,
    /// Limits the backend must enforce before inference.
    pub budget: InferenceBudget,
    /// Requested output-token cap.
    pub requested_output_tokens: u32,
    /// Tool-call loop count already consumed by this request.
    pub tool_iterations: u32,
}

impl InferenceRequest {
    /// Construct a request with the default budget.
    pub fn new(model: ModelId, messages: Vec<ChatMessage>) -> Self {
        let budget = InferenceBudget::default();
        Self {
            model,
            messages,
            requested_output_tokens: budget.max_output_tokens,
            tool_iterations: 0,
            budget,
        }
    }

    /// Override the request budget.
    pub fn with_budget(mut self, budget: InferenceBudget) -> Self {
        self.requested_output_tokens = self.requested_output_tokens.min(budget.max_output_tokens);
        self.budget = budget;
        self
    }

    /// Override the requested output-token cap.
    pub fn with_requested_output_tokens(mut self, requested_output_tokens: u32) -> Self {
        self.requested_output_tokens = requested_output_tokens;
        self
    }

    /// Override the current tool-iteration count.
    pub fn with_tool_iterations(mut self, tool_iterations: u32) -> Self {
        self.tool_iterations = tool_iterations;
        self
    }

    /// Total prompt bytes across all message contents.
    pub fn prompt_bytes(&self) -> usize {
        self.messages.iter().map(ChatMessage::content_bytes).sum()
    }
}

/// Token usage returned by a backend.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TokenUsage {
    /// Prompt tokens consumed.
    pub prompt_tokens: u32,
    /// Completion tokens generated.
    pub completion_tokens: u32,
}

impl TokenUsage {
    /// Total tokens consumed by the request.
    pub fn total_tokens(self) -> u32 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }
}

/// Why a backend stopped generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishReason {
    /// The model reached a natural stop point.
    Stop,
    /// The output-token budget stopped generation.
    Length,
    /// The model requested a tool call.
    ToolCall,
    /// The backend stopped for a safety or policy reason.
    Safety,
}

/// Response returned by an LLM backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferenceResponse {
    /// Model that produced the response.
    pub model: ModelId,
    /// Assistant text output.
    pub output: String,
    /// Backend usage counters.
    pub usage: TokenUsage,
    /// Stop reason.
    pub finish_reason: FinishReason,
}

impl InferenceResponse {
    /// Construct a text response with zero usage counters.
    pub fn text(model: ModelId, output: impl Into<String>) -> Self {
        Self {
            model,
            output: output.into(),
            usage: TokenUsage::default(),
            finish_reason: FinishReason::Stop,
        }
    }
}

/// Error returned by an LLM backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LlmError {
    /// Request exceeded an inference budget.
    BudgetExceeded(String),
    /// Requested model is unknown to the backend.
    ModelNotFound(ModelId),
    /// Caller lacks authority for the requested operation.
    Unauthorized(String),
    /// Backend timed out.
    Timeout,
    /// Backend failed for an implementation-specific reason.
    Backend(String),
}

impl fmt::Display for LlmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BudgetExceeded(reason) => write!(formatter, "budget exceeded: {reason}"),
            Self::ModelNotFound(model) => write!(formatter, "model not found: {model}"),
            Self::Unauthorized(reason) => write!(formatter, "unauthorized: {reason}"),
            Self::Timeout => formatter.write_str("backend timeout"),
            Self::Backend(reason) => write!(formatter, "backend error: {reason}"),
        }
    }
}

impl Error for LlmError {}

/// Common trait implemented by concrete LLM backends.
pub trait LlmBackend {
    /// Return the backend family.
    fn kind(&self) -> LlmBackendKind;

    /// Run inference for one request.
    fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse, LlmError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoBackend;

    impl LlmBackend for EchoBackend {
        fn kind(&self) -> LlmBackendKind {
            LlmBackendKind::Mock
        }

        fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse, LlmError> {
            request
                .budget
                .validate(&request)
                .map_err(|violation| LlmError::BudgetExceeded(format!("{violation:?}")))?;
            Ok(InferenceResponse::text(
                request.model,
                request
                    .messages
                    .last()
                    .map(|message| message.content.clone())
                    .unwrap_or_default(),
            ))
        }
    }

    #[test]
    fn model_id_round_trips_as_str() {
        let model = ModelId::new("qwen2.5");

        assert_eq!(model.as_str(), "qwen2.5");
        assert_eq!(model.to_string(), "qwen2.5");
    }

    #[test]
    fn inference_request_counts_prompt_bytes() {
        let request = InferenceRequest::new(
            ModelId::new("mock"),
            vec![ChatMessage::user("ab"), ChatMessage::user("cde")],
        );

        assert_eq!(request.prompt_bytes(), 5);
    }

    #[test]
    fn backend_trait_can_return_response() {
        let response = EchoBackend
            .infer(InferenceRequest::new(
                ModelId::new("mock"),
                vec![ChatMessage::user("hello")],
            ))
            .expect("echo backend should infer");

        assert_eq!(response.model, ModelId::new("mock"));
        assert_eq!(response.output, "hello");
        assert_eq!(response.finish_reason, FinishReason::Stop);
    }
}
