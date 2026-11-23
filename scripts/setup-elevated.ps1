<#
.SYNOPSIS
Installs, updates, checks, or removes the elevated Scheduled Task for vscroll-wm.

.DESCRIPTION
Copies vscroll-wm.exe to an admin-protected Program Files directory and registers a
Windows Scheduled Task ("Run with highest privileges") that starts automatically at
logon without a UAC prompt, allowing vscroll-wm to manage administrator windows
(Task Manager, elevated terminals, installers, etc.) without code-signing.
#>
[CmdletBinding()]
param(
  [ValidateSet("Install", "Uninstall", "Status")]
  [string]$Action = "Install",
  [string]$SourceExe = "",
  [string]$TargetDir = "C:\Program Files\vscroll-wm",
  [string]$ConfigDir = "",
  [string]$TaskName = "vscroll-wm",
  [string]$UserSid = "",
  [switch]$SelfElevate,
  [switch]$NoStart
)

$ErrorActionPreference = "Stop"

function Test-IsAdmin {
  $id = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = New-Object Security.Principal.WindowsPrincipal($id)
  return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Resolve-TargetUserSid {
  param([string]$ExplicitSid)
  if (-not [string]::IsNullOrWhiteSpace($ExplicitSid)) {
    return $ExplicitSid.Trim()
  }
  return [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
}

function Resolve-SourceExe {
  param([string]$ExplicitPath)
  if (-not [string]::IsNullOrWhiteSpace($ExplicitPath)) {
    if (-not (Test-Path -LiteralPath $ExplicitPath -PathType Leaf)) {
      throw "Source binary not found: $ExplicitPath"
    }
    return (Resolve-Path -LiteralPath $ExplicitPath).Path
  }
  $root = Split-Path -Parent $PSScriptRoot
  $candidates = @(
    (Join-Path $root "target\release\vscroll-wm.exe"),
    (Join-Path $root "target\debug\vscroll-wm.exe")
  )
  foreach ($c in $candidates) {
    if (Test-Path -LiteralPath $c -PathType Leaf) {
      return $c
    }
  }
  throw "Could not find vscroll-wm.exe. Build it first (cargo build --release) or pass -SourceExe."
}

function Resolve-ConfigDir {
  param([string]$ExplicitDir)
  if (-not [string]::IsNullOrWhiteSpace($ExplicitDir)) {
    return (Resolve-Path -LiteralPath $ExplicitDir).Path
  }
  return (Split-Path -Parent $PSScriptRoot)
}

function Escape-Xml {
  param([string]$Value)
  return [System.Security.SecurityElement]::Escape($Value)
}

$targetExe = Join-Path $TargetDir "vscroll-wm.exe"

if ($Action -eq "Status") {
  $exeOk = Test-Path -LiteralPath $targetExe -PathType Leaf
  $taskOk = $false
  try {
    & schtasks.exe /Query /TN $TaskName *> $null
    $taskOk = ($LASTEXITCODE -eq 0)
  } catch {
    $taskOk = $false
  }
  [PSCustomObject]@{
    TaskName  = $TaskName
    TaskReady = $taskOk
    TargetExe = $targetExe
    ExeExists = $exeOk
  }
  exit ($(if ($exeOk -and $taskOk) { 0 } else { 1 }))
}

if (-not (Test-IsAdmin)) {
  if ($SelfElevate) {
    $resolvedSid = Resolve-TargetUserSid -ExplicitSid $UserSid
    $argList = @(
      "-NoProfile",
      "-ExecutionPolicy", "Bypass",
      "-File", "`"$PSCommandPath`"",
      "-Action", $Action,
      "-TargetDir", "`"$TargetDir`"",
      "-TaskName", "`"$TaskName`"",
      "-UserSid", "`"$resolvedSid`""
    )
    if ($Action -eq "Install") {
      $resolvedSrc = Resolve-SourceExe -ExplicitPath $SourceExe
      $resolvedCfg = Resolve-ConfigDir -ExplicitDir $ConfigDir
      $argList += @("-SourceExe", "`"$resolvedSrc`"", "-ConfigDir", "`"$resolvedCfg`"")
      if ($NoStart) {
        $argList += "-NoStart"
      }
    }
    $proc = Start-Process -FilePath "powershell.exe" -Verb RunAs -Wait -PassThru -ArgumentList $argList
    exit $proc.ExitCode
  }
  throw "Administrator privileges are required for Action=$Action. Re-run as Administrator or pass -SelfElevate."
}

$sid = Resolve-TargetUserSid -ExplicitSid $UserSid

if ($Action -eq "Uninstall") {
  try {
    & schtasks.exe /End /TN $TaskName *> $null
  } catch {}
  try {
    & schtasks.exe /Delete /TN $TaskName /F *> $null
  } catch {}
  if (Test-Path -LiteralPath $targetExe -PathType Leaf) {
    Remove-Item -LiteralPath $targetExe -Force
  }
  if ((Test-Path -LiteralPath $TargetDir -PathType Container) -and
      ((Get-ChildItem -LiteralPath $TargetDir -Force | Measure-Object).Count -eq 0)) {
    Remove-Item -LiteralPath $TargetDir -Force
  }
  Write-Output "Removed scheduled task '$TaskName' and '$targetExe'."
  exit 0
}

$src = Resolve-SourceExe -ExplicitPath $SourceExe
$cfgDir = Resolve-ConfigDir -ExplicitDir $ConfigDir

# Stop any running vscroll-wm instance so replacing the binary and starting fresh succeeds cleanly.
try {
  & schtasks.exe /End /TN $TaskName *> $null
} catch {}
Get-Process -Name "vscroll-wm" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 150

New-Item -ItemType Directory -Force -Path $TargetDir | Out-Null
Copy-Item -LiteralPath $src -Destination $targetExe -Force

# Remove legacy HKCU\...\Run autostart entry so Windows doesn't launch a duplicate non-admin instance.
Remove-ItemProperty -Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name "vscroll-wm" -ErrorAction SilentlyContinue

$escapedExe = Escape-Xml $targetExe
$escapedCfg = Escape-Xml $cfgDir
$escapedSid = Escape-Xml $sid
$taskXml = @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.3" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Elevated vscroll-wm tiling window manager (starts at logon without UAC prompt).</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>$escapedSid</UserId>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>$escapedSid</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <DisallowStartOnRemoteAppSession>false</DisallowStartOnRemoteAppSession>
    <UseUnifiedSchedulingEngine>true</UseUnifiedSchedulingEngine>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>$escapedExe</Command>
      <WorkingDirectory>$escapedCfg</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"@

# Grant Full Access to Administrators & SYSTEM, and Read+Execute only to the target user SID
# so non-admin tools/shortcuts can trigger the task on demand without a UAC prompt.
$sddl = "D:(A;;FA;;;BA)(A;;FA;;;SY)(A;;GRGX;;;$sid)"

$svc = New-Object -ComObject Schedule.Service
$svc.Connect()
$folder = $svc.GetFolder("\")
$TASK_CREATE_OR_UPDATE = 6
$TASK_LOGON_INTERACTIVE_TOKEN = 3
$null = $folder.RegisterTask($TaskName, $taskXml, $TASK_CREATE_OR_UPDATE, $sid, $null, $TASK_LOGON_INTERACTIVE_TOKEN, $sddl)

Write-Output "Installed elevated logon task '$TaskName' -> '$targetExe' (config dir: '$cfgDir')."

if (-not $NoStart) {
  & schtasks.exe /Run /TN $TaskName *> $null
  Write-Output "Started '$TaskName' elevated."
}
