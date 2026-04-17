param(
  [Parameter(Mandatory = $true)]
  [string]$StackName,
  [Parameter(Mandatory = $false)]
  [string]$Region = "us-east-1",
  [Parameter(Mandatory = $false)]
  [string]$Distro = "Ubuntu"
)

$ErrorActionPreference = "Stop"

function Convert-ToWslPath {
  param([Parameter(Mandatory = $true)][string]$WindowsPath)

  $resolved = (Resolve-Path -LiteralPath $WindowsPath).Path
  $drive = $resolved.Substring(0, 1).ToLowerInvariant()
  $rest = $resolved.Substring(2).Replace('\', '/')
  return "/mnt/$drive$rest"
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$safeScriptWin = Join-Path $scriptDir "status-snapshot-safe.sh"
$safeScriptWsl = Convert-ToWslPath -WindowsPath $safeScriptWin

Write-Host "Running status snapshot in WSL distro '$Distro'..."
Write-Host "Stack:  $StackName"
Write-Host "Region: $Region"
Write-Host "Script: $safeScriptWsl"

$bashCmd = "set -euo pipefail; bash '$safeScriptWsl' '$StackName' '$Region'"
wsl.exe -d $Distro --exec bash -lc $bashCmd

