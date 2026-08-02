//! Static USD pricing table, with a `TK_PRICES` environment override.
//!
//! Every LLM provider's pricing changes on its own schedule and none of it
//! is queryable without a network call, which this tool deliberately never
//! makes. So the table below is a dated snapshot (see `PRICING_AS_OF`), not
//! a live source of truth. `TK_PRICES` lets you correct it without waiting
//! for a new release: set it to a comma-separated list of
//! `model=input_per_million:output_per_million` USD rates, e.g.
//!
//! ```text
//! TK_PRICES="gpt-4o=2.50:10.00,my-custom-model=1:2" tokcost ...
//! ```
//!
//! A model name matches the *longest* table key that's a prefix of it (so
//! `gpt-4o-mini-2024-07-18` matches the `gpt-4o-mini` entry, not the
//! shorter `gpt-4o`). `TK_PRICES` entries are checked as their own
//! namespace first — if any override prefix-matches, it wins outright,
//! otherwise the built-in table is consulted the same way.

use std::env;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Price {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cost {
    pub input_usd: f64,
    pub output_usd: f64,
}

impl Cost {
    pub fn total_usd(&self) -> f64 {
        self.input_usd + self.output_usd
    }
}

/// The date this table was last checked against provider pricing pages.
/// Treat anything computed from it as approximate past this date.
pub const PRICING_AS_OF: &str = "2025-06-01";

const fn price(input_per_million: f64, output_per_million: f64) -> Price {
    Price {
        input_per_million,
        output_per_million,
    }
}

/// Snapshot of published per-million-token USD pricing as of
/// `PRICING_AS_OF`. Embedding models have no output rate (0.0).
const BASE_PRICES: &[(&str, Price)] = &[
    ("gpt-3.5-turbo", price(0.50, 1.50)),
    ("gpt-4-32k", price(60.00, 120.00)),
    ("gpt-4", price(30.00, 60.00)),
    ("gpt-4o-mini", price(0.15, 0.60)),
    ("gpt-4o", price(2.50, 10.00)),
    ("o1-mini", price(3.00, 12.00)),
    ("o1", price(15.00, 60.00)),
    ("o3-mini", price(1.10, 4.40)),
    ("text-embedding-ada-002", price(0.10, 0.0)),
    ("claude-3-5-sonnet", price(3.00, 15.00)),
    ("claude-3-5-haiku", price(0.80, 4.00)),
    ("claude-3-opus", price(15.00, 75.00)),
    ("claude-3-haiku", price(0.25, 1.25)),
];

/// Find the price for `model` among `entries` by longest matching prefix.
/// Pure and allocation-free beyond the iterator, so it's directly testable
/// without touching process environment state.
fn best_prefix_match<'a>(
    model: &str,
    entries: impl Iterator<Item = (&'a str, Price)>,
) -> Option<Price> {
    entries
        .filter(|(key, _)| model.starts_with(key))
        .max_by_key(|(key, _)| key.len())
        .map(|(_, p)| p)
}

/// Parse a `TK_PRICES`-formatted string into `(lowercased model, Price)`
/// pairs. Malformed entries are skipped rather than failing the whole
/// parse, so one typo doesn't take down every other override.
fn parse_tk_prices(raw: &str) -> Vec<(String, Price)> {
    raw.split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            if entry.is_empty() {
                return None;
            }
            let (model, rates) = entry.split_once('=')?;
            let (input, output) = rates.split_once(':')?;
            Some((
                model.trim().to_ascii_lowercase(),
                price(input.trim().parse().ok()?, output.trim().parse().ok()?),
            ))
        })
        .collect()
}

/// Core lookup: overrides are their own namespace, tried before the
/// built-in table.
fn lookup(model: &str, overrides: &[(String, Price)]) -> Option<Price> {
    let model = model.to_ascii_lowercase();
    best_prefix_match(&model, overrides.iter().map(|(k, p)| (k.as_str(), *p)))
        .or_else(|| best_prefix_match(&model, BASE_PRICES.iter().copied()))
}

/// Look up the price for `model`, honoring a `TK_PRICES` override if set.
/// Returns `None` when nothing matches — callers should surface that as
/// "pricing unknown for this model", never guess.
pub fn price_for(model: &str) -> Option<Price> {
    let overrides = env::var("TK_PRICES")
        .ok()
        .map(|raw| parse_tk_prices(&raw))
        .unwrap_or_default();
    lookup(model, &overrides)
}

/// Estimate USD cost for `input_tokens`/`output_tokens` under `model`'s
/// pricing. Returns `None` if the model has no known price.
pub fn estimate_cost(model: &str, input_tokens: usize, output_tokens: usize) -> Option<Cost> {
    let price = price_for(model)?;
    Some(Cost {
        input_usd: input_tokens as f64 / 1_000_000.0 * price.input_per_million,
        output_usd: output_tokens as f64 / 1_000_000.0 * price.output_per_million,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_prefix_wins_among_overlapping_families() {
        assert_eq!(lookup("gpt-4", &[]), Some(price(30.00, 60.00)));
        assert_eq!(lookup("gpt-4-32k", &[]), Some(price(60.00, 120.00)));
        assert_eq!(lookup("gpt-4-32k-0613", &[]), Some(price(60.00, 120.00)));
        assert_eq!(lookup("gpt-4o", &[]), Some(price(2.50, 10.00)));
        assert_eq!(
            lookup("gpt-4o-mini-2024-07-18", &[]),
            Some(price(0.15, 0.60))
        );
    }

    #[test]
    fn unknown_model_has_no_price() {
        assert_eq!(lookup("some-fictional-model", &[]), None);
    }

    #[test]
    fn overrides_take_priority_over_base_table() {
        let overrides = vec![("gpt-4o".to_string(), price(1.0, 2.0))];
        assert_eq!(lookup("gpt-4o", &overrides), Some(price(1.0, 2.0)));
        // A model the base table also knows, but the override doesn't
        // mention, still falls back to the base table.
        assert_eq!(lookup("gpt-4", &overrides), Some(price(30.00, 60.00)));
    }

    #[test]
    fn overrides_can_cover_models_the_base_table_does_not() {
        let overrides = vec![("my-custom-model".to_string(), price(1.0, 2.0))];
        assert_eq!(lookup("my-custom-model", &overrides), Some(price(1.0, 2.0)));
    }

    #[test]
    fn parse_tk_prices_reads_comma_separated_pairs() {
        let parsed = parse_tk_prices("gpt-4o=2.5:10,my-model=1:2");
        assert_eq!(
            parsed,
            vec![
                ("gpt-4o".to_string(), price(2.5, 10.0)),
                ("my-model".to_string(), price(1.0, 2.0)),
            ]
        );
    }

    #[test]
    fn parse_tk_prices_skips_malformed_entries() {
        // Missing ':' in the second entry shouldn't drop the valid first one.
        let parsed = parse_tk_prices("gpt-4o=2.5:10,broken-entry,claude-x=3:15");
        assert_eq!(
            parsed,
            vec![
                ("gpt-4o".to_string(), price(2.5, 10.0)),
                ("claude-x".to_string(), price(3.0, 15.0)),
            ]
        );
    }

    #[test]
    fn parse_tk_prices_of_empty_string_is_empty() {
        assert!(parse_tk_prices("").is_empty());
    }

    #[test]
    fn cost_total_sums_input_and_output() {
        let cost = Cost {
            input_usd: 1.5,
            output_usd: 2.5,
        };
        assert_eq!(cost.total_usd(), 4.0);
    }

    #[test]
    fn estimate_cost_scales_by_million_tokens() {
        let overrides = vec![("test-model".to_string(), price(10.0, 20.0))];
        let cost = lookup("test-model", &overrides).map(|p| Cost {
            input_usd: 500_000.0 / 1_000_000.0 * p.input_per_million,
            output_usd: 250_000.0 / 1_000_000.0 * p.output_per_million,
        });
        assert_eq!(
            cost,
            Some(Cost {
                input_usd: 5.0,
                output_usd: 5.0
            })
        );
    }
}
