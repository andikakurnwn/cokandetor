//! Token accounting: what each model call consumed and roughly what it cost.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{ProviderKind, Settings};

/// Token counts for one model call, or a sum of several.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    /// Input billed at the full rate (neither read from nor written to the cache).
    pub input: u64,
    /// Input written to the prompt cache (1.25x the input rate).
    pub cache_write: u64,
    /// Input served from the prompt cache.
    pub cache_read: u64,
    pub output: u64,
    /// Model calls summed into this value.
    pub calls: u32,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.input += o.input;
        self.cache_write += o.cache_write;
        self.cache_read += o.cache_read;
        self.output += o.output;
        self.calls += o.calls;
    }

    /// Everything the model was sent: the size of the context.
    pub fn prompt(&self) -> u64 {
        self.input + self.cache_write + self.cache_read
    }

    /// Share of the prompt served from the cache, 0–100.
    pub fn cached_pct(&self) -> u64 {
        match self.prompt() {
            0 => 0,
            p => self.cache_read * 100 / p,
        }
    }

    /// Anthropic `usage` (from `message_start`, or the running totals in
    /// `message_delta`): only the fields present are taken.
    pub fn merge_anthropic(&mut self, v: &Value) {
        let fields = [
            ("input_tokens", &mut self.input),
            ("cache_creation_input_tokens", &mut self.cache_write),
            ("cache_read_input_tokens", &mut self.cache_read),
            ("output_tokens", &mut self.output),
        ];
        for (key, slot) in fields {
            if let Some(n) = v[key].as_u64() {
                *slot = n;
            }
        }
    }

    /// OpenAI-compatible `usage`, where `prompt_tokens` includes the cached part.
    pub fn from_openai(v: &Value) -> Option<Self> {
        let prompt = v["prompt_tokens"].as_u64()?;
        let cached = v["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0).min(prompt);
        Some(Self {
            input: prompt - cached,
            cache_write: 0,
            cache_read: cached,
            output: v["completion_tokens"].as_u64().unwrap_or(0),
            calls: 1,
        })
    }
}

/// Claude list prices in USD per million tokens: (model-id prefix, input,
/// output, cache read). Most specific prefix first. Cache writes (5-minute
/// TTL) cost 1.25x input.
const CLAUDE_PRICES: &[(&str, f64, f64, f64)] = &[
    ("claude-fable-5-1", 10.0, 50.0, 0.25),
    ("claude-mythos-5-1", 10.0, 50.0, 0.25),
    ("claude-fable-5", 10.0, 50.0, 1.0),
    ("claude-mythos-5", 10.0, 50.0, 1.0),
    ("claude-opus-5-5", 4.0, 20.0, 0.20),
    ("claude-opus-5", 5.0, 25.0, 0.50),
    ("claude-opus-4-8", 5.0, 25.0, 0.50),
    ("claude-opus-4-7", 5.0, 25.0, 0.50),
    ("claude-opus-4-6", 5.0, 25.0, 0.50),
    ("claude-opus-4-5", 5.0, 25.0, 0.50),
    ("claude-opus-4", 15.0, 75.0, 1.50),
    ("claude-sonnet-5-5", 2.0, 10.0, 0.20),
    ("claude-sonnet-5", 2.0, 10.0, 0.20),
    ("claude-sonnet-4", 3.0, 15.0, 0.30),
    ("claude-haiku-5-5", 0.10, 0.50, 0.01),
    ("claude-haiku-4-5", 1.0, 5.0, 0.10),
];

/// Rough cost in USD of one call: `None` when the price is unknown.
pub fn cost(s: &Settings, u: &Usage) -> Option<f64> {
    match s.provider {
        ProviderKind::Local => Some(0.0),
        ProviderKind::OpenAI => None,
        ProviderKind::Anthropic => claude_cost(s.anthropic_model.trim(), u),
    }
}

fn claude_cost(model: &str, u: &Usage) -> Option<f64> {
    let &(_, mut input, mut output, mut read) = CLAUDE_PRICES.iter().find(|(p, ..)| model.starts_with(p))?;
    // Haiku 5.5 bills prompts above 100K tokens at 5x.
    if model.starts_with("claude-haiku-5-5") && u.prompt() > 100_000 {
        input *= 5.0;
        output *= 5.0;
        read *= 5.0;
    }
    let total = u.input as f64 * input
        + u.cache_write as f64 * input * 1.25
        + u.cache_read as f64 * read
        + u.output as f64 * output;
    Some(total / 1e6)
}

/// "850", "12,3 rb", "1,25 jt".
pub fn fmt_tokens(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    let s = if n < 1_000_000 {
        format!("{:.1} rb", n as f64 / 1e3)
    } else {
        format!("{:.2} jt", n as f64 / 1e6)
    };
    s.replace(".0 ", " ").replace('.', ",")
}

/// "$0,042" (Indonesian decimal comma).
pub fn fmt_usd(x: f64) -> String {
    let s = if x >= 1.0 {
        format!("{x:.2}")
    } else if x >= 0.01 {
        format!("{x:.3}")
    } else {
        format!("{x:.4}")
    };
    format!("${}", s.replace('.', ","))
}

/// The cost part of a summary line.
pub fn fmt_cost(cost: Option<f64>) -> String {
    match cost {
        None => "biaya tidak diketahui".into(),
        Some(0.0) => "gratis".into(),
        Some(c) => format!("≈ {}", fmt_usd(c)),
    }
}

/// "54,2 rb token masuk (91% dari cache) · 2,1 rb keluar · ≈ $0,040".
pub fn summary(u: &Usage, cost: Option<f64>) -> String {
    format!(
        "{} token masuk ({}% dari cache) · {} keluar · {}",
        fmt_tokens(u.prompt()),
        u.cached_pct(),
        fmt_tokens(u.output),
        fmt_cost(cost)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_both_usage_shapes() {
        let mut u = Usage::default();
        u.merge_anthropic(&json!({"input_tokens": 12, "cache_creation_input_tokens": 300,
                                   "cache_read_input_tokens": 9000, "output_tokens": 1}));
        u.merge_anthropic(&json!({"output_tokens": 450}));
        assert_eq!((u.input, u.cache_write, u.cache_read, u.output), (12, 300, 9000, 450));
        assert_eq!(u.prompt(), 9312);
        assert_eq!(u.cached_pct(), 96);

        let o = Usage::from_openai(&json!({"prompt_tokens": 1000, "completion_tokens": 50,
                                           "prompt_tokens_details": {"cached_tokens": 800}}))
        .unwrap();
        assert_eq!((o.input, o.cache_read, o.output), (200, 800, 50));
        assert!(Usage::from_openai(&Value::Null).is_none());
    }

    #[test]
    fn prices_by_model() {
        let u = Usage { input: 1_000_000, cache_write: 0, cache_read: 1_000_000, output: 1_000_000, calls: 1 };
        let c = claude_cost("claude-opus-5-5", &u).unwrap();
        assert!((c - 24.2).abs() < 1e-9, "{c}");
        // Dated ids match their family; unknown models have no price.
        assert!((claude_cost("claude-sonnet-4-5-20250929", &u).unwrap() - 18.3).abs() < 1e-9);
        assert!(claude_cost("my-proxy-model", &u).is_none());
        // Haiku 5.5 above 100K prompt tokens is billed at the higher tier.
        assert!((claude_cost("claude-haiku-5-5", &u).unwrap() - 3.05).abs() < 1e-9);
    }

    #[test]
    fn formats_indonesian() {
        assert_eq!(fmt_tokens(850), "850");
        assert_eq!(fmt_tokens(12_345), "12,3 rb");
        assert_eq!(fmt_tokens(3_000), "3 rb");
        assert_eq!(fmt_tokens(1_250_000), "1,25 jt");
        assert_eq!(fmt_usd(0.0423), "$0,042");
        assert_eq!(fmt_usd(1.5), "$1,50");
        assert_eq!(fmt_cost(Some(0.0)), "gratis");
    }
}
