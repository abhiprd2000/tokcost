//! Codegen: turns the canonical tiktoken rank files in `assets/` into
//! compact embeddable blobs.
//!
//! Each `assets/<name>.tiktoken` file is the canonical text format tiktoken
//! itself ships: one `<base64-encoded-bytes> <rank>` pair per line, rank
//! ascending from 0. We don't want to embed that text (and its base64
//! bloat, plus a decode step) directly in the binary, so at build time we:
//!
//! 1. Parse and base64-decode every line (hand-rolled decoder, no crate).
//! 2. Verify ranks are exactly `0..N` in order — a corrupt or truncated
//!    asset file fails the build instead of silently producing a broken
//!    tokenizer.
//! 3. Emit two files per vocab into `$OUT_DIR`: a `.blob` (every token's
//!    raw bytes concatenated, in rank order) and an `.offsets` table (`N+1`
//!    little-endian `u32`s marking each token's start in the blob). Slicing
//!    `blob[offsets[r]..offsets[r+1]]` recovers token `r`'s bytes.
//! 4. Write `vocab_generated.rs`, which `include_bytes!`s those blobs and
//!    declares their token counts — the only thing `vocab.rs` needs to
//!    reconstruct the full rank tables at first use.

use std::env;
use std::fs;
use std::path::Path;

struct Vocab {
    name: &'static str,
    asset: &'static str,
}

const VOCABS: &[Vocab] = &[
    Vocab {
        name: "CL100K",
        asset: "cl100k_base.tiktoken",
    },
    Vocab {
        name: "O200K",
        asset: "o200k_base.tiktoken",
    },
];

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR set by cargo");
    let mut generated = String::new();

    for vocab in VOCABS {
        let asset_path = format!("assets/{}", vocab.asset);
        println!("cargo:rerun-if-changed={asset_path}");
        let text =
            fs::read_to_string(&asset_path).unwrap_or_else(|e| panic!("reading {asset_path}: {e}"));

        let mut blob = Vec::new();
        let mut offsets = vec![0u32];
        let mut expected_rank = 0u32;

        for (line_no, line) in text.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let (b64, rank_str) = line.split_once(' ').unwrap_or_else(|| {
                panic!("{asset_path}:{}: expected '<base64> <rank>'", line_no + 1)
            });
            let rank: u32 = rank_str
                .trim()
                .parse()
                .unwrap_or_else(|e| panic!("{asset_path}:{}: bad rank: {e}", line_no + 1));
            assert_eq!(
                rank,
                expected_rank,
                "{asset_path}:{}: ranks must be sequential from 0",
                line_no + 1
            );
            expected_rank += 1;

            let bytes = b64_decode(b64);
            blob.extend_from_slice(&bytes);
            offsets.push(blob.len() as u32);
        }

        let blob_path = Path::new(&out_dir).join(format!("{}.blob", vocab.asset));
        let offsets_path = Path::new(&out_dir).join(format!("{}.offsets", vocab.asset));
        fs::write(&blob_path, &blob).expect("write blob");
        let offsets_bytes: Vec<u8> = offsets.iter().flat_map(|o| o.to_le_bytes()).collect();
        fs::write(&offsets_path, &offsets_bytes).expect("write offsets");

        generated.push_str(&format!(
            "pub const {name}_COUNT: usize = {count};\n\
             pub static {name}_BLOB: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{asset}.blob\"));\n\
             pub static {name}_OFFSETS: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{asset}.offsets\"));\n\n",
            name = vocab.name,
            count = expected_rank,
            asset = vocab.asset,
        ));
    }

    fs::write(Path::new(&out_dir).join("vocab_generated.rs"), generated)
        .expect("write vocab_generated.rs");
}

fn b64_val(c: u8) -> Option<u8> {
    match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Standard-alphabet base64 decode (no URL-safe variant needed here).
/// Padding (`=`) is simply stripped before decoding since it only ever
/// shortens the final 4-char group, which the chunked loop below already
/// handles by output length.
fn b64_decode(s: &str) -> Vec<u8> {
    let vals: Vec<u8> = s
        .bytes()
        .filter(|&c| c != b'=')
        .map(|c| b64_val(c).unwrap_or_else(|| panic!("invalid base64 byte {c:#x} in {s:?}")))
        .collect();

    let mut out = Vec::with_capacity(vals.len() * 3 / 4 + 3);
    for chunk in vals.chunks(4) {
        out.push((chunk[0] << 2) | chunk.get(1).map_or(0, |v| v >> 4));
        if let Some(&v2) = chunk.get(2) {
            out.push((chunk[1] << 4) | (v2 >> 2));
        }
        if let Some(&v3) = chunk.get(3) {
            out.push((chunk[2] << 6) | v3);
        }
    }
    out
}
