$scriptPath = Join-Path (Split-Path -Parent $PSScriptRoot) "run-desktop.sh"
if (-not (Test-Path -LiteralPath $scriptPath -PathType Leaf)) {
    throw "Could not find run-desktop.sh at '$scriptPath'."
}

$listing = (& wsl.exe --list --verbose 2>$null | Out-String) -replace "`0", ""
if ($LASTEXITCODE -ne 0) {
    throw "Could not list WSL distributions with 'wsl.exe --list --verbose'."
}

$distros = @(
    foreach ($line in ($listing -split "`r?`n")) {
        if ($line -match '^\s*\*?\s*(.+?)\s{2,}\S+\s+([12])\s*$' -and $Matches[2] -eq "1") {
            $Matches[1].Trim()
        }
    }
)

if ($distros.Count -eq 0) {
    throw "No WSL1 distributions found. This launcher requires WSL1."
}

Write-Host "Available WSL1 distributions:"
for ($i = 0; $i -lt $distros.Count; $i++) {
    Write-Host ("  {0}) {1}" -f ($i + 1), $distros[$i])
}

$selection = Read-Host "Choose a distribution"
$index = 0
if (-not [int]::TryParse($selection, [ref]$index) -or $index -lt 1 -or $index -gt $distros.Count) {
    throw "Invalid distribution selection."
}
$distro = $distros[$index - 1]

$desktopProbe = 'command -v sway >/dev/null 2>&1 && echo sway; command -v labwc >/dev/null 2>&1 && echo labwc; command -v weston >/dev/null 2>&1 && echo weston; command -v cage >/dev/null 2>&1 && echo cage; command -v kwin_wayland >/dev/null 2>&1 && echo kwin; command -v wayfire >/dev/null 2>&1 && echo wayfire; true'
$desktopListing = (& wsl.exe --distribution $distro --user root -- bash -lc $desktopProbe 2>$null | Out-String)
if ($LASTEXITCODE -ne 0) {
    throw "Could not check installed desktops in '$distro'."
}
$desktops = @($desktopListing -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ })
if ($desktops.Count -eq 0) {
    throw "No supported desktops found in '$distro'. Install sway, weston, labwc, cage, kwin, or wayfire first."
}

Write-Host "Available desktops in '$distro':"
for ($i = 0; $i -lt $desktops.Count; $i++) {
    Write-Host ("  {0}) {1}" -f ($i + 1), $desktops[$i])
}

$selection = Read-Host "Choose a desktop"
$index = 0
if (-not [int]::TryParse($selection, [ref]$index) -or $index -lt 1 -or $index -gt $desktops.Count) {
    throw "Invalid desktop selection."
}
$desktop = $desktops[$index - 1]

$windowsPath = [System.IO.Path]::GetFullPath($scriptPath)
if ($windowsPath -notmatch '^([A-Za-z]):\\(.*)$') {
    throw "Expected a drive-letter workspace path, got '$windowsPath'."
}
$drive = $Matches[1].ToLowerInvariant()
$relativePath = $Matches[2] -replace '\\', '/'
$wslScriptPath = "/mnt/$drive/$relativePath"
$wslWorkspacePath = $wslScriptPath.Substring(0, $wslScriptPath.LastIndexOf('/'))
$framebufferPath = "$wslWorkspacePath/target/desktop.fb"
$framebufferDirectory = Join-Path (Split-Path -Parent $scriptPath) "target"
if (-not (Test-Path -LiteralPath $framebufferDirectory -PathType Container)) {
    throw "Workspace target directory not found at '$framebufferDirectory'. Build the project first."
}

Write-Host "Starting '$desktop' via run-desktop.sh in '$distro'."
& wsl.exe --distribution $distro --user root -- env "WWC_FB=$framebufferPath" bash -x $wslScriptPath $desktop
if ($LASTEXITCODE -ne 0) {
    throw "run-desktop.sh exited with status $LASTEXITCODE."
}
