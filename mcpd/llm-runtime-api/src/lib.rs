//! LLM runtime backend API for ACOS.
//!
//! This crate defines the narrow interface between MCP-facing handlers and
//! concrete inference backends. WS12.M1 intentionally keeps the surface
//! dependency-free and synchronous; concrete async/runtime integration belongs
//! to later backend crates.

#![deny(unsafe_code)]

pub mod budget;
pub mod redaction;
pub mod types;

pub use budget::{BudgetViolation, InferenceBudget};
pub use types::{
    ChatMessage, FinishReason, InferenceRequest, InferenceResponse, LlmBackend, LlmBackendKind,
    LlmError, MessageRole, ModelId, TokenUsage,
};
