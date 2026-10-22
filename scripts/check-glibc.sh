#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <shared-library> <maximum-glibc-version>" >&2
  exit 2
fi

library=$1
maximum=$2

if [[ ! -f "$library" ]]; then
  echo "shared library not found: $library" >&2
  exit 1
fi

required=$(readelf --version-info "$library" \
  | sed -n 's/.*Name: \(GLIBC_[0-9.]*\).*/\1/p' \
  | sort -Vu \
  | tail -n 1)

if [[ -z "$required" ]]; then
  echo "no GLIBC version requirement found in $library" >&2
  exit 1
fi

if [[ $(printf '%s\n%s\n' "$required" "$maximum" | sort -V | tail -n 1) != "$maximum" ]]; then
  echo "$library requires $required, which exceeds supported $maximum" >&2
  exit 1
fi

echo "$library requires $required (supported through $maximum)"
