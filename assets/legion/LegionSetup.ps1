# One-time setup for V-Notch's Legion tab, started elevated (UAC) from the notch.
# Copies the helper to Program Files (only administrators can change it there, since it runs
# elevated) and registers it as a logon task with highest privileges, then starts it.
param([Parameter(Mandatory = $true)][string]$User)

$ErrorActionPreference = 'Stop'
$sid = New-Object System.Security.Principal.SecurityIdentifier($User)
$account = $sid.Translate([System.Security.Principal.NTAccount]).Value
$dir = Join-Path $env:ProgramFiles 'V-Notch Legion'
New-Item -ItemType Directory -Force $dir | Out-Null
$helper = Join-Path $dir 'LegionHelper.ps1'
Copy-Item (Join-Path $PSScriptRoot 'LegionHelper.ps1') $helper -Force

$powershell = Join-Path $env:WINDIR 'System32\WindowsPowerShell\v1.0\powershell.exe'
# conhost --headless keeps the console window from flashing up at every logon.
$action = New-ScheduledTaskAction -Execute (Join-Path $env:WINDIR 'System32\conhost.exe') `
    -Argument "--headless `"$powershell`" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$helper`" -User $User"
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $account
$principal = New-ScheduledTaskPrincipal -UserId $account -LogonType Interactive -RunLevel Highest
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew -StartWhenAvailable
Register-ScheduledTask -TaskName 'V-Notch Legion' -Action $action -Trigger $trigger -Principal $principal `
    -Settings $settings -Force | Out-Null
# An older helper still holds the pipe: end it so the new copy takes over.
Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" |
    Where-Object CommandLine -like '*LegionHelper.ps1*' | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
Start-ScheduledTask -TaskName 'V-Notch Legion'
