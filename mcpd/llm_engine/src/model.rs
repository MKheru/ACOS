//! GGUF model loading and weight storage.
//!
//! Parses GGUF v2/v3 binary format and stores transformer weights in memory.
//! Uses `std::fs::read()` instead of mmap for ACOS compatibility.
//!
//! Reference: lm.rs (samuel-vitorino/lm.rs) for minimal GGUF parsing approach.
//! GGUF spec: https://github.com/ggerganov/ggml/blob/master/docs/gguf.md

use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// Magic number for GGUF files: "GGUF" in little-endian.
const GGUF_MAGIC: u32 = 0x46554747; // "GGUF" in ASCII, read as little-endian u32

/// Default alignment for the data section.
const DEFAULT_ALIGNMENT: usize = 32;

// GGUF value type constants
const GGUF_TYPE_UINT8: u32 = 0;
const GGUF_TYPE_INT8: u32 = 1;
const GGUF_TYPE_UINT16: u32 = 2;
const GGUF_TYPE_INT16: u32 = 3;
const GGUF_TYPE_UINT32: u32 = 4;
const GGUF_TYPE_INT32: u32 = 5;
const GGUF_TYPE_FLOAT32: u32 = 6;
const GGUF_TYPE_BOOL: u32 = 7;
const GGUF_TYPE_STRING: u32 = 8;
const GGUF_TYPE_ARRAY: u32 = 9;
const GGUF_TYPE_UINT64: u32 = 10;
const GGUF_TYPE_INT64: u32 = 11;
const GGUF_TYPE_FLOAT64: u32 = 12;

/// Supported GGUF quantization types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(non_camel_case_types)]
pub enum QuantType {
    F32 = 0,
    F16 = 1,
    Q4_0 = 2,
    Q4_1 = 3,
    Q5_0 = 6,
    Q5_1 = 7,
    Q8_0 = 8,
    Q8_1 = 9,
    Q2_K = 10,
    Q3_K = 11,
    Q4_K = 12,
    Q5_K = 13,
    Q6_K = 14,
}

impl QuantType {
    fn from_u32(v: u32) -> Result<Self, String> {
        match v {
            0 => Ok(Self::F32),
            1 => Ok(Self::F16),
            2 => Ok(Self::Q4_0),
            3 => Ok(Self::Q4_1),
            6 => Ok(Self::Q5_0),
            7 => Ok(Self::Q5_1),
            8 => Ok(Self::Q8_0),
            9 => Ok(Self::Q8_1),
            10 => Ok(Self::Q2_K),
            11 => Ok(Self::Q3_K),
            12 => Ok(Self::Q4_K),
            13 => Ok(Self::Q5_K),
            14 => Ok(Self::Q6_K),
            _ => Err(format!("unsupported quantization type: {v}")),
        }
    }

    /// Return the block size for this quantization type (number of elements per block).
    pub fn block_size(&self) -> usize {
        match self {
            Self::F32 => 1,
            Self::F16 => 1,
            Self::Q4_0 => 32,
            Self::Q4_1 => 32,
            Self::Q5_0 => 32,
            Self::Q5_1 => 32,
            Self::Q8_0 => 32,
            Self::Q8_1 => 32,
            Self::Q2_K => 256,
            Self::Q3_K => 256,
            Self::Q4_K => 256,
            Self::Q5_K => 256,
            Self::Q6_K => 256,
        }
    }

    /// Return the byte size of one block for this quantization type.
    pub fn type_size(&self) -> usize {
        match self {
            Self::F32 => 4,
            Self::F16 => 2,
            Self::Q4_0 => 18,   // 2 (scale) + 16 (4bit × 32)
            Self::Q4_1 => 20,   // 2 (scale) + 2 (min) + 16
            Self::Q5_0 => 22,   // 2 + 4 + 16
            Self::Q5_1 => 24,   // 2 + 2 + 4 + 16
            Self::Q8_0 => 34,   // 2 (scale) + 32 (8bit × 32)
            Self::Q8_1 => 40,   // 4 (scale) + 4 (min) + 32
            Self::Q2_K => 84,
            Self::Q3_K => 110,
            Self::Q4_K => 144,
            Self::Q5_K => 176,
            Self::Q6_K => 210,
        }
    }

    /// Calculate total bytes needed for `n_elements` of this quantization type.
    pub fn tensor_bytes(&self, n_elements: usize) -> usize {
        let bs = self.block_size();
        let n_blocks = (n_elements + bs - 1) / bs;
        n_blocks * self.type_size()
    }
}

/// A parsed GGUF metadata value.
#[derive(Debug, Clone)]
pub enum GgufValue {
    Uint8(u8),
    Int8(i8),
    Uint16(u16),
    Int16(i16),
    Uint32(u32),
    Int32(i32),
    Float32(f32),
    Bool(bool),
    Str(String),
    Uint64(u64),
    Int64(i64),
    Float64(f64),
    Array(Vec<GgufValue>),
}

impl GgufValue {
    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Self::Uint32(v) => Some(*v),
            Self::Int32(v) => Some(*v as u32),
            Self::Uint64(v) => Some(*v as u32),
            Self::Int64(v) => Some(*v as u32),
            Self::Uint8(v) => Some(*v as u32),
            Self::Uint16(v) => Some(*v as u32),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Uint64(v) => Some(*v),
            Self::Int64(v) => Some(*v as u64),
            Self::Uint32(v) => Some(*v as u64),
            Self::Int32(v) => Some(*v as u64),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Float32(v) => Some(*v),
            Self::Float64(v) => Some(*v as f32),
            _ => None,
        }
    }

    pub fn as_string_array(&self) -> Option<Vec<String>> {
        match self {
            Self::Array(arr) => {
                let mut out = Vec::with_capacity(arr.len());
                for v in arr {
                    match v {
                        Self::Str(s) => out.push(s.clone()),
                        _ => return None,
                    }
                }
                Some(out)
            }
            _ => None,
        }
    }
}

/// A single tensor stored in memory.
#[derive(Debug)]
pub struct Tensor {
    pub name: String,
    pub shape: Vec<usize>,
    pub quant: QuantType,
    pub data: Vec<u8>,
}

/// Transformer model configuration parsed from GGUF metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub vocab_size: usize,
    pub hidden_dim: usize,
    pub intermediate_dim: usize,
    pub n_layers: usize,
    pub n_heads: usize,
    pub n_kv_heads: usize,
    pub max_seq_len: usize,
    pub rope_theta: f32,
}

impl Default for ModelConfig {
    fn default() -> Self {
        // SmolLM-135M defaults
        Self {
            vocab_size: 49152,
            hidden_dim: 576,
            intermediate_dim: 1536,
            n_layers: 30,
            n_heads: 9,
            n_kv_heads: 3,
            max_seq_len: 2048,
            rope_theta: 10000.0,
        }
    }
}

/// Parsed tensor info from the GGUF header (before loading data).
#[derive(Debug)]
struct TensorInfo {
    name: String,
    shape: Vec<usize>,
    quant: QuantType,
    offset: usize,
    n_elements: usize,
}

/// A fully loaded model ready for inference.
#[derive(Debug)]
pub struct LoadedModel {
    pub name: String,
    pub quantization: String,
    pub ram_bytes: usize,
    pub config: ModelConfig,
    pub tensors: Vec<Tensor>,
    pub vocab: Vec<String>,
    pub merges: Vec<(String, String)>,
}

// ---------------------------------------------------------------------------
// Binary reading helpers
// ---------------------------------------------------------------------------

/// Read a little-endian u32 from a byte slice at the given offset.
#[inline]
fn read_u32(data: &[u8], offset: usize) -> Result<u32, String> {
    if offset + 4 > data.len() {
        return Err(format!("unexpected EOF at offset {offset}"));
    }
    Ok(u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// Read a little-endian u64 from a byte slice at the given offset.
#[inline]
fn read_u64(data: &[u8], offset: usize) -> Result<u64, String> {
    if offset + 8 > data.len() {
        return Err(format!("unexpected EOF at offset {offset}"));
    }
    Ok(u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]))
}

/// Read a little-endian i32 from a byte slice.
#[inline]
fn read_i32(data: &[u8], offset: usize) -> Result<i32, String> {
    Ok(read_u32(data, offset)? as i32)
}

/// Read a little-endian f32 from a byte slice.
#[inline]
fn read_f32(data: &[u8], offset: usize) -> Result<f32, String> {
    if offset + 4 > data.len() {
        return Err(format!("unexpected EOF at offset {offset}"));
    }
    Ok(f32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// Read a little-endian f64 from a byte slice.
#[inline]
fn read_f64(data: &[u8], offset: usize) -> Result<f64, String> {
    if offset + 8 > data.len() {
        return Err(format!("unexpected EOF at offset {offset}"));
    }
    Ok(f64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ]))
}

/// Read a UTF-8 string (length-prefixed with u64) from a byte slice.
/// Returns (string, new_offset).
#[inline]
fn read_string(data: &[u8], offset: usize) -> Result<(String, usize), String> {
    let len = read_u64(data, offset)? as usize;
    let start = offset + 8;
    let end = start + len;
    if end > data.len() {
        return Err(format!("string extends past EOF at offset {offset}"));
    }
    let s = std::str::from_utf8(&data[start..end])
        .map_err(|e| format!("invalid UTF-8 in string at offset {offset}: {e}"))?
        .to_string();
    Ok((s, end))
}

// ---------------------------------------------------------------------------
// GGUF value parsing
// ---------------------------------------------------------------------------

/// Read a single GGUF metadata value. Returns (value, new_offset).
fn read_gguf_value(data: &[u8], offset: usize, value_type: u32) -> Result<(GgufValue, usize), String> {
    match value_type {
        GGUF_TYPE_UINT8 => {
            if offset >= data.len() {
                return Err(format!("unexpected EOF reading uint8 at {offset}"));
            }
            Ok((GgufValue::Uint8(data[offset]), offset + 1))
        }
        GGUF_TYPE_INT8 => {
            if offset >= data.len() {
                return Err(format!("unexpected EOF reading int8 at {offset}"));
            }
            Ok((GgufValue::Int8(data[offset] as i8), offset + 1))
        }
        GGUF_TYPE_UINT16 => {
            if offset + 2 > data.len() {
                return Err(format!("unexpected EOF reading uint16 at {offset}"));
            }
            let v = u16::from_le_bytes([data[offset], data[offset + 1]]);
            Ok((GgufValue::Uint16(v), offset + 2))
        }
        GGUF_TYPE_INT16 => {
            if offset + 2 > data.len() {
                return Err(format!("unexpected EOF reading int16 at {offset}"));
            }
            let v = i16::from_le_bytes([data[offset], data[offset + 1]]);
            Ok((GgufValue::Int16(v), offset + 2))
        }
        GGUF_TYPE_UINT32 => {
            let v = read_u32(data, offset)?;
            Ok((GgufValue::Uint32(v), offset + 4))
        }
        GGUF_TYPE_INT32 => {
            let v = read_i32(data, offset)?;
            Ok((GgufValue::Int32(v), offset + 4))
        }
        GGUF_TYPE_FLOAT32 => {
            let v = read_f32(data, offset)?;
            Ok((GgufValue::Float32(v), offset + 4))
        }
        GGUF_TYPE_BOOL => {
            if offset >= data.len() {
                return Err(format!("unexpected EOF reading bool at {offset}"));
            }
            Ok((GgufValue::Bool(data[offset] != 0), offset + 1))
        }
        GGUF_TYPE_STRING => {
            let (s, new_off) = read_string(data, offset)?;
            Ok((GgufValue::Str(s), new_off))
        }
        GGUF_TYPE_ARRAY => {
            let elem_type = read_u32(data, offset)?;
            let count = read_u64(data, offset + 4)? as usize;
            let mut off = offset + 12;
            let mut arr = Vec::with_capacity(count.min(1_000_000));
            for _ in 0..count {
                let (val, new_off) = read_gguf_value(data, off, elem_type)?;
                arr.push(val);
                off = new_off;
            }
            Ok((GgufValue::Array(arr), off))
        }
        GGUF_TYPE_UINT64 => {
            let v = read_u64(data, offset)?;
            Ok((GgufValue::Uint64(v), offset + 8))
        }
        GGUF_TYPE_INT64 => {
            let v = read_u64(data, offset)? as i64;
            Ok((GgufValue::Int64(v), offset + 8))
        }
        GGUF_TYPE_FLOAT64 => {
            let v = read_f64(data, offset)?;
            Ok((GgufValue::Float64(v), offset + 8))
        }
        _ => Err(format!("unknown GGUF value type: {value_type} at offset {offset}")),
    }
}

// ---------------------------------------------------------------------------
// Config extraction from KV metadata
// ---------------------------------------------------------------------------

fn extract_model_config(kv: &HashMap<String, GgufValue>) -> ModelConfig {
    let defaults = ModelConfig::default();

    let hidden_dim = kv.get("llama.embedding_length")
        .and_then(|v| v.as_u32())
        .map(|v| v as usize)
        .unwrap_or(defaults.hidden_dim);

    let n_layers = kv.get("llama.block_count")
        .and_then(|v| v.as_u32())
        .map(|v| v as usize)
        .unwrap_or(defaults.n_layers);

    let n_heads = kv.get("llama.attention.head_count")
        .and_then(|v| v.as_u32())
        .map(|v| v as usize)
        .unwrap_or(defaults.n_heads);

    let n_kv_heads = kv.get("llama.attention.head_count_kv")
        .and_then(|v| v.as_u32())
        .map(|v| v as usize)
        .unwrap_or(defaults.n_kv_heads);

    let ffn_dim = kv.get("llama.feed_forward_length")
        .and_then(|v| v.as_u32())
        .map(|v| v as usize)
        .unwrap_or(defaults.intermediate_dim);

    let ctx_len = kv.get("llama.context_length")
        .and_then(|v| v.as_u32())
        .map(|v| v as usize)
        .unwrap_or(defaults.max_seq_len);

    let rope_theta = kv.get("llama.rope.freq_base")
        .and_then(|v| v.as_f32())
        .unwrap_or(defaults.rope_theta);

    // Vocab size comes from tokenizer tokens array length if available
    let vocab_size = kv.get("tokenizer.ggml.tokens")
        .and_then(|v| match v {
            GgufValue::Array(arr) => Some(arr.len()),
            _ => None,
        })
        .unwrap_or(defaults.vocab_size);

    ModelConfig {
        vocab_size,
        hidden_dim,
        intermediate_dim: ffn_dim,
        n_layers,
        n_heads,
        n_kv_heads,
        max_seq_len: ctx_len,
        rope_theta,
    }
}

fn extract_vocab(kv: &HashMap<String, GgufValue>) -> Vec<String> {
    kv.get("tokenizer.ggml.tokens")
        .and_then(|v| v.as_string_array())
        .unwrap_or_default()
}

fn extract_merges(kv: &HashMap<String, GgufValue>) -> Vec<(String, String)> {
    kv.get("tokenizer.ggml.merges")
        .and_then(|v| v.as_string_array())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|s| {
            let mut parts = s.splitn(2, ' ');
            let a = parts.next()?.to_string();
            let b = parts.next()?.to_string();
            Some((a, b))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Core GGUF parsing (shared between from_gguf and from_bytes)
// ---------------------------------------------------------------------------

/// Parse a GGUF binary blob into a LoadedModel.
fn parse_gguf(data: &[u8]) -> Result<LoadedModel, String> {
    if data.len() < 24 {
        return Err("file too small to be a valid GGUF".into());
    }

    // Header
    let magic = read_u32(data, 0)?;
    if magic != GGUF_MAGIC {
        return Err(format!(
            "invalid GGUF magic: expected 0x{:08X}, got 0x{:08X}",
            GGUF_MAGIC, magic
        ));
    }

    let version = read_u32(data, 4)?;
    if version < 2 || version > 3 {
        return Err(format!("unsupported GGUF version: {version}"));
    }

    let n_tensors = read_u64(data, 8)? as usize;
    let n_kv = read_u64(data, 16)? as usize;
    let mut offset: usize = 24;

    // Parse KV metadata
    let mut kv_map: HashMap<String, GgufValue> = HashMap::with_capacity(n_kv);
    for i in 0..n_kv {
        let (key, new_off) = read_string(data, offset)
            .map_err(|e| format!("KV entry {i} key: {e}"))?;
        offset = new_off;

        let value_type = read_u32(data, offset)
            .map_err(|e| format!("KV entry {i} type: {e}"))?;
        offset += 4;

        let (value, new_off) = read_gguf_value(data, offset, value_type)
            .map_err(|e| format!("KV entry {i} ('{key}') value: {e}"))?;
        offset = new_off;

        kv_map.insert(key, value);
    }

    // Extract config, vocab, merges
    let config = extract_model_config(&kv_map);
    let vocab = extract_vocab(&kv_map);
    let merges = extract_merges(&kv_map);

    // Get alignment from metadata
    let alignment = kv_map.get("general.alignment")
        .and_then(|v| v.as_u32())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_ALIGNMENT);

    // Parse tensor info entries
    let mut tensor_infos: Vec<TensorInfo> = Vec::with_capacity(n_tensors);
    for i in 0..n_tensors {
        let (name, new_off) = read_string(data, offset)
            .map_err(|e| format!("tensor {i} name: {e}"))?;
        offset = new_off;

        let n_dims = read_u32(data, offset)
            .map_err(|e| format!("tensor {i} n_dims: {e}"))? as usize;
        offset += 4;

        let mut shape = Vec::with_capacity(n_dims);
        let mut n_elements: usize = 1;
        for d in 0..n_dims {
            let dim = read_u64(data, offset)
                .map_err(|e| format!("tensor {i} dim {d}: {e}"))? as usize;
            shape.push(dim);
            n_elements = n_elements.checked_mul(dim)
                .ok_or_else(|| format!("tensor {i} shape overflow"))?;
            offset += 8;
        }

        let quant_type_raw = read_u32(data, offset)
            .map_err(|e| format!("tensor {i} quant type: {e}"))?;
        offset += 4;

        let quant = QuantType::from_u32(quant_type_raw)
            .map_err(|e| format!("tensor {i} ('{name}'): {e}"))?;

        let tensor_offset = read_u64(data, offset)
            .map_err(|e| format!("tensor {i} offset: {e}"))? as usize;
        offset += 8;

        tensor_infos.push(TensorInfo {
            name,
            shape,
            quant,
            offset: tensor_offset,
            n_elements,
        });
    }

    // Data section starts at next aligned boundary after all header data
    let data_start = (offset + alignment - 1) / alignment * alignment;

    // Load tensor data
    let mut tensors: Vec<Tensor> = Vec::with_capacity(n_tensors);
    let mut total_bytes: usize = 0;

    // Determine quantization name from first non-F32 tensor
    let mut quant_name = String::from("F32");

    for ti in &tensor_infos {
        let byte_size = ti.quant.tensor_bytes(ti.n_elements);
        let abs_offset = data_start + ti.offset;
        let abs_end = abs_offset + byte_size;

        if abs_end > data.len() {
            return Err(format!(
                "tensor '{}' data extends past EOF: need {} bytes at offset {}, file is {} bytes",
                ti.name, byte_size, abs_offset, data.len()
            ));
        }

        let tensor_data = data[abs_offset..abs_end].to_vec();
        total_bytes += byte_size;

        // Track dominant quantization type
        match ti.quant {
            QuantType::F32 | QuantType::F16 => {}
            q => {
                quant_name = format!("{:?}", q);
            }
        }

        tensors.push(Tensor {
            name: ti.name.clone(),
            shape: ti.shape.clone(),
            quant: ti.quant,
            data: tensor_data,
        });
    }

    // Model name from metadata
    let model_name = kv_map.get("general.name")
        .and_then(|v| match v {
            GgufValue::Str(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| kv_map.get("general.architecture")
            .and_then(|v| match v {
                GgufValue::Str(s) => Some(s.clone()),
                _ => None,
            }))
        .unwrap_or_else(|| "unknown".to_string());

    Ok(LoadedModel {
        name: model_name,
        quantization: quant_name,
        ram_bytes: total_bytes,
        config,
        tensors,
        vocab,
        merges,
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

impl LoadedModel {
    /// Load a GGUF model from disk into memory.
    ///
    /// Reads the entire file with `std::fs::read()` — no mmap.
    #[cfg(not(feature = "host-test"))]
    pub fn from_gguf(path: &str) -> Result<Self, String> {
        const MAX_MODEL_SIZE: u64 = 2 * 1024 * 1024 * 1024;
        let meta = std::fs::metadata(path)
            .map_err(|e| format!("failed to stat model file '{}': {}", path, e))?;
        if meta.len() > MAX_MODEL_SIZE {
            return Err(format!(
                "model file '{}' is too large ({} bytes, max {} bytes)",
                path, meta.len(), MAX_MODEL_SIZE
            ));
        }
        let data = std::fs::read(path)
            .map_err(|e| format!("failed to read model file '{}': {}", path, e))?;

        parse_gguf(&data)
    }

    /// Parse a GGUF model from an in-memory byte slice.
    /// Useful for testing without filesystem access.
    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        parse_gguf(data)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal synthetic GGUF v3 binary blob for testing.
    fn build_test_gguf(
        n_kv: u64,
        kv_entries: &[(String, u32, Vec<u8>)], // (key, type, raw_value_bytes)
        tensor_infos: &[(&str, &[u64], u32, u64)], // (name, dims, quant_type, offset)
        tensor_data: &[u8],
    ) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();

        // Header
        buf.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
        buf.extend_from_slice(&3u32.to_le_bytes()); // version
        buf.extend_from_slice(&(tensor_infos.len() as u64).to_le_bytes());
        buf.extend_from_slice(&n_kv.to_le_bytes());

        // KV entries
        for (key, vtype, raw) in kv_entries {
            // write key as gguf_string
            buf.extend_from_slice(&(key.len() as u64).to_le_bytes());
            buf.extend_from_slice(key.as_bytes());
            // write value type
            buf.extend_from_slice(&vtype.to_le_bytes());
            // write raw value bytes
            buf.extend_from_slice(raw);
        }

        // Tensor info entries
        for (name, dims, quant_type, tensor_offset) in tensor_infos {
            // name
            buf.extend_from_slice(&(name.len() as u64).to_le_bytes());
            buf.extend_from_slice(name.as_bytes());
            // n_dims
            buf.extend_from_slice(&(dims.len() as u32).to_le_bytes());
            // dims
            for d in *dims {
                buf.extend_from_slice(&d.to_le_bytes());
            }
            // quant type
            buf.extend_from_slice(&quant_type.to_le_bytes());
            // offset
            buf.extend_from_slice(&tensor_offset.to_le_bytes());
        }

        // Align to 32 bytes
        let alignment = DEFAULT_ALIGNMENT;
        let padding = (alignment - (buf.len() % alignment)) % alignment;
        buf.extend(vec![0u8; padding]);

        // Tensor data
        buf.extend_from_slice(tensor_data);

        buf
    }

    #[test]
    fn test_read_u32() {
        let data = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(read_u32(&data, 0).unwrap(), 0x04030201);
    }

    #[test]
    fn test_read_u64() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(read_u64(&data, 0).unwrap(), 0x0807060504030201);
    }

    #[test]
    fn test_read_string() {
        let mut data = Vec::new();
        let s = "hello";
        data.extend_from_slice(&(s.len() as u64).to_le_bytes());
        data.extend_from_slice(s.as_bytes());
        let (result, end) = read_string(&data, 0).unwrap();
        assert_eq!(result, "hello");
        assert_eq!(end, 8 + 5);
    }

    #[test]
    fn test_read_gguf_value_uint32() {
        let data = 42u32.to_le_bytes();
        let (val, off) = read_gguf_value(&data, 0, GGUF_TYPE_UINT32).unwrap();
        assert_eq!(val.as_u32(), Some(42));
        assert_eq!(off, 4);
    }

    #[test]
    fn test_read_gguf_value_float32() {
        let data = 3.14f32.to_le_bytes();
        let (val, off) = read_gguf_value(&data, 0, GGUF_TYPE_FLOAT32).unwrap();
        assert!((val.as_f32().unwrap() - 3.14).abs() < 0.001);
        assert_eq!(off, 4);
    }

    #[test]
    fn test_read_gguf_value_string() {
        let mut data = Vec::new();
        let s = "test";
        data.extend_from_slice(&(s.len() as u64).to_le_bytes());
        data.extend_from_slice(s.as_bytes());
        let (val, _) = read_gguf_value(&data, 0, GGUF_TYPE_STRING).unwrap();
        match val {
            GgufValue::Str(s) => assert_eq!(s, "test"),
            _ => panic!("expected string"),
        }
    }

    #[test]
    fn test_read_gguf_value_bool() {
        let data = [1u8];
        let (val, off) = read_gguf_value(&data, 0, GGUF_TYPE_BOOL).unwrap();
        match val {
            GgufValue::Bool(b) => assert!(b),
            _ => panic!("expected bool"),
        }
        assert_eq!(off, 1);
    }

    #[test]
    fn test_read_gguf_value_array() {
        let mut data = Vec::new();
        // element type: UINT32
        data.extend_from_slice(&GGUF_TYPE_UINT32.to_le_bytes());
        // count: 3
        data.extend_from_slice(&3u64.to_le_bytes());
        // 3 uint32 values
        data.extend_from_slice(&10u32.to_le_bytes());
        data.extend_from_slice(&20u32.to_le_bytes());
        data.extend_from_slice(&30u32.to_le_bytes());

        let (val, _) = read_gguf_value(&data, 0, GGUF_TYPE_ARRAY).unwrap();
        match val {
            GgufValue::Array(arr) => {
                assert_eq!(arr.len(), 3);
                assert_eq!(arr[0].as_u32(), Some(10));
                assert_eq!(arr[1].as_u32(), Some(20));
                assert_eq!(arr[2].as_u32(), Some(30));
            }
            _ => panic!("expected array"),
        }
    }

    #[test]
    fn test_quant_type_from_u32() {
        assert_eq!(QuantType::from_u32(0).unwrap(), QuantType::F32);
        assert_eq!(QuantType::from_u32(1).unwrap(), QuantType::F16);
        assert_eq!(QuantType::from_u32(12).unwrap(), QuantType::Q4_K);
        assert_eq!(QuantType::from_u32(14).unwrap(), QuantType::Q6_K);
        assert!(QuantType::from_u32(99).is_err());
    }

    #[test]
    fn test_parse_minimal_gguf() {
        // Build a GGUF with 1 KV (general.name = "test-model") and 1 F32 tensor
        let mut name_val = Vec::new();
        let name_str = "test-model";
        name_val.extend_from_slice(&(name_str.len() as u64).to_le_bytes());
        name_val.extend_from_slice(name_str.as_bytes());

        let tensor_data = vec![0u8; 16]; // 4 f32 values = 16 bytes

        let gguf = build_test_gguf(
            1,
            &[("general.name".to_string(), GGUF_TYPE_STRING, name_val)],
            &[("weight.0", &[4, 1], 0, 0)], // F32 tensor, 4 elements, offset 0
            &tensor_data,
        );

        let model = LoadedModel::from_bytes(&gguf).unwrap();
        assert_eq!(model.name, "test-model");
        assert_eq!(model.tensors.len(), 1);
        assert_eq!(model.tensors[0].name, "weight.0");
        assert_eq!(model.tensors[0].shape, vec![4, 1]);
        assert_eq!(model.tensors[0].quant, QuantType::F32);
        assert_eq!(model.tensors[0].data.len(), 16);
    }

    #[test]
    fn test_parse_gguf_with_config() {
        // Build KV entries for model config
        let mut kvs: Vec<(String, u32, Vec<u8>)> = Vec::new();

        // llama.embedding_length = 576 (uint32)
        kvs.push(("llama.embedding_length".into(), GGUF_TYPE_UINT32, 576u32.to_le_bytes().to_vec()));
        // llama.block_count = 30
        kvs.push(("llama.block_count".into(), GGUF_TYPE_UINT32, 30u32.to_le_bytes().to_vec()));
        // llama.attention.head_count = 9
        kvs.push(("llama.attention.head_count".into(), GGUF_TYPE_UINT32, 9u32.to_le_bytes().to_vec()));
        // llama.attention.head_count_kv = 3
        kvs.push(("llama.attention.head_count_kv".into(), GGUF_TYPE_UINT32, 3u32.to_le_bytes().to_vec()));
        // llama.feed_forward_length = 1536
        kvs.push(("llama.feed_forward_length".into(), GGUF_TYPE_UINT32, 1536u32.to_le_bytes().to_vec()));
        // llama.context_length = 2048
        kvs.push(("llama.context_length".into(), GGUF_TYPE_UINT32, 2048u32.to_le_bytes().to_vec()));
        // llama.rope.freq_base = 10000.0
        kvs.push(("llama.rope.freq_base".into(), GGUF_TYPE_FLOAT32, 10000.0f32.to_le_bytes().to_vec()));

        let tensor_data = vec![0u8; 4]; // 1 f32

        let gguf = build_test_gguf(
            kvs.len() as u64,
            &kvs,
            &[("test", &[1], 0, 0)],
            &tensor_data,
        );

        let model = LoadedModel::from_bytes(&gguf).unwrap();
        assert_eq!(model.config.hidden_dim, 576);
        assert_eq!(model.config.n_layers, 30);
        assert_eq!(model.config.n_heads, 9);
        assert_eq!(model.config.n_kv_heads, 3);
        assert_eq!(model.config.intermediate_dim, 1536);
        assert_eq!(model.config.max_seq_len, 2048);
        assert!((model.config.rope_theta - 10000.0).abs() < 0.1);
    }

    #[test]
    fn test_invalid_magic() {
        let data = vec![0u8; 24];
        assert!(LoadedModel::from_bytes(&data).is_err());
    }

    #[test]
    fn test_file_too_small() {
        let data = vec![0u8; 10];
        assert!(LoadedModel::from_bytes(&data).is_err());
    }

    #[test]
    fn test_extract_merges() {
        let mut kv = HashMap::new();
        kv.insert("tokenizer.ggml.merges".to_string(), GgufValue::Array(vec![
            GgufValue::Str("hello world".into()),
            GgufValue::Str("foo bar".into()),
        ]));
        let merges = extract_merges(&kv);
        assert_eq!(merges.len(), 2);
        assert_eq!(merges[0], ("hello".into(), "world".into()));
        assert_eq!(merges[1], ("foo".into(), "bar".into()));
    }
}
