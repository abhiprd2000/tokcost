#!/usr/bin/env bash
# Emits text word-by-word with small delays, standing in for a live LLM
# response for the `tokcost meter` demo recording — no real API needed.
set -euo pipefail

text="Sure, since the headphones themselves are fine and it's just the case that arrived damaged, I can send a replacement case right away instead of a full refund. I've flagged order #48213 and a new case will ship within one business day."

for word in $text; do
    printf '%s ' "$word"
    sleep 0.09
done
printf '\n'
