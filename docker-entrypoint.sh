#!/bin/bash
set -e

echo '151.101.2.132 umami.tag1.io' >> /etc/hosts
cargo run --features "${GOOSE_FEATURES}" --release --example "${GOOSE_EXAMPLE}" -- $@
