@echo off
rem Startet winrm-einrichten.ps1 mit Administratorrechten (Execution Policy nur fuer diesen Aufruf umgangen).
rem Aufruf: Rechtsklick - "Als Administrator ausfuehren". Weitere Parameter werden durchgereicht,
rem z. B.:  winrm-einrichten.bat -NurVon 192.0.2.10 -DnsNamen ziel.example.test
rem Rueckgaengig:  winrm-einrichten.bat -Rueckgaengig
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo Bitte als Administrator ausfuehren: Rechtsklick auf die Datei, "Als Administrator ausfuehren".
    pause
    exit /b 1
)
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0winrm-einrichten.ps1" %*
pause
