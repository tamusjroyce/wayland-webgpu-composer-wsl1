# -----------------------------------------------------------------------------
# One-shot installer / updater / launcher for wayland-webgpu-composer.
#
# Does everything needed to see Linux apps in a WebGPU window on Windows:
#   1. Downloads/updates the Windows host binaries (win-host, fb-dump).
#   2. Ensures a WSL1 distro exists for the compositor (imports Ubuntu 22.04 on
#      first run).
#   3. Installs/updates the Linux compositor + a demo desktop inside that distro.
#   4. Launches the compositor (with a nested Weston desktop) and the WebGPU window.
#
# Re-run any time to update to the latest release and relaunch. Normally invoked
# through install.cmd, but can be run directly:
#   powershell -ExecutionPolicy Bypass -File install.ps1 [client-command]
#
# Optional first argument overrides the client shown inside the compositor, e.g.
#   install.cmd "gnome-calculator"
# -----------------------------------------------------------------------------
param(
    [string]$Client = 'weston --use-pixman --width=1280 --height=800'
)

$ErrorActionPreference = 'Stop'

$Repo      = 'tamusjroyce/wayland-webgpu-composer-wsl1'
$WinAsset  = 'wayland-webgpu-composer-windows-x64.zip'
$WslAsset  = 'wayland-webgpu-composer-wsl1-x64.tar.gz'
$WinUrl    = "https://github.com/$Repo/releases/latest/download/$WinAsset"
$WslUrl    = "https://github.com/$Repo/releases/latest/download/$WslAsset"
$RootfsUrl = 'https://cloud-images.ubuntu.com/wsl/jammy/current/ubuntu-jammy-wsl-amd64-ubuntu22.04lts.rootfs.tar.gz'
$Distro    = 'WWC-WSL1'
$Dest      = Join-Path $env:LOCALAPPDATA 'wayland-webgpu-composer'
$DistroDir = Join-Path $env:LOCALAPPDATA "WSL\$Distro"

Write-Host '=== wayland-webgpu-composer installer ==='

# 1. Windows binaries ---------------------------------------------------------
Write-Host '[1/5] Downloading Windows host...'
$tmpZip = Join-Path $env:TEMP $WinAsset
Invoke-WebRequest -Uri $WinUrl -OutFile $tmpZip
New-Item -ItemType Directory -Force $Dest | Out-Null
Expand-Archive -LiteralPath $tmpZip -DestinationPath $Dest -Force
Remove-Item $tmpZip -ErrorAction SilentlyContinue
Write-Host "      installed to $Dest"

# 2. WSL distro ---------------------------------------------------------------
if (-not (Get-Command wsl -ErrorAction SilentlyContinue)) {
    throw 'WSL is not installed. Install it with "wsl --install" and re-run.'
}
$distros = (& wsl -l -q) -replace "`0", '' | ForEach-Object { $_.Trim() } | Where-Object { $_ }
if ($distros -notcontains $Distro) {
    Write-Host "[2/5] Creating WSL1 distro '$Distro' (first run only; downloads Ubuntu rootfs)..."
    New-Item -ItemType Directory -Force $DistroDir | Out-Null
    $tmpRootfs = Join-Path $env:TEMP 'wwc-rootfs.tar.gz'
    Invoke-WebRequest -Uri $RootfsUrl -OutFile $tmpRootfs
    & wsl --import $Distro $DistroDir $tmpRootfs --version 1
    Remove-Item $tmpRootfs -ErrorAction SilentlyContinue
} else {
    Write-Host "[2/5] Using existing WSL distro '$Distro'"
}

# 3. Compositor install/update inside the distro ------------------------------
Write-Host "[3/5] Installing compositor into '$Distro'..."
$setup = @"
set -e
cd /tmp
curl -fL '$WslUrl' -o wwc.tgz
tar -xzf wwc.tgz
install -Dm755 wsl-compositor /usr/local/bin/wsl-compositor
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq || true
apt-get install -y libxkbcommon0 weston dmz-cursor-theme >/dev/null 2>&1 || true
rm -f wwc.tgz
"@ -replace "`r", ''
& wsl -d $Distro -u root -- bash -lc $setup

# 4. Launch the compositor ----------------------------------------------------
Write-Host '[4/5] Starting compositor...'
$fbWin = Join-Path $Dest 'fb\desktop.fb'
New-Item -ItemType Directory -Force (Split-Path $fbWin) | Out-Null
$fbWsl = (& wsl -d $Distro -u root -- wslpath -a "$fbWin").Trim()
# Kill any previous compositor, then start fresh so re-running relaunches cleanly.
$run = "pkill -9 -x wsl-compositor 2>/dev/null; mkdir -p /tmp/wwc-desk; " +
       "XDG_RUNTIME_DIR=/tmp/wwc-desk /usr/local/bin/wsl-compositor " +
       "--shm '$fbWsl' --width 1440 --height 900 -c '$Client'"
Start-Process wsl -ArgumentList @('-d', $Distro, '-u', 'root', '--', 'bash', '-lc', $run)

# 5. Launch the WebGPU window -------------------------------------------------
Write-Host '[5/5] Starting WebGPU window...'
Get-Process win-host -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Process (Join-Path $Dest 'win-host.exe')

Write-Host ''
Write-Host 'Done. The compositor (console) and the WebGPU window are running.'
Write-Host 'Re-run install.cmd any time to update to the latest release and relaunch.'
