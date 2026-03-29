//! Text generation via autoregressive transformer inference.
//!
//! Implements the token-by-token generation loop with timing,
//! greedy decoding, and optional temperature sampling.
//! Includes Q4_0 and Q4_K_M dequantization for quantized weight inference.
//! Implements a full LLaMA-style transformer forward pass with:
//!   - RMSNorm, RoPE, grouped-query attention, SwiGLU FFN
//!   - KV cache for efficient autoregressive generation

#[cfg(not(feature = "host-test"))]
use crate::model::LoadedModel;
#[cfg(not(feature = "host-test"))]
use crate::tokenizer::Tokenizer;
#[cfg(not(feature = "host-test"))]
use crate::GenerateResult;
#[cfg(not(feature = "host-test"))]
use crate::model::Tensor;

use crate::model::QuantType;

/// Sampling strategy for token selection.
#[derive(Debug, Clone, Copy)]
pub enum SamplingStrategy {
    /// Always pick the highest-probability token.
    Greedy,
    /// Sample with temperature scaling (1.0 = neutral, <1.0 = sharper, >1.0 = flatter).
    Temperature(f32),
}

impl Default for SamplingStrategy {
    fn default() -> Self {
        Self::Greedy
    }
}

// ---------------------------------------------------------------------------
// IEEE 754 half-float conversion
// ---------------------------------------------------------------------------

/// Convert an IEEE 754 half-precision float (16-bit) to single-precision (32-bit).
#[inline]
pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = ((bits >> 15) & 1) as u32;
    let exp = ((bits >> 10) & 0x1F) as u32;
    let mantissa = (bits & 0x3FF) as u32;

    if exp == 0 {
        if mantissa == 0 {
            f32::from_bits(sign << 31)
        } else {
            let val = (mantissa as f32) * (1.0 / 1024.0) * (1.0 / 16384.0);
            if sign == 1 { -val } else { val }
        }
    } else if exp == 31 {
        if mantissa == 0 {
            f32::from_bits((sign << 31) | 0x7F800000)
        } else {
            f32::from_bits((sign << 31) | 0x7F800000 | (mantissa << 13))
        }
    } else {
        let f32_exp = exp + 112;
        f32::from_bits((sign << 31) | (f32_exp << 23) | (mantissa << 13))
    }
}

/// Read a half-float from a byte slice at offset (little-endian).
#[inline]
fn read_f16(data: &[u8], offset: usize) -> f32 {
    let bits = u16::from_le_bytes([data[offset], data[offset + 1]]);
    f16_to_f32(bits)
}

// ---------------------------------------------------------------------------
// Q4_0 dequantization
// ---------------------------------------------------------------------------

const Q4_0_BLOCK_SIZE: usize = 32;
const Q4_0_BYTES_PER_BLOCK: usize = 18;

#[inline]
pub fn dequant_q4_0_block(block: &[u8], out: &mut [f32]) {
    let scale = read_f16(block, 0);
    for j in 0..16 {
        let byte = block[2 + j];
        let lo = (byte & 0x0F) as i32 - 8;
        let hi = (byte >> 4) as i32 - 8;
        out[j * 2] = lo as f32 * scale;
        out[j * 2 + 1] = hi as f32 * scale;
    }
}

pub fn dequant_q4_0(data: &[u8], n_elements: usize) -> Vec<f32> {
    let n_blocks = (n_elements + Q4_0_BLOCK_SIZE - 1) / Q4_0_BLOCK_SIZE;
    let mut out = vec![0.0f32; n_blocks * Q4_0_BLOCK_SIZE];
    for b in 0..n_blocks {
        let block_off = b * Q4_0_BYTES_PER_BLOCK;
        let out_off = b * Q4_0_BLOCK_SIZE;
        dequant_q4_0_block(
            &data[block_off..block_off + Q4_0_BYTES_PER_BLOCK],
            &mut out[out_off..out_off + Q4_0_BLOCK_SIZE],
        );
    }
    out.truncate(n_elements);
    out
}

// ---------------------------------------------------------------------------
// Q4_K_M dequantization
// ---------------------------------------------------------------------------

const Q4_K_BLOCK_SIZE: usize = 256;
const Q4_K_BYTES_PER_BLOCK: usize = 144;

#[inline]
fn unpack_q4k_scales(scales_raw: &[u8]) -> ([u8; 8], [u8; 8]) {
    let mut sc = [0u8; 8];
    let mut mn = [0u8; 8];

    for i in 0..4 {
        sc[i] = scales_raw[i] & 0x3F;
        mn[i] = scales_raw[i + 4] & 0x3F;
    }
    for i in 0..4 {
        let upper_sc = (scales_raw[i] >> 6) & 0x03;
        let upper_mn = (scales_raw[i + 4] >> 6) & 0x03;
        sc[i + 4] = (scales_raw[8 + i] & 0x0F) | (upper_sc << 4);
        mn[i + 4] = (scales_raw[8 + i] >> 4) | (upper_mn << 4);
    }

    (sc, mn)
}

#[inline]
pub fn dequant_q4k_block(block: &[u8], out: &mut [f32]) {
    let d = read_f16(block, 0);
    let dmin = read_f16(block, 2);
    let (sc, mn) = unpack_q4k_scales(&block[4..16]);
    let qs = &block[16..144];

    for j in 0..8u32 {
        let scale = d * sc[j as usize] as f32;
        let min = dmin * mn[j as usize] as f32;
        let qs_off = (j as usize) * 16;

        for i in 0..16 {
            let byte = qs[qs_off + i];
            let lo = (byte & 0x0F) as f32;
            let hi = (byte >> 4) as f32;
            out[(j as usize) * 32 + i * 2] = scale * lo - min;
            out[(j as usize) * 32 + i * 2 + 1] = scale * hi - min;
        }
    }
}

pub fn dequant_q4k(data: &[u8], n_elements: usize) -> Vec<f32> {
    let n_blocks = (n_elements + Q4_K_BLOCK_SIZE - 1) / Q4_K_BLOCK_SIZE;
    let mut out = vec![0.0f32; n_blocks * Q4_K_BLOCK_SIZE];
    for b in 0..n_blocks {
        let block_off = b * Q4_K_BYTES_PER_BLOCK;
        let out_off = b * Q4_K_BLOCK_SIZE;
        dequant_q4k_block(
            &data[block_off..block_off + Q4_K_BYTES_PER_BLOCK],
            &mut out[out_off..out_off + Q4_K_BLOCK_SIZE],
        );
    }
    out.truncate(n_elements);
    out
}

// ---------------------------------------------------------------------------
// Q8_0 dequantization
// ---------------------------------------------------------------------------

const Q8_0_BLOCK_SIZE: usize = 32;
const Q8_0_BYTES_PER_BLOCK: usize = 34; // 2 (f16 scale) + 32 (int8 values)

pub fn dequant_q8_0(data: &[u8], n_elements: usize) -> Vec<f32> {
    let n_blocks = (n_elements + Q8_0_BLOCK_SIZE - 1) / Q8_0_BLOCK_SIZE;
    let mut out = vec![0.0f32; n_blocks * Q8_0_BLOCK_SIZE];
    for b in 0..n_blocks {
        let off = b * Q8_0_BYTES_PER_BLOCK;
        let scale = f16_to_f32(u16::from_le_bytes([data[off], data[off + 1]]));
        let out_off = b * Q8_0_BLOCK_SIZE;
        for i in 0..Q8_0_BLOCK_SIZE {
            let val = data[off + 2 + i] as i8;
            out[out_off + i] = scale * val as f32;
        }
    }
    out.truncate(n_elements);
    out
}

#[inline]
fn dot_q8_0(row: &[u8], x: &[f32], n: usize) -> f32 {
    let n_blocks = (n + Q8_0_BLOCK_SIZE - 1) / Q8_0_BLOCK_SIZE;
    let mut sum = 0.0f32;
    for b in 0..n_blocks {
        let off = b * Q8_0_BYTES_PER_BLOCK;
        let scale = f16_to_f32(u16::from_le_bytes([row[off], row[off + 1]]));
        let x_off = b * Q8_0_BLOCK_SIZE;
        let mut block_sum = 0.0f32;
        for i in 0..Q8_0_BLOCK_SIZE {
            let idx = x_off + i;
            if idx >= n { break; }
            let val = row[off + 2 + i] as i8;
            block_sum += val as f32 * x[idx];
        }
        sum += scale * block_sum;
    }
    sum
}

// ---------------------------------------------------------------------------
// Q5_0 dequantization
// ---------------------------------------------------------------------------

const Q5_0_BLOCK_SIZE: usize = 32;
const Q5_0_BYTES_PER_BLOCK: usize = 22; // 2 (f16 scale) + 4 (qh) + 16 (qs)

pub fn dequant_q5_0(data: &[u8], n_elements: usize) -> Vec<f32> {
    let n_blocks = (n_elements + Q5_0_BLOCK_SIZE - 1) / Q5_0_BLOCK_SIZE;
    let mut out = vec![0.0f32; n_blocks * Q5_0_BLOCK_SIZE];
    for b in 0..n_blocks {
        let off = b * Q5_0_BYTES_PER_BLOCK;
        let scale = f16_to_f32(u16::from_le_bytes([data[off], data[off + 1]]));
        let qh = u32::from_le_bytes([data[off + 2], data[off + 3], data[off + 4], data[off + 5]]);
        let out_off = b * Q5_0_BLOCK_SIZE;
        for i in 0..Q5_0_BLOCK_SIZE {
            let byte_idx = i / 2;
            let nibble = if i % 2 == 0 {
                data[off + 6 + byte_idx] & 0x0F
            } else {
                data[off + 6 + byte_idx] >> 4
            };
            let high_bit = ((qh >> i) & 1) as u8;
            let val = (nibble | (high_bit << 4)) as i32 - 16;
            out[out_off + i] = scale * val as f32;
        }
    }
    out.truncate(n_elements);
    out
}

#[inline]
fn dot_q5_0(row: &[u8], x: &[f32], n: usize) -> f32 {
    let n_blocks = (n + Q5_0_BLOCK_SIZE - 1) / Q5_0_BLOCK_SIZE;
    let mut sum = 0.0f32;
    for b in 0..n_blocks {
        let off = b * Q5_0_BYTES_PER_BLOCK;
        let scale = f16_to_f32(u16::from_le_bytes([row[off], row[off + 1]]));
        let qh = u32::from_le_bytes([row[off + 2], row[off + 3], row[off + 4], row[off + 5]]);
        let x_off = b * Q5_0_BLOCK_SIZE;
        let mut block_sum = 0.0f32;
        for i in 0..Q5_0_BLOCK_SIZE {
            let idx = x_off + i;
            if idx >= n { break; }
            let byte_idx = i / 2;
            let nibble = if i % 2 == 0 {
                row[off + 6 + byte_idx] & 0x0F
            } else {
                row[off + 6 + byte_idx] >> 4
            };
            let high_bit = ((qh >> i) & 1) as u8;
            let val = (nibble | (high_bit << 4)) as i32 - 16;
            block_sum += val as f32 * x[idx];
        }
        sum += scale * block_sum;
    }
    sum
}

// ---------------------------------------------------------------------------
// Q6_K dequantization
// ---------------------------------------------------------------------------

const Q6_K_BLOCK_SIZE: usize = 256;
const Q6_K_BYTES_PER_BLOCK: usize = 210; // 128 (ql) + 64 (qh) + 16 (scales) + 2 (d)

pub fn dequant_q6k(data: &[u8], n_elements: usize) -> Vec<f32> {
    let n_blocks = (n_elements + Q6_K_BLOCK_SIZE - 1) / Q6_K_BLOCK_SIZE;
    let mut out = vec![0.0f32; n_blocks * Q6_K_BLOCK_SIZE];
    for b in 0..n_blocks {
        let off = b * Q6_K_BYTES_PER_BLOCK;
        let ql = &data[off..off + 128];
        let qh = &data[off + 128..off + 192];
        let scales = &data[off + 192..off + 208];
        let d = f16_to_f32(u16::from_le_bytes([data[off + 208], data[off + 209]]));
        let out_off = b * Q6_K_BLOCK_SIZE;
        for i in 0..Q6_K_BLOCK_SIZE {
            let ql_byte = ql[i / 2];
            let ql_val = if i % 2 == 0 { ql_byte & 0xF } else { ql_byte >> 4 };
            let qh_byte = qh[i / 4];
            let qh_val = (qh_byte >> ((i % 4) * 2)) & 3;
            let q = (ql_val | (qh_val << 4)) as i32 - 32;
            let sc = scales[i / 16] as i8 as f32;
            out[out_off + i] = d * sc * q as f32;
        }
    }
    out.truncate(n_elements);
    out
}

// ---------------------------------------------------------------------------
// Q5_1 dequantization
// ---------------------------------------------------------------------------

const Q5_1_BLOCK_SIZE: usize = 32;
const Q5_1_BYTES_PER_BLOCK: usize = 24; // 2 (f16 scale) + 2 (f16 min) + 4 (qh) + 16 (qs)

pub fn dequant_q5_1(data: &[u8], n_elements: usize) -> Vec<f32> {
    let n_blocks = (n_elements + Q5_1_BLOCK_SIZE - 1) / Q5_1_BLOCK_SIZE;
    let mut out = vec![0.0f32; n_blocks * Q5_1_BLOCK_SIZE];
    for b in 0..n_blocks {
        let off = b * Q5_1_BYTES_PER_BLOCK;
        let d = f16_to_f32(u16::from_le_bytes([data[off], data[off + 1]]));
        let m = f16_to_f32(u16::from_le_bytes([data[off + 2], data[off + 3]]));
        let qh = u32::from_le_bytes([data[off + 4], data[off + 5], data[off + 6], data[off + 7]]);
        let out_off = b * Q5_1_BLOCK_SIZE;
        for i in 0..Q5_1_BLOCK_SIZE {
            let byte_idx = i / 2;
            let nibble = if i % 2 == 0 {
                data[off + 8 + byte_idx] & 0x0F
            } else {
                data[off + 8 + byte_idx] >> 4
            };
            let high_bit = ((qh >> i) & 1) as u8;
            let val = (nibble | (high_bit << 4)) as f32;
            out[out_off + i] = d * val + m;
        }
    }
    out.truncate(n_elements);
    out
}

// ---------------------------------------------------------------------------
// Quantized matrix-vector multiply
// ---------------------------------------------------------------------------

/// Matrix-vector multiply with on-the-fly dequantization.
/// Computes out[i] = dot(weight_row[i], x) for each row i.
pub fn matmul_q(out: &mut [f32], weight: &[u8], x: &[f32], cols: usize, quant: QuantType) {
    let rows = out.len();
    match quant {
        QuantType::F32 => {
            matmul_f32(out, weight, x, cols);
        }
        QuantType::Q4_0 => {
            let row_bytes = Q4_0_BYTES_PER_BLOCK * ((cols + Q4_0_BLOCK_SIZE - 1) / Q4_0_BLOCK_SIZE);
            for i in 0..rows {
                let row_off = i * row_bytes;
                out[i] = dot_q4_0(&weight[row_off..row_off + row_bytes], x, cols);
            }
        }
        QuantType::Q4_K => {
            let row_bytes = Q4_K_BYTES_PER_BLOCK * ((cols + Q4_K_BLOCK_SIZE - 1) / Q4_K_BLOCK_SIZE);
            for i in 0..rows {
                let row_off = i * row_bytes;
                out[i] = dot_q4k(&weight[row_off..row_off + row_bytes], x, cols);
            }
        }
        QuantType::Q8_0 => {
            let row_bytes = Q8_0_BYTES_PER_BLOCK * ((cols + Q8_0_BLOCK_SIZE - 1) / Q8_0_BLOCK_SIZE);
            for i in 0..rows {
                let row_off = i * row_bytes;
                out[i] = dot_q8_0(&weight[row_off..row_off + row_bytes], x, cols);
            }
        }
        QuantType::Q5_0 => {
            let row_bytes = Q5_0_BYTES_PER_BLOCK * ((cols + Q5_0_BLOCK_SIZE - 1) / Q5_0_BLOCK_SIZE);
            for i in 0..rows {
                let row_off = i * row_bytes;
                out[i] = dot_q5_0(&weight[row_off..row_off + row_bytes], x, cols);
            }
        }
        _ => {
            let row_bytes = quant.tensor_bytes(cols);
            for i in 0..rows {
                let row_off = i * row_bytes;
                let dequantized = dequant_generic(&weight[row_off..row_off + row_bytes], cols, quant);
                out[i] = dot_f32(&dequantized, x);
            }
        }
    }
}

fn matmul_f32(out: &mut [f32], weight: &[u8], x: &[f32], cols: usize) {
    let rows = out.len();
    let row_bytes = cols * 4;
    for i in 0..rows {
        let off = i * row_bytes;
        let mut sum = 0.0f32;
        for j in 0..cols {
            let w = f32::from_le_bytes([
                weight[off + j * 4],
                weight[off + j * 4 + 1],
                weight[off + j * 4 + 2],
                weight[off + j * 4 + 3],
            ]);
            sum += w * x[j];
        }
        out[i] = sum;
    }
}

fn dot_q4_0(row: &[u8], x: &[f32], n: usize) -> f32 {
    let n_blocks = (n + Q4_0_BLOCK_SIZE - 1) / Q4_0_BLOCK_SIZE;
    let mut sum = 0.0f32;
    for b in 0..n_blocks {
        let block = &row[b * Q4_0_BYTES_PER_BLOCK..];
        let scale = read_f16(block, 0);
        let x_off = b * Q4_0_BLOCK_SIZE;
        for j in 0..16 {
            let idx = x_off + j * 2;
            if idx + 1 >= n { break; }
            let byte = block[2 + j];
            let lo = (byte & 0x0F) as i32 - 8;
            let hi = (byte >> 4) as i32 - 8;
            sum += (lo as f32 * scale) * x[idx];
            sum += (hi as f32 * scale) * x[idx + 1];
        }
    }
    sum
}

fn dot_q4k(row: &[u8], x: &[f32], n: usize) -> f32 {
    let n_blocks = (n + Q4_K_BLOCK_SIZE - 1) / Q4_K_BLOCK_SIZE;
    let mut sum = 0.0f32;
    for b in 0..n_blocks {
        let block = &row[b * Q4_K_BYTES_PER_BLOCK..];
        let d = read_f16(block, 0);
        let dmin = read_f16(block, 2);
        let (sc, mn) = unpack_q4k_scales(&block[4..16]);
        let qs = &block[16..144];
        let x_off = b * Q4_K_BLOCK_SIZE;

        for j in 0..8usize {
            let scale = d * sc[j] as f32;
            let min = dmin * mn[j] as f32;
            let qs_off = j * 16;

            for i in 0..16 {
                let idx = x_off + j * 32 + i * 2;
                if idx + 1 >= n { break; }
                let byte = qs[qs_off + i];
                let lo = (byte & 0x0F) as f32;
                let hi = (byte >> 4) as f32;
                sum += (scale * lo - min) * x[idx];
                sum += (scale * hi - min) * x[idx + 1];
            }
        }
    }
    sum
}

fn dequant_generic(data: &[u8], n_elements: usize, quant: QuantType) -> Vec<f32> {
    match quant {
        QuantType::F16 => {
            let mut out = Vec::with_capacity(n_elements);
            for i in 0..n_elements {
                out.push(read_f16(data, i * 2));
            }
            out
        }
        QuantType::Q4_0 => dequant_q4_0(data, n_elements),
        QuantType::Q4_K => dequant_q4k(data, n_elements),
        QuantType::Q8_0 => dequant_q8_0(data, n_elements),
        QuantType::Q5_0 => dequant_q5_0(data, n_elements),
        QuantType::Q5_1 => dequant_q5_1(data, n_elements),
        QuantType::Q6_K => dequant_q6k(data, n_elements),
        QuantType::F32 => {
            let mut out = Vec::with_capacity(n_elements);
            for i in 0..n_elements {
                let off = i * 4;
                out.push(f32::from_le_bytes([
                    data[off], data[off + 1], data[off + 2], data[off + 3],
                ]));
            }
            out
        }
        _ => {
            panic!("dequant_generic: unsupported quant type {:?} for {} elements", quant, n_elements);
        }
    }
}

#[inline]
fn dot_f32(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

// ---------------------------------------------------------------------------
// Dequantize a full tensor to f32
// ---------------------------------------------------------------------------

/// Dequantize any tensor's raw bytes into f32 values.
#[cfg(not(feature = "host-test"))]
pub fn dequant_tensor(tensor: &Tensor) -> Vec<f32> {
    let n: usize = tensor.shape.iter().product();
    dequant_generic(&tensor.data, n, tensor.quant)
}

// ---------------------------------------------------------------------------
// Transformer forward pass helpers
// ---------------------------------------------------------------------------

/// Apply RMSNorm: x * rsqrt(mean(x^2) + eps) * weight
#[inline]
pub fn rmsnorm(out: &mut [f32], x: &[f32], weight: &[f32], eps: f32) {
    if x.is_empty() { return; }
    let ss: f32 = x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32;
    let scale = 1.0 / (ss + eps).sqrt();
    for i in 0..x.len() {
        out[i] = x[i] * scale * weight[i];
    }
}

/// RMSNorm in-place: x = rmsnorm(x, weight)
#[inline]
#[cfg(not(feature = "host-test"))]
fn rmsnorm_inplace(x: &mut [f32], weight: &[f32], eps: f32) {
    if x.is_empty() { return; }
    let ss: f32 = x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32;
    let scale = 1.0 / (ss + eps).sqrt();
    for i in 0..x.len() {
        x[i] = x[i] * scale * weight[i];
    }
}

/// Softmax in-place over a slice of logits.
#[inline]
pub fn softmax(logits: &mut [f32]) {
    if logits.is_empty() { return; }
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let mut sum = 0.0f32;
    for v in logits.iter_mut() {
        *v = (*v - max).exp();
        sum += *v;
    }
    for v in logits.iter_mut() {
        *v /= sum;
    }
}

/// Argmax over a slice — returns the index of the largest element.
#[inline]
pub fn argmax(data: &[f32]) -> usize {
    let mut best_i = 0;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &v) in data.iter().enumerate() {
        if v > best_v {
            best_v = v;
            best_i = i;
        }
    }
    best_i
}

/// Apply RoPE (Rotary Position Embedding) to Q and K vectors.
#[inline]
pub fn apply_rope(q: &mut [f32], k: &mut [f32], pos: usize, head_dim: usize, theta: f32) {
    assert!(head_dim % 2 == 0, "head_dim must be even for RoPE");
    for i in (0..head_dim).step_by(2) {
        let freq = 1.0 / theta.powf(i as f32 / head_dim as f32);
        let angle = pos as f32 * freq;
        let (sin_v, cos_v) = angle.sin_cos();

        let q0 = q[i];
        let q1 = q[i + 1];
        q[i] = q0 * cos_v - q1 * sin_v;
        q[i + 1] = q0 * sin_v + q1 * cos_v;

        let k0 = k[i];
        let k1 = k[i + 1];
        k[i] = k0 * cos_v - k1 * sin_v;
        k[i + 1] = k0 * sin_v + k1 * cos_v;
    }
}

/// Apply RoPE to a single head (Q or K independently).
fn apply_rope_q(v: &mut [f32], pos: usize, head_dim: usize, theta: f32) {
    for i in (0..head_dim).step_by(2) {
        let freq = 1.0 / theta.powf(i as f32 / head_dim as f32);
        let angle = pos as f32 * freq;
        let (sin_v, cos_v) = angle.sin_cos();
        let v0 = v[i];
        let v1 = v[i + 1];
        v[i] = v0 * cos_v - v1 * sin_v;
        v[i + 1] = v0 * sin_v + v1 * cos_v;
    }
}

/// SiLU (Swish) activation: x * sigmoid(x)
#[inline]
#[allow(dead_code)]
fn silu(x: f32) -> f32 {
    x / (1.0 + (-x).exp())
}

// ---------------------------------------------------------------------------
// Tensor lookup helper
// ---------------------------------------------------------------------------

/// Find a tensor by name in the model's tensor list.
#[cfg(not(feature = "host-test"))]
/// Sample a token using temperature scaling and top-k filtering.
/// Uses a simple LCG PRNG (no external deps needed).
#[cfg(not(feature = "host-test"))]
fn sample_top_k(logits: &mut [f32], temperature: f32, k: usize) -> u32 {
    if temperature <= 0.0 {
        return argmax(logits) as u32;
    }

    // Apply temperature
    for v in logits.iter_mut() {
        *v /= temperature;
    }

    // Top-k: find k largest logits
    let mut indexed: Vec<(usize, f32)> = logits.iter().enumerate().map(|(i, &v)| (i, v)).collect();
    indexed.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    indexed.truncate(k);

    // Softmax over top-k
    let max_val = indexed[0].1;
    let mut probs: Vec<f32> = indexed.iter().map(|(_, v)| (v - max_val).exp()).collect();
    let sum: f32 = probs.iter().sum();
    for p in probs.iter_mut() {
        *p /= sum;
    }

    // Sample from distribution using simple PRNG
    // Use system time as seed (no rand crate needed)
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let r = (seed as f32) / (u32::MAX as f32);

    let mut cumsum = 0.0f32;
    for (i, &p) in probs.iter().enumerate() {
        cumsum += p;
        if r < cumsum {
            return indexed[i].0 as u32;
        }
    }
    indexed[0].0 as u32
}

fn find_tensor<'a>(tensors: &'a [Tensor], name: &str) -> Result<&'a Tensor, String> {
    tensors.iter()
        .find(|t| t.name == name)
        .ok_or_else(|| format!("tensor '{}' not found in model", name))
}

// ---------------------------------------------------------------------------
// KV Cache
// ---------------------------------------------------------------------------

/// KV cache for autoregressive generation.
/// Stores key and value vectors for each layer at each position.
#[cfg(not(feature = "host-test"))]
pub struct KvCache {
    key_cache: Vec<Vec<f32>>,
    val_cache: Vec<Vec<f32>>,
    n_kv_heads: usize,
    head_dim: usize,
}

#[cfg(not(feature = "host-test"))]
impl KvCache {
    pub fn new(n_layers: usize, n_kv_heads: usize, head_dim: usize, max_seq_len: usize) -> Self {
        let layer_size = max_seq_len * n_kv_heads * head_dim;
        Self {
            key_cache: vec![vec![0.0f32; layer_size]; n_layers],
            val_cache: vec![vec![0.0f32; layer_size]; n_layers],
            n_kv_heads,
            head_dim,
        }
    }

    /// Store a key vector for a given layer, position, and kv_head.
    fn store_key(&mut self, layer: usize, pos: usize, kv_head: usize, data: &[f32]) {
        let off = (pos * self.n_kv_heads + kv_head) * self.head_dim;
        self.key_cache[layer][off..off + self.head_dim].copy_from_slice(data);
    }

    /// Store a value vector for a given layer, position, and kv_head.
    fn store_val(&mut self, layer: usize, pos: usize, kv_head: usize, data: &[f32]) {
        let off = (pos * self.n_kv_heads + kv_head) * self.head_dim;
        self.val_cache[layer][off..off + self.head_dim].copy_from_slice(data);
    }

    /// Get key vector for a given layer, position, and kv_head.
    fn get_key(&self, layer: usize, pos: usize, kv_head: usize) -> &[f32] {
        let off = (pos * self.n_kv_heads + kv_head) * self.head_dim;
        &self.key_cache[layer][off..off + self.head_dim]
    }

    /// Get value vector for a given layer, position, and kv_head.
    fn get_val(&self, layer: usize, pos: usize, kv_head: usize) -> &[f32] {
        let off = (pos * self.n_kv_heads + kv_head) * self.head_dim;
        &self.val_cache[layer][off..off + self.head_dim]
    }
}

// ---------------------------------------------------------------------------
// Transformer forward pass (single token)
// ---------------------------------------------------------------------------

/// Run the transformer forward pass for a single token at a given position.
/// Returns logits over the vocabulary.
#[cfg(not(feature = "host-test"))]
pub fn transformer_forward(
    model: &LoadedModel,
    token: u32,
    pos: usize,
    kv_cache: &mut KvCache,
) -> Result<Vec<f32>, String> {
    let cfg = &model.config;
    let dim = cfg.hidden_dim;
    let n_layers = cfg.n_layers;
    let n_heads = cfg.n_heads;
    let n_kv_heads = cfg.n_kv_heads;
    let head_dim = dim / n_heads;
    let kv_dim = n_kv_heads * head_dim;
    let heads_per_kv = n_heads / n_kv_heads;
    let ffn_dim = cfg.intermediate_dim;
    let eps = 1e-5f32;

    // 1. Token embedding lookup
    let emb_tensor = find_tensor(&model.tensors, "token_embd.weight")?;
    let emb_data = dequant_tensor(emb_tensor);
    let emb_off = (token as usize) * dim;
    let mut x = emb_data[emb_off..emb_off + dim].to_vec();

    // Scratch buffers
    let mut xb = vec![0.0f32; dim];       // after rmsnorm
    let mut q_buf = vec![0.0f32; dim];     // query: n_heads * head_dim
    let mut k_buf = vec![0.0f32; kv_dim];  // key: n_kv_heads * head_dim
    let mut v_buf = vec![0.0f32; kv_dim];  // value: n_kv_heads * head_dim
    let mut attn_out = vec![0.0f32; dim];
    let mut xb2 = vec![0.0f32; dim];
    let mut hb = vec![0.0f32; ffn_dim];    // ffn gate output
    let mut hb2 = vec![0.0f32; ffn_dim];   // ffn up output

    // 2. Transformer layers
    for layer in 0..n_layers {
        // 2a. RMSNorm (pre-attention)
        let attn_norm = find_tensor(&model.tensors, &format!("blk.{layer}.attn_norm.weight"))?;
        let attn_norm_w = dequant_tensor(attn_norm);
        rmsnorm(&mut xb, &x, &attn_norm_w, eps);

        // 2b. QKV projections
        let wq = find_tensor(&model.tensors, &format!("blk.{layer}.attn_q.weight"))?;
        matmul_q(&mut q_buf, &wq.data, &xb, dim, wq.quant);

        let wk = find_tensor(&model.tensors, &format!("blk.{layer}.attn_k.weight"))?;
        matmul_q(&mut k_buf, &wk.data, &xb, dim, wk.quant);

        let wv = find_tensor(&model.tensors, &format!("blk.{layer}.attn_v.weight"))?;
        matmul_q(&mut v_buf, &wv.data, &xb, dim, wv.quant);

        // 2c. Apply RoPE to Q heads and K heads separately
        // Q: apply to all n_heads
        for h in 0..n_heads {
            let q_off = h * head_dim;
            apply_rope_q(&mut q_buf[q_off..q_off + head_dim], pos, head_dim, cfg.rope_theta);
        }
        // K: apply once per KV head (NOT per query head — would corrupt via repeated rotation)
        for kv_h in 0..n_kv_heads {
            let k_off = kv_h * head_dim;
            apply_rope_q(&mut k_buf[k_off..k_off + head_dim], pos, head_dim, cfg.rope_theta);
        }

        // 2d. Store K, V in cache
        for kv_h in 0..n_kv_heads {
            let k_off = kv_h * head_dim;
            let v_off = kv_h * head_dim;
            kv_cache.store_key(layer, pos, kv_h, &k_buf[k_off..k_off + head_dim]);
            kv_cache.store_val(layer, pos, kv_h, &v_buf[v_off..v_off + head_dim]);
        }

        // 2e. Grouped-query attention
        let scale = 1.0 / (head_dim as f32).sqrt();
        attn_out.fill(0.0);

        for h in 0..n_heads {
            let kv_h = h / heads_per_kv;
            let q_off = h * head_dim;
            let q_head = &q_buf[q_off..q_off + head_dim];

            // Compute attention scores for all positions up to and including current
            let seq_len = pos + 1;
            let mut attn_scores = vec![0.0f32; seq_len];
            for t in 0..seq_len {
                let k_cached = kv_cache.get_key(layer, t, kv_h);
                let mut score = 0.0f32;
                for d in 0..head_dim {
                    score += q_head[d] * k_cached[d];
                }
                attn_scores[t] = score * scale;
            }

            // Softmax over attention scores
            softmax(&mut attn_scores);

            // Weighted sum of values
            let out_off = h * head_dim;
            for t in 0..seq_len {
                let v_cached = kv_cache.get_val(layer, t, kv_h);
                let w = attn_scores[t];
                for d in 0..head_dim {
                    attn_out[out_off + d] += w * v_cached[d];
                }
            }
        }

        // 2f. Output projection
        let wo = find_tensor(&model.tensors, &format!("blk.{layer}.attn_output.weight"))?;
        matmul_q(&mut xb2, &wo.data, &attn_out, dim, wo.quant);

        // 2g. Residual connection
        for i in 0..dim {
            x[i] += xb2[i];
        }

        // 2h. RMSNorm (pre-FFN)
        let ffn_norm = find_tensor(&model.tensors, &format!("blk.{layer}.ffn_norm.weight"))?;
        let ffn_norm_w = dequant_tensor(ffn_norm);
        rmsnorm(&mut xb, &x, &ffn_norm_w, eps);

        // 2i. SwiGLU FFN
        // gate = silu(gate_proj(xb))
        let w_gate = find_tensor(&model.tensors, &format!("blk.{layer}.ffn_gate.weight"))?;
        matmul_q(&mut hb, &w_gate.data, &xb, dim, w_gate.quant);

        // up = up_proj(xb)
        let w_up = find_tensor(&model.tensors, &format!("blk.{layer}.ffn_up.weight"))?;
        matmul_q(&mut hb2, &w_up.data, &xb, dim, w_up.quant);

        // hidden = silu(gate) * up
        for i in 0..ffn_dim {
            hb[i] = silu(hb[i]) * hb2[i];
        }

        // down_proj
        let w_down = find_tensor(&model.tensors, &format!("blk.{layer}.ffn_down.weight"))?;
        matmul_q(&mut xb2, &w_down.data, &hb, ffn_dim, w_down.quant);

        // 2j. Residual connection
        for i in 0..dim {
            x[i] += xb2[i];
        }
    }

    // 3. Final RMSNorm
    let out_norm = find_tensor(&model.tensors, "output_norm.weight")?;
    let out_norm_w = dequant_tensor(out_norm);
    rmsnorm_inplace(&mut x, &out_norm_w, eps);

    // 4. LM head projection → logits
    let lm_head = find_tensor(&model.tensors, "output.weight")
        .or_else(|_| find_tensor(&model.tensors, "token_embd.weight"))?; // weight tying
    let mut logits = vec![0.0f32; cfg.vocab_size];
    matmul_q(&mut logits, &lm_head.data, &x, dim, lm_head.quant);

    Ok(logits)
}

// ---------------------------------------------------------------------------
// Generation loop
// ---------------------------------------------------------------------------

/// Generate tokens autoregressively from a prompt.
#[cfg(not(feature = "host-test"))]
pub fn generate_tokens(
    model: &LoadedModel,
    tokenizer: &Tokenizer,
    input_ids: &[u32],
    max_tokens: usize,
) -> Result<GenerateResult, String> {
    let start = std::time::Instant::now();
    let cfg = &model.config;
    let head_dim = cfg.hidden_dim / cfg.n_heads;

    // Initialize KV cache
    let total_len = input_ids.len() + max_tokens;
    let cache_len = total_len.min(cfg.max_seq_len);
    let mut kv_cache = KvCache::new(cfg.n_layers, cfg.n_kv_heads, head_dim, cache_len);

    let mut generated_tokens: Vec<u32> = Vec::with_capacity(max_tokens);
    let temperature: f32 = 0.7; // Balanced: some diversity without chaos

    // Process prompt tokens (prefill)
    let mut next_token = input_ids[0];
    for pos in 0..input_ids.len() {
        let token = input_ids[pos];
        let mut logits = transformer_forward(model, token, pos, &mut kv_cache)?;

        if pos == input_ids.len() - 1 {
            // Last prompt token: sample next
            next_token = sample_top_k(&mut logits, temperature, 40);
        }
    }

    // Autoregressive generation
    for step in 0..max_tokens {
        let pos = input_ids.len() + step;
        if pos >= cfg.max_seq_len {
            break;
        }

        if step > 0 {
            let mut logits = transformer_forward(model, next_token, pos, &mut kv_cache)?;
            next_token = sample_top_k(&mut logits, temperature, 40);
        }

        generated_tokens.push(next_token);

        // Stop on EOS or <|im_end|> or <|endoftext|>
        if next_token == tokenizer.eos_id || next_token == 0 {
            break;
        }
    }

    let elapsed = start.elapsed().as_secs_f64();
    let tokens_generated = generated_tokens.len();
    let tokens_per_sec = if elapsed > 0.0 {
        tokens_generated as f64 / elapsed
    } else {
        0.0
    };

    let text = tokenizer.decode(&generated_tokens);

    Ok(GenerateResult {
        text,
        tokens_generated,
        tokens_per_sec,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_f16_to_f32_zero() {
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert_eq!(f16_to_f32(0x8000), -0.0);
    }

    #[test]
    fn test_f16_to_f32_one() {
        let result = f16_to_f32(0x3C00);
        assert!((result - 1.0).abs() < 1e-6, "expected 1.0, got {result}");
    }

    #[test]
    fn test_f16_to_f32_negative() {
        let result = f16_to_f32(0xC000);
        assert!((result - (-2.0)).abs() < 1e-6, "expected -2.0, got {result}");
    }

    #[test]
    fn test_f16_to_f32_half() {
        let result = f16_to_f32(0x3800);
        assert!((result - 0.5).abs() < 1e-6, "expected 0.5, got {result}");
    }

    #[test]
    fn test_f16_to_f32_infinity() {
        let result = f16_to_f32(0x7C00);
        assert!(result.is_infinite() && result > 0.0);
    }

    #[test]
    fn test_f16_to_f32_nan() {
        let result = f16_to_f32(0x7C01);
        assert!(result.is_nan());
    }

    #[test]
    fn test_dequant_q4_0_known_values() {
        let mut block = [0u8; 18];
        block[0] = 0x00;
        block[1] = 0x3C;
        for i in 2..18 {
            block[i] = 0x88;
        }

        let mut out = [0.0f32; 32];
        dequant_q4_0_block(&block, &mut out);
        for v in &out {
            assert!((v - 0.0).abs() < 1e-6, "expected 0.0, got {v}");
        }

        block[2] = 0xF0;
        dequant_q4_0_block(&block, &mut out);
        assert!((out[0] - (-8.0)).abs() < 1e-6);
        assert!((out[1] - 7.0).abs() < 1e-6);
    }

    #[test]
    fn test_dequant_q4_0_full() {
        let n = 64;
        let mut data = vec![0u8; 36];

        data[0] = 0x00;
        data[1] = 0x38;
        for i in 2..18 {
            data[i] = 0x88;
        }

        data[18] = 0x00;
        data[19] = 0x40;
        for i in 20..36 {
            data[i] = 0x99;
        }

        let result = dequant_q4_0(&data, n);
        assert_eq!(result.len(), 64);
        for v in &result[0..32] {
            assert!((v - 0.0).abs() < 1e-6);
        }
        for v in &result[32..64] {
            assert!((v - 2.0).abs() < 1e-3, "expected 2.0, got {v}");
        }
    }

    #[test]
    fn test_dequant_q4k_zero() {
        let block = [0u8; 144];
        let mut out = [0.0f32; 256];
        dequant_q4k_block(&block, &mut out);
        for v in &out {
            assert!(*v == 0.0, "expected 0.0, got {v}");
        }
    }

    #[test]
    fn test_matmul_f32_identity() {
        let mut weight = Vec::new();
        weight.extend_from_slice(&1.0f32.to_le_bytes());
        weight.extend_from_slice(&0.0f32.to_le_bytes());
        weight.extend_from_slice(&0.0f32.to_le_bytes());
        weight.extend_from_slice(&1.0f32.to_le_bytes());

        let x = [3.0f32, 7.0];
        let mut out = [0.0f32; 2];
        matmul_q(&mut out, &weight, &x, 2, QuantType::F32);

        assert!((out[0] - 3.0).abs() < 1e-6);
        assert!((out[1] - 7.0).abs() < 1e-6);
    }

    #[test]
    fn test_matmul_f32_simple() {
        let mut weight = Vec::new();
        for v in [1.0f32, 2.0, 3.0, 4.0] {
            weight.extend_from_slice(&v.to_le_bytes());
        }

        let x = [1.0f32; 4];
        let mut out = [0.0f32; 1];
        matmul_q(&mut out, &weight, &x, 4, QuantType::F32);

        assert!((out[0] - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_matmul_q4_0() {
        let mut weight = vec![0u8; 18];
        weight[0] = 0x00;
        weight[1] = 0x3C;
        for i in 2..18 {
            weight[i] = 0x99;
        }

        let x = vec![1.0f32; 32];
        let mut out = [0.0f32; 1];
        matmul_q(&mut out, &weight, &x, 32, QuantType::Q4_0);

        assert!((out[0] - 32.0).abs() < 0.5, "expected ~32.0, got {}", out[0]);
    }

    #[test]
    fn test_rmsnorm() {
        let x = [1.0f32, 2.0, 3.0, 4.0];
        let w = [1.0f32; 4];
        let mut out = [0.0f32; 4];
        rmsnorm(&mut out, &x, &w, 1e-5);
        let expected_scale = 1.0 / (7.5f32 + 1e-5).sqrt();
        for i in 0..4 {
            let expected = x[i] * expected_scale;
            assert!((out[i] - expected).abs() < 1e-4, "rmsnorm[{i}]: expected {expected}, got {}", out[i]);
        }
    }

    #[test]
    fn test_softmax() {
        let mut logits = [1.0f32, 2.0, 3.0];
        softmax(&mut logits);
        let sum: f32 = logits.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(logits[2] > logits[1]);
        assert!(logits[1] > logits[0]);
    }

    #[test]
    fn test_argmax() {
        assert_eq!(argmax(&[1.0, 5.0, 3.0, 2.0]), 1);
        assert_eq!(argmax(&[-1.0, -0.5, -2.0]), 1);
    }

    #[test]
    fn test_silu() {
        assert!((silu(0.0) - 0.0).abs() < 1e-6);
        // silu(1.0) = 1.0 / (1.0 + exp(-1.0)) ≈ 0.7311
        assert!((silu(1.0) - 0.7311).abs() < 0.001);
        // silu(-1.0) = -1.0 / (1.0 + exp(1.0)) ≈ -0.2689
        assert!((silu(-1.0) - (-0.2689)).abs() < 0.001);
    }

    #[test]
    fn test_apply_rope() {
        let mut q = vec![1.0f32, 0.0, 1.0, 0.0];
        let mut k = vec![1.0f32, 0.0, 1.0, 0.0];
        // pos=0 → angle=0 for all → cos=1, sin=0 → no change
        apply_rope(&mut q, &mut k, 0, 4, 10000.0);
        assert!((q[0] - 1.0).abs() < 1e-6);
        assert!((q[1] - 0.0).abs() < 1e-6);

        // pos=1 → rotation should change values
        let mut q2 = vec![1.0f32, 0.0, 1.0, 0.0];
        let mut k2 = vec![1.0f32, 0.0, 1.0, 0.0];
        apply_rope(&mut q2, &mut k2, 1, 4, 10000.0);
        // q should be rotated — values differ from [1,0,1,0]
        assert!((q2[0] - 1.0).abs() > 1e-6 || (q2[1] - 0.0).abs() > 1e-6);
    }
}
