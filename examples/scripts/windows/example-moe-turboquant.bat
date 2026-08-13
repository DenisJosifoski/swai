@echo off
setlocal

:: ==============================================================================
:: EXAMPLE: MoE Model with TurboQuant & MTP Speculative Decoding (AtomicBot Fork)
:: 
:: PLATFORM: Windows
:: 
:: This script demonstrates how to run a Mixture-of-Experts (MoE) model.
:: Note the use of ^ for line continuation in Windows batch scripts.
:: ==============================================================================

:: 1. Define paths (Update these paths to match your actual files and directories)
if not defined MODEL_PATH set MODEL_PATH=C:\ai-models\Qwen3.6-35B-A3B-UD-Q4_K_XL.gguf
if not defined LLAMA_SERVER set LLAMA_SERVER=C:\atomic-llama-cpp-turboquant\build\bin\Release\llama-server.exe

:: 2. Network & UI variables (SWAI will override PORT automatically)
if not defined PORT set PORT=8090
if not defined PARALLEL_SLOTS set PARALLEL_SLOTS=1

"%LLAMA_SERVER%" ^
  --model "%MODEL_PATH%" ^
  --port "%PORT%" ^
  --host 127.0.0.1 ^
  :: CRITICAL FOR SWAI: The --slots flag enables the /slots endpoint ^
  :: which SWAI uses to monitor the context window progress. ^
  --slots ^
  ^
  :: --- MoE Expert Placement --- ^
  :: Keep attention + shared/always-active tensors on GPU. ^
  --n-gpu-layers 999 ^
  :: Push routed experts to CPU RAM while keeping attention on GPU ^
  :: Force routed FFN expert tensors to CPU/system RAM. This allows massive ^
  :: MoE models (like 35B) to run on 12GB GPUs by only holding active experts. ^
  :: Note: Adjust the regex if tensor naming differs on your specific quant. ^
  --override-tensor "\.ffn_.*_exps\.=CPU" ^
  ^
  :: --- Context & Memory --- ^
  :: Set context window size (e.g., 256K) ^
  --ctx-size 262144 ^
  :: Lock weights in RAM to prevent OS swap-induced latency spikes ^
  --mlock ^
  :: Concurrent request slots; each consumes extra KV cache memory ^
  --parallel "%PARALLEL_SLOTS%" ^
  ^
  :: --- MTP Speculative Decoding --- ^
  :: Use the same model for drafting (MTP leverages already-loaded target weights) ^
  --model-draft "%MODEL_PATH%" ^
  --spec-type draft-mtp ^
  --spec-draft-n-max 2 ^
  --spec-draft-n-min 1 ^
  --spec-draft-ngl 999 ^
  --spec-draft-override-tensor "\.ffn_.*_exps\.=CPU" ^
  ^
  :: --- TurboQuant KV Cache Compression --- ^
  :: turbo3 = 3-bit KV Cache (~4.3x memory savings vs F16). ^
  :: swap to turbo2 for max compression or turbo4 for max accuracy. ^
  --flash-attn on ^
  --cache-type-k turbo3 ^
  --cache-type-v turbo3 ^
  ^
  :: --- Inference Parameters --- ^
  --temp 0.6 ^
  --top-p 0.95 ^
  --top-k 20 ^
  --repeat-penalty 1.05
