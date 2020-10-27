#!/bin/bash
set -e

cargo run --features "${GOOSE_FEATURES}" --release --example "${GOOSE_EXAMPLE}" -- $@
