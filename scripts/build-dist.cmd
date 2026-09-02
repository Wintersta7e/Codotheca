@echo off
REM Thin wrapper. All the logic lives in build-dist.mjs so the two platforms cannot drift apart.
setlocal

where node >nul 2>nul
if errorlevel 1 (
  echo build-dist: node is not on PATH; install Node 22.12.0 or newer 1>&2
  exit /b 1
)

node "%~dp0build-dist.mjs" %*
exit /b %errorlevel%
