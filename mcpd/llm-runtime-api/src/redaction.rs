//! Redaction hooks for prompts, tool output, and backend traces.
//!
//! WS12.M1 only defines the module boundary. Enforcement and trace hashing are
//! wired by later WS12 tasks once caller context and observability integration
//! are available.

/// Redacted text payload with the original byte length retained for accounting.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactedText {
    /// Sanitized content safe for logs or downstream untrusted surfaces.
    pub text: String,
    /// Byte length of the original input before redaction.
    pub original_len: usize,
}

impl RedactedText {
    /// Construct a redacted payload.
    pub fn new(text: impl Into<String>, original_len: usize) -> Self {
        Self {
            text: text.into(),
            original_len,
        }
    }
}

/// Placeholder prompt redaction hook.
pub fn redact_prompt_for_trace(prompt: &str) -> RedactedText {
    RedactedText::new("[prompt redacted]", prompt.len())
}

/// Placeholder tool-output redaction hook.
pub fn redact_tool_output(output: &str) -> RedactedText {
    RedactedText::new("[tool output redacted]", output.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_redaction_does_not_return_raw_text() {
        let redacted = redact_prompt_for_trace("secret prompt");

        assert_eq!(redacted.text, "[prompt redacted]");
        assert_eq!(redacted.original_len, 13);
    }
}
