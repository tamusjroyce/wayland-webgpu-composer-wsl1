<#
.SYNOPSIS
  End-to-end visual test: run the compositor + a Wayland client inside the WSL1 distro,
  snapshot the composited framebuffer to a PNG (headless), and optionally open the live
  win-host window.

.EXAMPLE
  ./scripts/visual-test.ps1                    # snapshot to target/visual.png and open it
  ./scripts/visual-test.ps1 -Live              # also open the interactive WebGPU window
  ./scripts/visual-test.ps1 -Client weston-terminal
#>
[CmdletBinding()]
param(
  [string]$Distro = "WWC-WSL1",
  [int]$Port = 7900,
  [string]$Client = "weston-simple-shm",
  [int]$Seconds = 40,
  [switch]$Live
)
$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot
$winShm = Join-Path $repo "target\visual.fb"
$png = Join-Path $repo "target\visual.png"

# C:\a\b  ->  /mnt/c/a/b
$wslRepo = "/mnt/" + $repo.Substring(0, 1).ToLower() + ($repo.Substring(2) -replace '\\', '/')
$wslShm = "$wslRepo/target/visual.fb"
$script = "$wslRepo/scripts/wsl1-visual-test.sh"

Write-Host "Building fb-dump$(if ($Live) { ' + win-host' })..."
cargo build -q -p fb-dump
if ($Live) { cargo build -q -p win-host }

Write-Host "Starting compositor + $Client in $Distro (runs ~${Seconds}s)..."
# sed normalizes CRLF so the script runs regardless of checkout line endings.
$job = Start-Job -ScriptBlock {
  param($d, $s, $shm, $port, $client, $secs)
  wsl -d $d -u root -- bash -lc "sed -i 's/\r$//' '$s'; bash '$s' '$shm' '$port' '$client' '$secs'"
} -ArgumentList $Distro, $script, $wslShm, $Port, $Client, $Seconds

try {
  Start-Sleep -Seconds 6   # let the client connect and draw a few frames
  Write-Host "Snapshotting $winShm -> $png"
  cargo run -q -p fb-dump -- "$winShm" "$png"
  if (Test-Path $png) {
    Write-Host "Opening $png"
    Start-Process $png
  }
  if ($Live) {
    Write-Host "Launching win-host; close its window to finish."
    cargo run -q -p win-host -- --host "127.0.0.1:$Port"
  }
}
finally {
  Write-Host "Stopping WSL processes..."
  wsl -d $Distro -u root -- bash -lc "pkill -f wsl-compositor 2>/dev/null; pkill -f '$Client' 2>/dev/null; true" | Out-Null
  Stop-Job $job -ErrorAction SilentlyContinue | Out-Null
  Remove-Job $job -Force -ErrorAction SilentlyContinue | Out-Null
}
