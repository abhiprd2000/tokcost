#!/usr/bin/env python3
"""Generate golden BPE fixtures for tokcost's tests/golden.rs.

Requires the real `tiktoken` package (`pip install tiktoken`), which is a
*dev-only* dependency of this script — it never ships in the tokcost
binary. tokcost's own encoder is the hand-rolled one in src/bpe.rs; this
script exists purely to produce ground truth to check that encoder
against.

Usage:
    pip install tiktoken
    python3 scripts/gen_golden_fixtures.py

Writes tests/fixtures/{cl100k,o200k}.fixtures: one test case per line, as
`<base64-encoded-utf8-text> <comma-separated-token-ids>`. Base64 avoids any
escaping headache for cases containing newlines, tabs, or quotes, and the
Rust side already has a base64 decoder (build.rs) so this reuses that
format rather than inventing a second one.
"""

import base64
import pathlib
import sys

try:
    import tiktoken
except ImportError:
    print("error: this script requires the 'tiktoken' package.", file=sys.stderr)
    print("       pip install tiktoken", file=sys.stderr)
    sys.exit(1)

# Deliberately exercises the same edge cases src/bpe.rs's pretokenizer
# tests reason about by hand: contractions, digit grouping into runs of
# three, whitespace-run boundaries, blank lines, camelCase/acronym runs,
# CJK, emoji, and ordinary prose/code.
CASES = [
    "",
    "a",
    "A",
    "hello world",
    "Hello, world!",
    "don't stop believing",
    "I'll be there, you're welcome, we've done it, they'd know",
    "don't can't won't isn't aren't",
    # Contraction case-folding: both vocabs' contraction groups are (?i:),
    # but they place boundaries differently (cl100k: x | 'S | y,
    # o200k: x'S | y). These pin the case-insensitive behavior.
    "x'Sy",
    "DON'T STOP",
    "it'S weird",
    "O'Brien and O'BRIEN",
    "abc'LL def",
    "The year 2024 had 365 days and 8760 hours.",
    "1234567890",
    "   leading and trailing whitespace   ",
    "line one\nline two\n\nline four after blank",
    "tabs\tare\tfun",
    "multiple   spaces   between   words",
    "trailing spaces at the end   ",
    "hello\n",
    "\n\n\n\n",
    "     ",
    "café naïve résumé Zürich",
    "日本語のテキストです",
    "emoji test 🚀🔥😀 works?",
    "https://example.com/path?query=1&other=2",
    "/usr/local/bin/tokcost",
    "snake_case and camelCase and PascalCase and SCREAMING_SNAKE",
    "HTMLParser XMLHttpRequest NASA iPhone",
    "AAAA",
    "aaaa",
    "ABCdef",
    "Special chars: !@#$%^&*()_+-=[]{}|;':\",./<>?",
    "Mixed 123abc456def",
    "wait... really?! no way!!!",
    "$100.50 costs €90 or £80",
    "# Heading\n\n- item one\n- item two\n\n**bold** and _italic_",
    "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    "def add(a, b):\n    return a + b\n",
    (
        "The quick brown fox jumps over the lazy dog, again and again, "
        "until the sun sets over the quiet hills and the stars come out "
        "one by one."
    ),
]

ENCODINGS = [
    ("cl100k_base", "cl100k"),
    ("o200k_base", "o200k"),
]


def main() -> None:
    repo_root = pathlib.Path(__file__).resolve().parent.parent
    fixtures_dir = repo_root / "tests" / "fixtures"
    fixtures_dir.mkdir(parents=True, exist_ok=True)

    for encoding_name, out_name in ENCODINGS:
        enc = tiktoken.get_encoding(encoding_name)
        lines = []
        for text in CASES:
            token_ids = enc.encode_ordinary(text)
            b64 = base64.b64encode(text.encode("utf-8")).decode("ascii")
            ids = ",".join(str(t) for t in token_ids)
            lines.append(f"{b64} {ids}")

        out_path = fixtures_dir / f"{out_name}.fixtures"
        out_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        print(f"wrote {len(lines)} cases to {out_path.relative_to(repo_root)}")


if __name__ == "__main__":
    main()
