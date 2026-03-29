//! Minimal BPE tokenizer for LLM inference.
//!
//! Pure Rust implementation — no C dependencies.
//! Loads vocabulary and merge rules from the GGUF model metadata
//! or from separate files on disk.

use std::collections::HashMap;

#[cfg(not(feature = "host-test"))]
use crate::model::LoadedModel;

/// A BPE (Byte-Pair Encoding) tokenizer.
pub struct Tokenizer {
    /// Token ID → string piece
    pub vocab: Vec<String>,
    /// String piece → token ID
    token_to_id: HashMap<String, u32>,
    /// BPE merge rules: (piece_a, piece_b) → merged piece, ordered by priority
    merges: Vec<(String, String)>,
    /// Special token IDs
    pub bos_id: u32,
    pub eos_id: u32,
}

impl Tokenizer {
    /// Build a tokenizer from model metadata (vocab + merges).
    #[cfg(not(feature = "host-test"))]
    pub fn from_model(model: &LoadedModel) -> Result<Self, String> {
        if model.vocab.is_empty() {
            return Err("model contains no vocabulary".into());
        }

        let mut token_to_id = HashMap::with_capacity(model.vocab.len());
        for (i, piece) in model.vocab.iter().enumerate() {
            token_to_id.insert(piece.clone(), i as u32);
        }

        let bos_id = token_to_id.get("<s>").copied().unwrap_or(1);
        let eos_id = token_to_id.get("</s>").copied().unwrap_or(2);

        Ok(Self {
            vocab: model.vocab.clone(),
            token_to_id,
            merges: model.merges.clone(),
            bos_id,
            eos_id,
        })
    }

    /// Encode a text string into a sequence of token IDs using BPE.
    ///
    /// Special tokens (e.g. `<|im_start|>`, `<|im_end|>`, `<|endoftext|>`) are
    /// recognized and mapped directly to their token IDs, not passed through BPE.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        // 1. Split text into segments: special tokens vs regular text
        let segments = self.split_special_tokens(text);
        let mut result = Vec::new();

        for seg in segments {
            if let Some(&id) = self.token_to_id.get(&seg) {
                // Special token — emit directly
                result.push(id);
            } else {
                // Regular text — encode with BPE
                result.extend(self.encode_bpe(&seg));
            }
        }
        result
    }

    /// Split text into segments, separating special tokens from regular text.
    fn split_special_tokens(&self, text: &str) -> Vec<String> {
        // Collect special tokens (those matching <|...|> pattern)
        let special: Vec<&str> = self.vocab.iter()
            .filter(|v| v.starts_with("<|") && v.ends_with("|>"))
            .map(String::as_str)
            .collect();

        let mut segments = Vec::new();
        let mut remaining = text;

        while !remaining.is_empty() {
            // Find earliest special token in remaining text
            let mut earliest: Option<(usize, &str)> = None;
            for &sp in &special {
                if let Some(pos) = remaining.find(sp) {
                    if earliest.is_none() || pos < earliest.unwrap().0 {
                        earliest = Some((pos, sp));
                    }
                }
            }

            match earliest {
                Some((pos, sp)) => {
                    if pos > 0 {
                        segments.push(remaining[..pos].to_string());
                    }
                    segments.push(sp.to_string());
                    remaining = &remaining[pos + sp.len()..];
                }
                None => {
                    segments.push(remaining.to_string());
                    break;
                }
            }
        }
        segments
    }

    /// Encode regular text (no special tokens) using byte-level BPE.
    fn encode_bpe(&self, text: &str) -> Vec<u32> {
        if text.is_empty() { return vec![]; }

        // Start with UTF-8 bytes as individual tokens
        let mut pieces: Vec<String> = text.bytes().map(|b| {
            let byte_str = format!("<0x{:02X}>", b);
            if self.token_to_id.contains_key(&byte_str) {
                byte_str
            } else {
                String::from(b as char)
            }
        }).collect();

        // Iteratively apply BPE merges in priority order
        for (left, right) in &self.merges {
            let merged = format!("{}{}", left, right);
            let mut i = 0;
            while i + 1 < pieces.len() {
                if &pieces[i] == left && &pieces[i + 1] == right {
                    pieces[i] = merged.clone();
                    pieces.remove(i + 1);
                } else {
                    i += 1;
                }
            }
        }

        // Map pieces to token IDs
        let unk_id = self.token_to_id.get("<unk>").copied();
        pieces
            .iter()
            .flat_map(|p| {
                if let Some(&id) = self.token_to_id.get(p) {
                    vec![id]
                } else if let Some(uid) = unk_id {
                    vec![uid; p.len().max(1)]
                } else {
                    p.bytes().map(|b| b as u32).collect()
                }
            })
            .collect()
    }

    /// Decode a sequence of token IDs back into a string.
    pub fn decode(&self, tokens: &[u32]) -> String {
        let mut out = String::new();
        for &id in tokens {
            if let Some(piece) = self.vocab.get(id as usize) {
                // Handle byte tokens like <0xFF>
                if piece.starts_with("<0x") && piece.ends_with('>') && piece.len() == 6 {
                    if let Ok(byte) = u8::from_str_radix(&piece[3..5], 16) {
                        out.push(byte as char);
                        continue;
                    }
                }
                // GPT-2 BPE: Ġ (U+0120) represents a space, Ċ (U+010A) represents newline
                let decoded = piece.replace('Ġ', " ").replace('Ċ', "\n");
                out.push_str(&decoded);
            }
        }
        out
    }
}
