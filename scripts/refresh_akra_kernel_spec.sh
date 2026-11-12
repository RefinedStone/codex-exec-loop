#!/usr/bin/env bash
# Rewrites schema/akra-kernel-spec.json from the kernel protocol types and tables (plan 18 K4-1).
# The file is the exported kernel spec. A test fails when it differs from the code.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
AKRA_REFRESH_KERNEL_SPEC=1 cargo test --locked --lib -- the_kernel_spec_file_matches_the_spec
