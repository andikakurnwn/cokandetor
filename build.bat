@echo off
setlocal EnableDelayedExpansion
rem ------------------------------------------------------------------
rem  Cokandetor build helper
rem  Finds a Visual Studio install that has the full x64 C++ toolset
rem  (some machines have a partial VS that lacks msvcrt.lib), loads its
rem  environment and then runs cargo.
rem
rem  Usage:  build.bat            -> release build
rem          build.bat debug      -> debug build
rem          build.bat run        -> release build + run
rem          build.bat test       -> unit tests
rem          build.bat clippy     -> lints
rem ------------------------------------------------------------------

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "VSPATH="
if not exist "!VSWHERE!" goto :novs
for /f "usebackq tokens=*" %%i in (`"!VSWHERE!" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSPATH=%%i"
if not defined VSPATH goto :novs
if not exist "!VSPATH!\VC\Auxiliary\Build\vcvars64.bat" goto :novs
echo [cokandetor] Using MSVC from: !VSPATH!
call "!VSPATH!\VC\Auxiliary\Build\vcvars64.bat" >nul
goto :build

:novs
echo [cokandetor] No VS install with the C++ x64 toolset found via vswhere.
echo [cokandetor] Trying cargo with the default environment...

:build
where cargo >nul 2>nul || set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

if /i "%~1"=="debug" goto :debug
if /i "%~1"=="run" goto :run
if /i "%~1"=="test" goto :test
if /i "%~1"=="clippy" goto :clippy
cargo build --release
exit /b !errorlevel!

:clippy
cargo clippy --all-targets
exit /b !errorlevel!

:test
cargo test
exit /b !errorlevel!

:debug
cargo build
exit /b !errorlevel!

:run
cargo run --release
exit /b !errorlevel!
