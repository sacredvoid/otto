# Dot-source before building the "optimized" Windows baseline:  . .\bench\windows-env.ps1
# Why: cmake-rs 0.1.58 with the Visual Studio generator overwrites
# CMAKE_{C,CXX}_FLAGS_RELEASE with opt_level(0) flags, so whisper.cpp is compiled at /Od
# (about 10x slower). Setting these env vars makes cmake-rs keep our flags. Use -MD, not /MD
# (Git Bash mangles /MD). Do NOT put these in .cargo/config.toml: they would break macOS.
$env:CMAKE_C_FLAGS_RELEASE   = "-MD -O2 -Ob2 -DNDEBUG"
$env:CMAKE_CXX_FLAGS_RELEASE = "-MD -O2 -Ob2 -DNDEBUG -utf-8"
# Point CARGO_TARGET_DIR at a folder of its own if you also build without these flags.
