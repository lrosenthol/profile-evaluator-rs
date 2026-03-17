#!/bin/zsh

set -euo pipefail

cd "$(dirname "$0")/.."

wasm-pack build --target web --out-dir ui/pkg
