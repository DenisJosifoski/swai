#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# EXAMPLE: MoE Model with TurboQuant & MTP Speculative Decoding (AtomicBot Fork)
# 
# PLATFORM: Linux
# 
# This script demonstrates how to run a Mixture-of-Experts (MoE) model where
# the model weights are split. It offloads the shared layers and attention 
# to the GPU while keeping the massive routed FFN experts on CPU/RAM.
# 
# Designed for the AtomicBot llama.cpp fork which features `turboquant` 
# KV cache compression and `draft-mtp` speculative decoding.
# ==============================================================================

# 1. Define paths (Update these paths to match your actual files and directories)
MODEL_PATH="${MODEL_PATH:-/path/to/your/models/Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf}"
LLAMA_SERVER="${LLAMA_SERVER:-/path/to/atomic-llama-cpp-turboquant/build/bin/llama-server}"

# 2. Network & UI variables (SWAI will override PORT automatically)
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
  `# --- MoE Expert Placement ---` \
  `# Keep attention + shared/always-active tensors on GPU.` \
  --n-gpu-layers 999 \
  `# Force routed FFN expert tensors to CPU/system RAM. This allows massive` \
  `# MoE models (like 35B) to run on 12GB GPUs by only holding active experts.` \
  `# Note: Adjust the regex if tensor naming differs on your specific quant.` \
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
