# Third-party notices

Otto bundles the models and libraries below. Each keeps its own license.

## NVIDIA Parakeet TDT 0.6B v2

- Source: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2
- Copyright: NVIDIA Corporation
- License: Creative Commons Attribution 4.0 International (CC-BY-4.0), https://creativecommons.org/licenses/by/4.0/
- Changes: Otto ships `parakeet-tdt-0.6b-v2-Q8_0.gguf`, a GGUF conversion with 8-bit (Q8_0) quantized weights published by handy-computer at https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v2-gguf. Otto does not change the file.

## transcribe.cpp

- Source: https://github.com/handy-computer/transcribe.cpp (Rust crates `transcribe-cpp` and `transcribe-cpp-sys`)
- License: MIT
- transcribe.cpp builds on ggml (https://github.com/ggml-org/ggml), also MIT.

## Silero VAD v6.2

- Source: https://github.com/snakers4/silero-vad
- Copyright: Silero Team
- License: MIT
- Changes: Otto ships `silero-vad-v6.2-F32.gguf`, a GGUF conversion published by handy-computer at https://huggingface.co/handy-computer/silero-vad-v6.2-gguf. Otto does not change the file.

## Microsoft Visual C++ runtime (Windows installer only)

The Windows installer carries `msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll` and `vcomp140.dll` from the Visual C++ Redistributable, which Microsoft lets developers ship with their apps under the Visual Studio license terms.
