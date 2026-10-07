@echo off
REM Fork build helper: MSVC + LLVM 18 + CUDA toolkit + Ninja.
REM Usage: build-cuda-env.bat [helper|libhelper|lib|bundle|check|test|fix]
REM   check = cargo check only (type-check, NO exe link) - safe to run while the
REM           app is still running (won't hit LNK1104 on the locked meetily.exe).
REM Needs Visual Studio 2022 (or its Build Tools) with C++, LLVM 18.1.8 and a
REM CUDA toolkit in CUDA_PATH (the CUDA installer sets it). Nothing here is
REM tied to one machine: every location comes from the environment or vswhere.
setlocal enabledelayedexpansion

set "ROOT=%~dp0"
set "REPO=%ROOT%.."

REM --- MSVC environment: the latest Visual Studio with the C++ tools ---
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" goto :no_vs
set "VS="
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VS=%%i"
if not defined VS goto :no_vs
call "%VS%\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
where cl.exe >nul 2>&1 || goto :no_vs

REM --- LLVM 18 for bindgen (LLVM 19 and later break whisper-rs-sys) ---
REM An explicit LIBCLANG_PATH wins, then the portable copy that
REM scripts\bootstrap-llvm18.ps1 unpacks, then the default LLVM install.
if defined LIBCLANG_PATH goto :llvm_found
set "LIBCLANG_PATH=%REPO%\.build-tools\clang+llvm-18.1.8-x86_64-pc-windows-msvc\bin"
if exist "%LIBCLANG_PATH%\libclang.dll" goto :llvm_found
set "LIBCLANG_PATH=%ProgramFiles%\LLVM\bin"
:llvm_found
if not exist "%LIBCLANG_PATH%\libclang.dll" goto :no_llvm
REM Bundled whisper-rs bindings are Linux-shaped; Windows must generate them
REM with the repository's pinned LLVM 18 toolchain.
set "WHISPER_DONT_GENERATE_BINDINGS="

REM --- CUDA toolkit, from CUDA_PATH ---
if not defined CUDA_PATH goto :no_cuda
if not exist "%CUDA_PATH%\bin\nvcc.exe" goto :no_cuda
set "CUDA_TOOLKIT_ROOT_DIR=%CUDA_PATH%"
set "CMAKE_GENERATOR=Ninja"
REM GPU architecture to compile for (89 = RTX 40 series); set it beforehand to override.
if not defined CMAKE_CUDA_ARCHITECTURES set "CMAKE_CUDA_ARCHITECTURES=89"
REM CUDA 13 CCCL (thrust/cub) needs C++17 + MSVC conforming preprocessor for .cu host compile
set "NVCC_APPEND_FLAGS=-std=c++17 -Xcompiler=/Zc:preprocessor -DCCCL_IGNORE_MSVC_TRADITIONAL_PREPROCESSOR_WARNING"
REM Ninja and CMake ship with the Visual Studio C++ CMake tools. Its Ninja goes
REM first (the CUDA build uses the Ninja generator), its CMake last, as a
REM fallback.
set "VS_CMAKE=%VS%\Common7\IDE\CommonExtensions\Microsoft\CMake"
set "PATH=%VS_CMAKE%\Ninja;%CUDA_PATH%\bin;%CUDA_PATH%\bin\x64;%USERPROFILE%\.cargo\bin;%PATH%;%VS_CMAKE%\CMake\bin"
set "TAURI_GPU_FEATURE=cuda"

echo === Build env ===
where cl.exe
where nvcc.exe
where cmake.exe
where ninja.exe
where cargo.exe
echo CUDA_PATH=%CUDA_PATH%
echo CMAKE_GENERATOR=%CMAKE_GENERATOR%  ARCH=%CMAKE_CUDA_ARCHITECTURES%
echo =================

set "MODE=%~1"
if "%MODE%"=="" set "MODE=lib"

if /I "%MODE%"=="helper"    goto :helper
if /I "%MODE%"=="libhelper" goto :helper
if /I "%MODE%"=="lib"       goto :lib
if /I "%MODE%"=="bundle"    goto :bundle
if /I "%MODE%"=="check"     goto :check
if /I "%MODE%"=="test"      goto :test
if /I "%MODE%"=="fix"       goto :fix
echo Unknown mode: %MODE%
exit /b 1

:helper
echo [helper] building llama-helper (CPU) release
cd /d "%REPO%\llama-helper"
cargo build --release
if errorlevel 1 ( echo [helper] build FAILED & exit /b 1 )
if not exist "%ROOT%src-tauri\binaries" mkdir "%ROOT%src-tauri\binaries"
copy /Y "%REPO%\target\release\llama-helper.exe" "%ROOT%src-tauri\binaries\llama-helper-x86_64-pc-windows-msvc.exe" >nul
if errorlevel 1 ( echo [helper] copy FAILED & exit /b 1 )
echo [helper] sidecar copied OK
if /I "%MODE%"=="libhelper" goto :lib
exit /b 0

:lib
echo [lib] building meetily --release --features cuda,custom-protocol
cd /d "%ROOT%src-tauri"
cargo build --release --features cuda,custom-protocol
if errorlevel 1 exit /b %errorlevel%
REM Stage bundled resources next to the exe. For a packaged build Tauri does
REM this itself, but a plain `cargo build` doesn't - and resource_dir() resolves
REM to the exe folder, so diarization models + templates must be copied here.
set "STAGE=%REPO%\target\release"
if not exist "%STAGE%\resources\diarization" mkdir "%STAGE%\resources\diarization"
copy /Y "%ROOT%src-tauri\resources\diarization\*" "%STAGE%\resources\diarization\" >nul
if not exist "%STAGE%\templates" mkdir "%STAGE%\templates"
copy /Y "%ROOT%src-tauri\templates\*.json" "%STAGE%\templates\" >nul
echo [lib] staged bundled resources (diarization models, templates)
exit /b 0

:check
echo [check] cargo check meetily --release --features cuda,custom-protocol (no exe link)
cd /d "%ROOT%src-tauri"
cargo check --release --features cuda,custom-protocol
exit /b %errorlevel%

:fix
REM Apply rustc's own suggested fixes (unused imports/variables etc.)
echo [fix] cargo fix --lib --release --features cuda,custom-protocol
cd /d "%ROOT%src-tauri"
cargo fix --lib -p meetily --release --features cuda,custom-protocol --allow-dirty --allow-staged
exit /b %errorlevel%

:test
REM Run a specific test with output shown, e.g.:
REM   build-cuda-env.bat test diarize_sample
echo [test] cargo test --release --features cuda %~2
cd /d "%ROOT%src-tauri"
cargo test --release --features cuda %~2 -- --nocapture --test-threads=1
exit /b %errorlevel%

:bundle
echo [bundle] Build CUDA app, NSIS overall-progress plugin, then Tauri bundle
cd /d "%ROOT%src-tauri"
cargo build --release -p meetily --bin meetily --no-default-features --features cuda,custom-protocol
if errorlevel 1 exit /b %errorlevel%
cd /d "%ROOT%"
powershell -NoProfile -ExecutionPolicy Bypass -File "scripts\build-nsis-progress-plugin.ps1" -ProgressMainBinary "..\target\release\meetily.exe"
if errorlevel 1 exit /b %errorlevel%
cd /d "%ROOT%"
call pnpm run tauri:build:cuda
exit /b %errorlevel%

:no_vs
1>&2 echo ERROR: Visual Studio 2022 with the C++ build tools was not found.
exit /b 1

:no_llvm
1>&2 echo ERROR: libclang.dll not found. Install LLVM 18.1.8 or set LIBCLANG_PATH to its bin folder.
exit /b 1

:no_cuda
1>&2 echo ERROR: CUDA_PATH is not set or has no bin\nvcc.exe. Install the CUDA toolkit or set CUDA_PATH to its folder.
exit /b 1
