#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# EXAMPLE: Standard Dense Model with TurboQuant (AtomicBot Fork)
# 
# PLATFORM: macOS (Apple Silicon / Metal)
# 
# This script demonstrates how to run a standard dense model fully offloaded
# to Apple's Unified Memory via the Metal backend.
# ==============================================================================

MODEL_PATH="${MODEL_PATH:-/Users/Shared/models/Dense-Model-9B-Q6_K.gguf}"
LLAMA_SERVER="${LLAMA_SERVER:-/Users/Shared/atomic-llama-cpp-turboquant/build/bin/llama-server}"

PORT="${PORT:-8080}"
PARALLEL_SLOTS="${PARALLEL_SLOTS:-1}"

exec "$LLAMA_SERVER" \
  --model "$MODEL_PATH" \
  --port "$PORT" \
  --host 127.0.0.1 \
  \
  # CRITICAL FOR SWAI: The --slots flag enables the /slots endpoint \
  # which SWAI uses to monitor the context window progress. \
  --slots \
  \
  `# --- Metal GPU Offloading ---` \
  `# Offloads all layers to Apple Silicon GPU` \
  --n-gpu-layers 999 \
  \
  `# --- Context & Memory ---` \
  `# Set context window size (e.g., 32K)` \
  --ctx-size 32768 \
  `# Lock weights in RAM to prevent OS swap-induced latency spikes` \
  --mlock \
  `# Concurrent request slots; each consumes extra KV cache memory` \
  --parallel "$PARALLEL_SLOTS" \
  \
  `# --- TurboQuant KV Cache Compression ---` \
  `# turbo4 = 4-bit KV Cache (excellent quality retention for dense models).` \
  --flash-attn on \
  --cache-type-k turbo4 \
  --cache-type-v turbo4 \
  \
  `# --- Prompt Caching & Continuous Batching ---` \
  `# Drastically reduces TTFT for agentic tools (Claude Code, Hermes, etc.)` \
  --cache-reuse 256 \
  --cont-batching \
  \
  `# --- Inference Parameters ---` \
  --temp 0.8 \
  --top-p 0.90 \
  --top-k 40 \
  --repeat-penalty 1.10
