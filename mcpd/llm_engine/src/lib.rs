//! LLM Engine for ACOS (Agent-Centric OS)
//!
//! A minimal pure-Rust LLM inference engine capable of loading quantized
//! models (GGUF format) and generating text on CPU.
//!
//! # Design Constraints
//!
//! - Pure Rust, no C dependencies
//! - No mmap — uses `std::fs::read()` for model loading
//! - No network at runtime — model weights pre-loaded from filesystem
//! - Target: SmolLM-135M Q4 (~80MB) fitting in <1GB RAM
//! - Cross-compiles for x86_64-unknown-redox (relibc, no glibc)
//!
//! # Features
//!
//! - `host-test` (default): Returns mock responses for development/testing
//! - `redox`: Enables real inference for ACOS deployment

pub mod model;
pub mod tokenizer;
pub mod generate;

use serde::{Serialize, Deserialize};

/// Result of a text generation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResult {
    pub text: String,
    pub tokens_generated: usize,
    pub tokens_per_sec: f64,
}

/// Metadata about the loaded model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub model_name: String,
    pub quantization: String,
    pub ram_mb: usize,
    pub tokens_per_sec: f64,
}

/// The main LLM inference engine.
///
/// Load a model from disk, then call `generate()` to produce text.
pub struct LlmEngine {
    #[cfg(feature = "host-test")]
    info: ModelInfo,

    #[cfg(not(feature = "host-test"))]
    model: model::LoadedModel,

    #[cfg(not(feature = "host-test"))]
    tok: tokenizer::Tokenizer,
}

// ---------------------------------------------------------------------------
// host-test mock implementation
// ---------------------------------------------------------------------------
#[cfg(feature = "host-test")]
impl LlmEngine {
    /// Load a model from the given path.
    ///
    /// In host-test mode this ignores the path and returns a mock engine.
    pub fn load(model_path: &str) -> Result<Self, String> {
        let _ = model_path;
        Ok(Self {
            info: ModelInfo {
                model_name: "mock-smollm-135m".into(),
                quantization: "Q4_0".into(),
                ram_mb: 80,
                tokens_per_sec: 42.0,
            },
        })
    }

    /// Generate text from a prompt.
    ///
    /// In host-test mode this returns a deterministic mock response.
    pub fn generate(&self, prompt: &str, max_tokens: usize) -> Result<GenerateResult, String> {
        let mock_text = format!(
            "[mock] Responding to '{}' with {} tokens",
            prompt.chars().take(40).collect::<String>(),
            max_tokens
        );
        Ok(GenerateResult {
            text: mock_text,
            tokens_generated: max_tokens.min(32),
            tokens_per_sec: self.info.tokens_per_sec,
        })
    }

    /// Return metadata about the loaded model.
    pub fn info(&self) -> ModelInfo {
        self.info.clone()
    }
}

// ---------------------------------------------------------------------------
// Real inference implementation (used when host-test is NOT enabled)
// ---------------------------------------------------------------------------
#[cfg(not(feature = "host-test"))]
impl LlmEngine {
    /// Load a GGUF model from the filesystem.
    ///
    /// Reads the entire file into memory via `std::fs::read()` (no mmap).
    pub fn load(model_path: &str) -> Result<Self, String> {
        let model = model::LoadedModel::from_gguf(model_path)?;
        let tok = tokenizer::Tokenizer::from_model(&model)?;
        Ok(Self { model, tok })
    }

    /// Generate text from a prompt.
    ///
    /// Wraps the prompt in a chat template for instruct models:
    /// `<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n`
    pub fn generate(&self, prompt: &str, max_tokens: usize) -> Result<GenerateResult, String> {
        // Format with chat template (works for both base and instruct models)
        let formatted = format!(
            "<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
            prompt
        );
        let mut input_ids = Vec::new();
        input_ids.extend(self.tok.encode(&formatted));
        let result = generate::generate_tokens(
            &self.model,
            &self.tok,
            &input_ids,
            max_tokens,
        )?;
        Ok(result)
    }

    /// Return metadata about the loaded model.
    pub fn info(&self) -> ModelInfo {
        ModelInfo {
            model_name: self.model.name.clone(),
            quantization: self.model.quantization.clone(),
            ram_mb: self.model.ram_bytes / (1024 * 1024),
            tokens_per_sec: 0.0, // updated after first generation
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_load_and_generate() {
        let engine = LlmEngine::load("/nonexistent/model.gguf").unwrap();
        let info = engine.info();
        assert_eq!(info.model_name, "mock-smollm-135m");
        assert_eq!(info.quantization, "Q4_0");

        let result = engine.generate("Hello world", 16).unwrap();
        assert!(result.text.contains("[mock]"));
        assert_eq!(result.tokens_generated, 16);
    }
}
