#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# EXAMPLE: MoE Model with TurboQuant & MTP Speculative Decoding (AtomicBot Fork)
# 
# PLATFORM: macOS (Apple Silicon / Metal)
# 
# This script demonstrates how to run a Mixture-of-Experts (MoE) model.
# On Apple Silicon (M1/M2/M3/M4), memory is unified, but routing specific experts 
# to CPU can still be useful if Metal memory limits are exceeded.
# ==============================================================================

MODEL_PATH="${MODEL_PATH:-/Users/Shared/models/Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf}"
LLAMA_SERVER="${LLAMA_SERVER:-/Users/Shared/atomic-llama-cpp-turboquant/build/bin/llama-server}"

PORT="${PORT:-8090}"
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
  --n-gpu-layers 999 \
  `# If your Unified Memory is too small for the whole MoE, keeping` \
  `# inactive experts on the CPU side of the RAM can bypass Metal allocation limits.` \
  --override-tensor "\.ffn_.*_exps\.=CPU" \
  \
  `# --- Context & Memory ---` \
  `# Set context window size (e.g., 256K)` \
  --ctx-size 262144 \
  `# Lock weights in RAM to prevent OS swap-induced latency spikes` \
  --mlock \
  `# Concurrent request slots; each consumes extra KV cache memory` \
  --parallel "$PARALLEL_SLOTS" \
  \
  `# --- MTP Speculative Decoding ---` \
  `# Use the same model for drafting (MTP leverages already-loaded target weights)` \
  --model-draft "$MODEL_PATH" \
  --spec-type draft-mtp \
  --spec-draft-n-max 2 \
  --spec-draft-n-min 1 \
  --spec-draft-ngl 999 \
  --spec-draft-override-tensor "\.ffn_.*_exps\.=CPU" \
  \
  `# --- TurboQuant KV Cache Compression ---` \
  `# turbo3 = 3-bit KV Cache (~4.3x memory savings vs F16).` \
  `# swap to turbo2 for max compression or turbo4 for max accuracy.` \
  --flash-attn on \
  --cache-type-k turbo3 \
  --cache-type-v turbo3 \
  \
  `# --- Prompt Caching & Continuous Batching ---` \
  `# Drastically reduces TTFT for agentic tools (Claude Code, Hermes, etc.)` \
  --cache-reuse 256 \
  --cont-batching \
  \
  `# --- Inference Parameters ---` \
  --temp 0.6 \
  --top-p 0.95 \
  --top-k 20 \
  --repeat-penalty 1.05
