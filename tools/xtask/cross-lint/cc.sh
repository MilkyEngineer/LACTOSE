#!/bin/sh
# Stand-in C compiler for cross-target clippy: clippy never links, so a C dependency's
# objects only need to exist. `-E` (the cc crate's detection) prints nothing; `-o <file>`
# gets an empty file.
out=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "-o" ]; then out="$arg"; fi
  prev="$arg"
done
if [ -n "$out" ]; then : > "$out"; fi
exit 0
