//! Inference budget limits shared by LLM backends.

use crate::types::InferenceRequest;
use std::time::Duration;

/// Runtime limits applied by a backend before it performs inference.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InferenceBudget {
    /// Maximum serialized prompt size accepted by a backend.
    pub max_prompt_bytes: usize,
    /// Maximum number of tokens a backend may generate.
    pub max_output_tokens: u32,
    /// Maximum number of tool-call iterations allowed for one request.
    pub max_tool_iterations: u32,
    /// End-to-end backend timeout.
    pub timeout: Duration,
}

impl InferenceBudget {
    /// Construct a new inference budget.
    pub const fn new(
        max_prompt_bytes: usize,
        max_output_tokens: u32,
        max_tool_iterations: u32,
        timeout: Duration,
    ) -> Self {
        Self {
            max_prompt_bytes,
            max_output_tokens,
            max_tool_iterations,
            timeout,
        }
    }

    /// Validate a request against this budget.
    pub fn validate(&self, request: &InferenceRequest) -> Result<(), BudgetViolation> {
        let prompt_bytes = request.prompt_bytes();
        if prompt_bytes > self.max_prompt_bytes {
            return Err(BudgetViolation::PromptBytes {
                actual: prompt_bytes,
                limit: self.max_prompt_bytes,
            });
        }

        if request.requested_output_tokens > self.max_output_tokens {
            return Err(BudgetViolation::OutputTokens {
                requested: request.requested_output_tokens,
                limit: self.max_output_tokens,
            });
        }

        if request.tool_iterations > self.max_tool_iterations {
            return Err(BudgetViolation::ToolIterations {
                requested: request.tool_iterations,
                limit: self.max_tool_iterations,
            });
        }

        Ok(())
    }
}

impl Default for InferenceBudget {
    fn default() -> Self {
        Self::new(64 * 1024, 1024, 8, Duration::from_secs(30))
    }
}

/// Reason a request exceeded an inference budget.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetViolation {
    /// Prompt bytes exceed the backend cap.
    PromptBytes { actual: usize, limit: usize },
    /// Requested output tokens exceed the backend cap.
    OutputTokens { requested: u32, limit: u32 },
    /// Requested tool iterations exceed the backend cap.
    ToolIterations { requested: u32, limit: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChatMessage, InferenceRequest, MessageRole, ModelId};

    #[test]
    fn budget_accepts_request_within_limits() {
        let request =
            InferenceRequest::new(ModelId::new("phi4-mini"), vec![ChatMessage::user("hello")]);

        assert_eq!(InferenceBudget::default().validate(&request), Ok(()));
    }

    #[test]
    fn budget_rejects_prompt_over_cap() {
        let request = InferenceRequest::new(
            ModelId::new("tiny"),
            vec![ChatMessage::new(MessageRole::User, "abcdef")],
        );
        let budget = InferenceBudget::new(5, 16, 1, Duration::from_secs(1));

        assert_eq!(
            budget.validate(&request),
            Err(BudgetViolation::PromptBytes {
                actual: 6,
                limit: 5,
            })
        );
    }

    #[test]
    fn budget_rejects_tool_iteration_over_cap() {
        let request = InferenceRequest::new(ModelId::new("tiny"), Vec::new())
            .with_requested_output_tokens(16)
            .with_tool_iterations(2);
        let budget = InferenceBudget::new(1024, 16, 1, Duration::from_secs(1));

        assert_eq!(
            budget.validate(&request),
            Err(BudgetViolation::ToolIterations {
                requested: 2,
                limit: 1,
            })
        );
    }
}
