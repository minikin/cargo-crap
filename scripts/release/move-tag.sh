#!/usr/bin/env bash
set -euo pipefail
commit=$(git rev-parse --verify --end-of-options "${1:?expected release commit}^{commit}")
git push --force origin "$commit:refs/tags/crapts-v1"
