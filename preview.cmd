@echo off
rem Run the PowerShell tools without the execution policy getting in the way.
rem
rem Windows refuses to run an unsigned .ps1 from a console whose policy is
rem Restricted, which is the default on a fresh install. The policy is a property of
rem the shell, not of the script, so it is set for this one invocation and nothing on
rem the machine is changed.
setlocal
set "SCRIPT=%~dp0tools\build-preview.ps1"
if not exist "%SCRIPT%" (
    echo cannot find %SCRIPT%
    exit /b 1
)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%SCRIPT%" %*
exit /b %ERRORLEVEL%
