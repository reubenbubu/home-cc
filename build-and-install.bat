@echo off
rem Builds home-cc on this PC and opens the installer it makes.
rem Double-click it, or run it from a terminal in the home-cc folder:
rem   build-and-install.bat             get the latest from GitHub, build, open the installer
rem   build-and-install.bat --no-pull   build what is in this folder now, without updating first
rem Needs Node.js, Rust and the Microsoft C++ Build Tools: see README.md, "Running from source".

setlocal
cd /d "%~dp0"

set "PULL=1"
if /i "%~1"=="--no-pull" set "PULL=0"

where node >nul 2>nul
if errorlevel 1 (
  echo Node.js is not installed, or not on the PATH. Install it from nodejs.org, then open a new window.
  goto :fail
)
where cargo >nul 2>nul
if errorlevel 1 (
  echo Rust is not installed, or not on the PATH. Install it from rustup.rs, then open a new window.
  goto :fail
)

if "%PULL%"=="0" goto :install
where git >nul 2>nul
if errorlevel 1 (
  echo Git is not installed, or not on the PATH, so the latest version can't be fetched.
  echo Run "build-and-install.bat --no-pull" to build what is here.
  goto :fail
)
echo.
echo === Getting the latest version from GitHub
git pull --ff-only
if errorlevel 1 (
  echo Couldn't update: you may have changes of your own in this folder.
  echo Run "build-and-install.bat --no-pull" to build what is here.
  goto :fail
)

:install
echo.
echo === Installing packages
call npm install --no-audit --no-fund
if errorlevel 1 goto :fail

echo.
echo === Building home-cc - the first build takes several minutes
call npm run tauri build
if errorlevel 1 goto :fail

rem Where Tauri puts the installers. CARGO_TARGET_DIR moves them, if it is set.
set "BUNDLE=%~dp0src-tauri\target\release\bundle"
if defined CARGO_TARGET_DIR set "BUNDLE=%CARGO_TARGET_DIR%\release\bundle"

rem The newest setup program, or else the newest .msi.
set "INSTALLER="
for /f "delims=" %%F in ('dir /b /o-d "%BUNDLE%\nsis\*-setup.exe" 2^>nul') do if not defined INSTALLER set "INSTALLER=%BUNDLE%\nsis\%%F"
if not defined INSTALLER for /f "delims=" %%F in ('dir /b /o-d "%BUNDLE%\msi\*.msi" 2^>nul') do if not defined INSTALLER set "INSTALLER=%BUNDLE%\msi\%%F"
if not defined INSTALLER (
  echo The build finished, but no installer was found in "%BUNDLE%".
  goto :fail
)

echo.
echo === Opening the installer: "%INSTALLER%"
start "" "%INSTALLER%"
endlocal
exit /b 0

:fail
echo.
echo Something went wrong: see the messages above.
pause
endlocal
exit /b 1
