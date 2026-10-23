@echo off
REM Wrapper script for locked Cargo commands and auditable release builds.
REM
REM Set `CARGO_LOCKED=1` to require an up-to-date `Cargo.lock`.
REM Cargo extensions still need their own `--locked` flags.
REM Set `CARGO_AUDITABLE=0` to disable SBOM embedding outside release builds.
REM Use `scripts/install-cargo-extensions.sh` to install cargo-auditable.
REM Select a Rust toolchain with `RUSTUP_TOOLCHAIN`.
REM
REM Usage:
REM
REM   set CARGO=%CD%\scripts\cargo.cmd
REM   maturin build --release

setlocal DisableDelayedExpansion
set "locked="
if "%CARGO_LOCKED%"=="1" set "locked=--locked"
set "auditable=auditable"
if "%CARGO_AUDITABLE%"=="0" set "auditable="
REM Only inspect global options; aliases can forward later arguments to a program.
:scan
if "%~1"=="" goto invoke
if "%~1"=="--" goto invoke
set "argument=%~1"
if not "%argument:~0,1%"=="-" goto invoke
if "%~1"=="--locked" set "locked="
if "%~1"=="--frozen" set "locked="
if "%~1"=="--version" set "auditable="
if "%~1"=="-V" set "auditable="
if "%~1"=="--color" goto skip_value
if "%~1"=="--config" goto skip_value
if "%~1"=="-C" goto skip_value
if "%~1"=="-Z" goto skip_value
shift
goto scan

:skip_value
shift
shift
goto scan

:invoke
REM Metadata and version queries do not compile artifacts to embed an SBOM in.
if "%~1"=="metadata" set "auditable="

if defined REAL_CARGO (
    "%REAL_CARGO%" %auditable% %locked% %*
) else (
    cargo.exe %auditable% %locked% %*
)
exit /b %errorlevel%
