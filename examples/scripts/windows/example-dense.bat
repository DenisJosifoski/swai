@echo off
setlocal

:: ==============================================================================
:: EXAMPLE: Standard Dense Model with TurboQuant (AtomicBot Fork)
:: 
:: PLATFORM: Windows
:: 
:: This script demonstrates how to run a standard dense model fully offloaded.
:: Note the use of ^ for line continuation in Windows batch scripts.
:: ==============================================================================

:: 1. Define paths (Update these paths to match your actual files and directories)
if not defined MODEL_PATH set MODEL_PATH=C:\ai-models\Dense-Model-9B-Q6_K.gguf
if not defined LLAMA_SERVER set LLAMA_SERVER=C:\atomic-llama-cpp-turboquant\build\bin\Release\llama-server.exe

:: 2. Network & UI variables (SWAI will override PORT automatically)
set "PORT=8080"
set "PARALLEL_SLOTS=1"

llama-server.exe ^
  --model "%MODEL_PATH%" ^
  --port %PORT% ^
  --host 127.0.0.1 ^
  ^
  :: CRITICAL FOR SWAI: The --slots flag enables the /slots endpoint ^
  :: which SWAI uses to monitor the context window progress. ^
  --slots ^
  ^
  :: --- Hardware Offloading --- ^
  :: For a dense model that fits in VRAM, 999 tells llama.cpp to push ^
  :: all layers to the GPU. ^
  --n-gpu-layers 999 ^
  ^
  :: --- Context & Memory --- ^
  :: Set context window size (e.g., 32K) ^
  --ctx-size 32768 ^
  :: Lock weights in RAM to prevent OS swap-induced latency spikes ^
  --mlock ^
  :: Concurrent request slots; each consumes extra KV cache memory ^
  --parallel "%PARALLEL_SLOTS%" ^
  ^
  :: --- TurboQuant KV Cache Compression --- ^
  :: turbo4 = 4-bit KV Cache (excellent quality retention for dense models). ^
  --flash-attn on ^
  --cache-type-k turbo4 ^
  --cache-type-v turbo4 ^
  ^
  :: --- Inference Parameters --- ^
  --temp 0.8 ^
  --top-p 0.90 ^
  --top-k 40 ^
  --repeat-penalty 1.10
