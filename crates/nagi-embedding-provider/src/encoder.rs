//! BERT encoder forward pass (post-LayerNorm, exact-erf GELU, absolute
//! positions, token type 0) followed by mean pooling. Weights are decoded
//! once from the artifact; word-embedding rows are decoded on demand.

use alloc::{string::String, vec, vec::Vec};

use crate::container::{read_f32s, Header, ModelError, TensorRef};
use crate::Checkpoint;

struct Linear {
    weight: Vec<f32>, // [out, in], row-major
    bias: Vec<f32>,
    inputs: usize,
    outputs: usize,
}

struct Norm {
    weight: Vec<f32>,
    bias: Vec<f32>,
}

struct Layer {
    query: Linear,
    key: Linear,
    value: Linear,
    attention_out: Linear,
    attention_norm: Norm,
    intermediate: Linear,
    output: Linear,
    output_norm: Norm,
}

pub struct Encoder {
    hidden: usize,
    heads: usize,
    eps: f32,
    max_positions: usize,
    word_offset: usize,
    word_rows: usize,
    positions: Vec<f32>,
    token_type0: Vec<f32>,
    embedding_norm: Norm,
    layers: Vec<Layer>,
}

/// Callback polled at every encoder [`Checkpoint`]; returning `true` aborts
/// inference at that checkpoint.
pub trait Interrupt {
    fn should_stop(&self, at: Checkpoint) -> bool;
}

#[derive(Debug, Eq, PartialEq)]
pub enum EncodeError {
    Interrupted(Checkpoint),
    InvalidToken,
    TooManyTokens,
}

struct Tensors<'a> {
    bytes: &'a [u8],
    list: &'a [TensorRef],
}

impl Tensors<'_> {
    fn find(&self, name: &str, shape: &[usize]) -> Result<&TensorRef, ModelError> {
        self.list
            .iter()
            .find(|t| t.name == name)
            .filter(|t| t.shape == shape)
            .ok_or_else(|| ModelError::Tensor(String::from(name)))
    }

    fn load(&self, name: &str, shape: &[usize]) -> Result<Vec<f32>, ModelError> {
        let t = self.find(name, shape)?;
        let values = read_f32s(self.bytes, t.offset, t.elements);
        if values.iter().any(|v| !v.is_finite()) {
            return Err(ModelError::Tensor(String::from(name)));
        }
        Ok(values)
    }

    fn linear(&self, prefix: &str, outputs: usize, inputs: usize) -> Result<Linear, ModelError> {
        Ok(Linear {
            weight: self.load(&alloc::format!("{prefix}.weight"), &[outputs, inputs])?,
            bias: self.load(&alloc::format!("{prefix}.bias"), &[outputs])?,
            inputs,
            outputs,
        })
    }

    fn norm(&self, prefix: &str, width: usize) -> Result<Norm, ModelError> {
        Ok(Norm {
            weight: self.load(&alloc::format!("{prefix}.weight"), &[width])?,
            bias: self.load(&alloc::format!("{prefix}.bias"), &[width])?,
        })
    }
}

impl Encoder {
    pub fn new(bytes: &[u8], header: &Header, list: &[TensorRef]) -> Result<Self, ModelError> {
        let t = Tensors { bytes, list };
        let h = header.hidden;
        let word = t.find("embeddings.word_embeddings.weight", &[header.vocab_rows, h])?;
        let mut layers = Vec::with_capacity(header.layers);
        for index in 0..header.layers {
            let p = alloc::format!("encoder.layer.{index}");
            layers.push(Layer {
                query: t.linear(&alloc::format!("{p}.attention.self.query"), h, h)?,
                key: t.linear(&alloc::format!("{p}.attention.self.key"), h, h)?,
                value: t.linear(&alloc::format!("{p}.attention.self.value"), h, h)?,
                attention_out: t.linear(&alloc::format!("{p}.attention.output.dense"), h, h)?,
                attention_norm: t.norm(&alloc::format!("{p}.attention.output.LayerNorm"), h)?,
                intermediate: t.linear(
                    &alloc::format!("{p}.intermediate.dense"),
                    header.intermediate,
                    h,
                )?,
                output: t.linear(&alloc::format!("{p}.output.dense"), h, header.intermediate)?,
                output_norm: t.norm(&alloc::format!("{p}.output.LayerNorm"), h)?,
            });
        }
        let token_types = t.load(
            "embeddings.token_type_embeddings.weight",
            &[header.type_vocab, h],
        )?;
        Ok(Self {
            hidden: h,
            heads: header.heads,
            eps: header.layer_norm_eps,
            max_positions: header.max_positions,
            word_offset: word.offset,
            word_rows: header.vocab_rows,
            positions: t.load(
                "embeddings.position_embeddings.weight",
                &[header.max_positions, h],
            )?,
            token_type0: token_types[..h].into(),
            embedding_norm: t.norm("embeddings.LayerNorm", h)?,
            layers,
        })
    }

    pub fn max_positions(&self) -> usize {
        self.max_positions
    }

    pub fn hidden(&self) -> usize {
        self.hidden
    }

    /// Mean-pooled (not yet normalized) sentence vector for `ids`.
    pub fn encode(
        &self,
        bytes: &[u8],
        ids: &[u32],
        interrupt: &dyn Interrupt,
    ) -> Result<Vec<f32>, EncodeError> {
        let h = self.hidden;
        let n = ids.len();
        if n == 0 || n > self.max_positions {
            return Err(EncodeError::TooManyTokens);
        }
        let mut x = vec![0.0f32; n * h];
        for (t, &id) in ids.iter().enumerate() {
            let id = id as usize;
            if id >= self.word_rows {
                return Err(EncodeError::InvalidToken);
            }
            let row = &mut x[t * h..(t + 1) * h];
            let start = self.word_offset + id * h * 4;
            for (i, value) in row.iter_mut().enumerate() {
                let b = &bytes[start + i * 4..start + i * 4 + 4];
                *value = f32::from_le_bytes([b[0], b[1], b[2], b[3]])
                    + self.positions[t * h + i]
                    + self.token_type0[i];
            }
            layer_norm(row, &self.embedding_norm, self.eps);
        }

        let head_dim = h / self.heads;
        let scale = 1.0 / libm::sqrtf(head_dim as f32);
        let mut q = vec![0.0f32; n * h];
        let mut k = vec![0.0f32; n * h];
        let mut v = vec![0.0f32; n * h];
        let mut context = vec![0.0f32; n * h];
        let mut projected = vec![0.0f32; n * h];
        let mut scores = vec![0.0f32; n];
        let inter_width = self.layers.first().map_or(0, |l| l.intermediate.outputs);
        let mut inter = vec![0.0f32; n * inter_width];

        for (index, layer) in self.layers.iter().enumerate() {
            if interrupt.should_stop(Checkpoint::LayerStart(index)) {
                return Err(EncodeError::Interrupted(Checkpoint::LayerStart(index)));
            }
            layer.query.apply(&x, &mut q, n);
            layer.key.apply(&x, &mut k, n);
            layer.value.apply(&x, &mut v, n);
            for head in 0..self.heads {
                let off = head * head_dim;
                for i in 0..n {
                    let qi = &q[i * h + off..i * h + off + head_dim];
                    let mut max = f32::NEG_INFINITY;
                    for (j, score) in scores.iter_mut().enumerate() {
                        *score = dot(qi, &k[j * h + off..j * h + off + head_dim]) * scale;
                        max = max.max(*score);
                    }
                    let mut sum = 0.0f32;
                    for score in scores.iter_mut() {
                        *score = libm::expf(*score - max);
                        sum += *score;
                    }
                    let out = &mut context[i * h + off..i * h + off + head_dim];
                    out.fill(0.0);
                    for (j, score) in scores.iter().enumerate() {
                        let weight = score / sum;
                        let vj = &v[j * h + off..j * h + off + head_dim];
                        for (o, value) in out.iter_mut().zip(vj) {
                            *o += weight * value;
                        }
                    }
                }
            }
            layer.attention_out.apply(&context, &mut projected, n);
            for t in 0..n {
                let row = &mut x[t * h..(t + 1) * h];
                for (value, add) in row.iter_mut().zip(&projected[t * h..(t + 1) * h]) {
                    *value += add;
                }
                layer_norm(row, &layer.attention_norm, self.eps);
            }
            if interrupt.should_stop(Checkpoint::LayerMid(index)) {
                return Err(EncodeError::Interrupted(Checkpoint::LayerMid(index)));
            }
            layer.intermediate.apply(&x, &mut inter, n);
            for value in inter.iter_mut() {
                *value = gelu(*value);
            }
            layer.output.apply(&inter, &mut projected, n);
            for t in 0..n {
                let row = &mut x[t * h..(t + 1) * h];
                for (value, add) in row.iter_mut().zip(&projected[t * h..(t + 1) * h]) {
                    *value += add;
                }
                layer_norm(row, &layer.output_norm, self.eps);
            }
        }

        if interrupt.should_stop(Checkpoint::Encoded) {
            return Err(EncodeError::Interrupted(Checkpoint::Encoded));
        }
        let mut pooled = vec![0.0f64; h];
        for t in 0..n {
            for (p, value) in pooled.iter_mut().zip(&x[t * h..(t + 1) * h]) {
                *p += f64::from(*value);
            }
        }
        if interrupt.should_stop(Checkpoint::Pooled) {
            return Err(EncodeError::Interrupted(Checkpoint::Pooled));
        }
        Ok(pooled.into_iter().map(|p| (p / n as f64) as f32).collect())
    }
}

impl Linear {
    fn apply(&self, input: &[f32], output: &mut [f32], rows: usize) {
        for r in 0..rows {
            let x = &input[r * self.inputs..(r + 1) * self.inputs];
            let y = &mut output[r * self.outputs..(r + 1) * self.outputs];
            for (o, out) in y.iter_mut().enumerate() {
                let w = &self.weight[o * self.inputs..(o + 1) * self.inputs];
                *out = self.bias[o] + dot(x, w);
            }
        }
    }
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut lanes = [0.0f32; 8];
    let chunks = a.len() / 8;
    for c in 0..chunks {
        let aa = &a[c * 8..c * 8 + 8];
        let bb = &b[c * 8..c * 8 + 8];
        for l in 0..8 {
            lanes[l] += aa[l] * bb[l];
        }
    }
    let mut sum = lanes.iter().sum::<f32>();
    for i in chunks * 8..a.len() {
        sum += a[i] * b[i];
    }
    sum
}

fn layer_norm(row: &mut [f32], norm: &Norm, eps: f32) {
    let n = row.len() as f64;
    let mean = row.iter().map(|v| f64::from(*v)).sum::<f64>() / n;
    let var = row
        .iter()
        .map(|v| {
            let d = f64::from(*v) - mean;
            d * d
        })
        .sum::<f64>()
        / n;
    let inv = 1.0 / libm::sqrt(var + f64::from(eps));
    for (i, value) in row.iter_mut().enumerate() {
        let normalized = ((f64::from(*value) - mean) * inv) as f32;
        *value = normalized * norm.weight[i] + norm.bias[i];
    }
}

#[inline]
fn gelu(x: f32) -> f32 {
    0.5 * x * (1.0 + libm::erff(x * core::f32::consts::FRAC_1_SQRT_2))
}
