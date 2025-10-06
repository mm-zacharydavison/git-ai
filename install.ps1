$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Write-ErrorAndExit {
    param(
        [Parameter(Mandatory = $true)][string]$Message
    )
    Write-Host "Error: $Message" -ForegroundColor Red
    exit 1
}

function Write-Success {
    param(
        [Parameter(Mandatory = $true)][string]$Message
    )
    Write-Host $Message -ForegroundColor Green
}

function Write-Warning {
    param(
        [Parameter(Mandatory = $true)][string]$Message
    )
    Write-Host $Message -ForegroundColor Yellow
}

# GitHub repository details
$Repo = 'acunniffe/git-ai'

# Ensure TLS 1.2 for GitHub downloads on older PowerShell versions
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
} catch { }

function Get-Architecture {
    try {
        $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
        switch ($arch) {
            'X64' { return 'x64' }
            'Arm64' { return 'arm64' }
            default { return $null }
        }
    } catch {
        $pa = $env:PROCESSOR_ARCHITECTURE
        if ($pa -match 'ARM64') { return 'arm64' }
        elseif ($pa -match '64') { return 'x64' }
        else { return $null }
    }
}

function Get-StdGitPath {
    $cmd = Get-Command git.exe -ErrorAction SilentlyContinue
    $gitPath = $null
    if ($cmd -and $cmd.Path) {
        # Ensure we never return a path for git that contains git-ai (recursive)
        if ($cmd.Path -notmatch "git-ai") {
            $gitPath = $cmd.Path
        }
    }

    if (-not $gitPath) {
        Write-ErrorAndExit "Could not detect a standard git binary on PATH. Please ensure you have Git installed and available on your PATH. If you believe this is a bug with the installer, please file an issue at https://github.com/acunniffe/git-ai/issues."
    }

    try {
        & $gitPath --version | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-ErrorAndExit "Detected git at $gitPath is not usable (--version failed). Please ensure you have Git installed and available on your PATH. If you believe this is a bug with the installer, please file an issue at https://github.com/acunniffe/git-ai/issues."
        }
    } catch {
        Write-ErrorAndExit "Detected git at $gitPath is not usable (--version failed). Please ensure you have Git installed and available on your PATH. If you believe this is a bug with the installer, please file an issue at https://github.com/acunniffe/git-ai/issues."
    }

    return $gitPath
}

# Ensure $PathToAdd is inserted before any PATH entry that contains "git" (case-insensitive)
# Updates Machine (system) PATH; if not elevated, emits a prominent error with instructions
function Set-PathPrependBeforeGit {
    param(
        [Parameter(Mandatory = $true)][string]$PathToAdd
    )

    $sep = ';'

    function NormalizePath([string]$p) {
        try { return ([IO.Path]::GetFullPath($p.Trim())).TrimEnd('\\').ToLowerInvariant() }
        catch { return ($p.Trim()).TrimEnd('\\').ToLowerInvariant() }
    }

    $normalizedAdd = NormalizePath $PathToAdd

    # Helper to build new PATH string with PathToAdd inserted before first 'git' entry
    function BuildPathWithInsert([string]$existingPath, [string]$toInsert) {
        $entries = @()
        if ($existingPath) { $entries = ($existingPath -split $sep) | Where-Object { $_ -and $_.Trim() -ne '' } }

        # De-duplicate and remove any existing instance of $toInsert
        $list = New-Object System.Collections.Generic.List[string]
        $seen = New-Object 'System.Collections.Generic.HashSet[string]'
        foreach ($e in $entries) {
            $n = NormalizePath $e
            if (-not $seen.Contains($n) -and $n -ne $normalizedAdd) {
                $seen.Add($n) | Out-Null
                $list.Add($e) | Out-Null
            }
        }

        # Find first index that matches 'git' anywhere (case-insensitive)
        $insertIndex = 0
        for ($i = 0; $i -lt $list.Count; $i++) {
            if ($list[$i] -match '(?i)git') { $insertIndex = $i; break }
        }

        $list.Insert($insertIndex, $toInsert)
        return ($list -join $sep)
    }

    # Try to update Machine PATH
    $updatedScope = $null
    try {
        $machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
        $newMachinePath = BuildPathWithInsert -existingPath $machinePath -toInsert $PathToAdd
        if ($newMachinePath -ne $machinePath) {
            [Environment]::SetEnvironmentVariable('Path', $newMachinePath, 'Machine')
            $updatedScope = 'Machine'
        } else {
            # Nothing changed at Machine scope; still treat as Machine for reporting
            $updatedScope = 'Machine'
        }
    } catch {
        # Access denied or not elevated; do NOT modify User PATH. Print big red error with instructions.
        $origGit = $null
        try { $origGit = Get-StdGitPath } catch { }
        $origGitDir = if ($origGit) { (Split-Path $origGit -Parent) } else { 'your Git installation directory' }
        Write-Host ''
        Write-Host 'ERROR: Unable to update the SYSTEM PATH (administrator rights required).' -ForegroundColor Red
        Write-Host 'Your PATH was NOT changed. To ensure git-ai takes precedence over Git:' -ForegroundColor Red
        Write-Host ("  1) Run PowerShell as Administrator and re-run this installer; OR") -ForegroundColor Red
        Write-Host ("  2) Manually edit the SYSTEM Path and move '{0}' before any entries containing 'Git' (e.g. '{1}')." -f $PathToAdd, $origGitDir) -ForegroundColor Red
        Write-Host "     Steps: Start → type 'Environment Variables' → 'Edit the system environment variables' → Environment Variables →" -ForegroundColor Red
        Write-Host "            Under 'System variables', select 'Path' → Edit → Move '{0}' to the top (before Git) → OK." -f $PathToAdd -ForegroundColor Red
        Write-Host ''
        $updatedScope = 'Error'
    }

    # Update current process PATH immediately for this session
    try {
        $procPath = $env:PATH
        $newProcPath = BuildPathWithInsert -existingPath $procPath -toInsert $PathToAdd
        if ($newProcPath -ne $procPath) { $env:PATH = $newProcPath }
    } catch { }

    return $updatedScope
}

# Detect standard Git early and validate (fail-fast behavior)
$stdGitPath = Get-StdGitPath

# Detect architecture and OS
$arch = Get-Architecture
if (-not $arch) { Write-ErrorAndExit "Unsupported architecture: $([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)" }
$os = 'windows'

# Determine binary name and download URLs
$binaryName = "git-ai-$os-$arch"
$downloadUrlExe = "https://github.com/$Repo/releases/latest/download/$binaryName.exe"
$downloadUrlNoExt = "https://github.com/$Repo/releases/latest/download/$binaryName"

# Install directory: %USERPROFILE%\.git-ai\bin
$installDir = Join-Path $HOME ".git-ai\bin"
New-Item -ItemType Directory -Force -Path $installDir | Out-Null

Write-Host 'Downloading git-ai...'
$tmpFile = Join-Path $installDir "git-ai.tmp.$PID.exe"

function Try-Download {
    param(
        [Parameter(Mandatory = $true)][string]$Url
    )
    try {
        Invoke-WebRequest -Uri $Url -OutFile $tmpFile -UseBasicParsing -ErrorAction Stop
        return $true
    } catch {
        return $false
    }
}

$downloaded = $false
if (Try-Download -Url $downloadUrlExe) { $downloaded = $true }
elseif (Try-Download -Url $downloadUrlNoExt) { $downloaded = $true }

if (-not $downloaded) {
    Remove-Item -Force -ErrorAction SilentlyContinue $tmpFile
    Write-ErrorAndExit 'Failed to download binary (HTTP error)'
}

try {
    if ((Get-Item $tmpFile).Length -le 0) {
        Remove-Item -Force -ErrorAction SilentlyContinue $tmpFile
        Write-ErrorAndExit 'Downloaded file is empty'
    }
} catch {
    Remove-Item -Force -ErrorAction SilentlyContinue $tmpFile
    Write-ErrorAndExit 'Download failed'
}

$finalExe = Join-Path $installDir 'git-ai.exe'
Move-Item -Force -Path $tmpFile -Destination $finalExe
try { Unblock-File -Path $finalExe -ErrorAction SilentlyContinue } catch { }

# Create a shim so calling `git` goes through git-ai by PATH precedence
$gitShim = Join-Path $installDir 'git.exe'
Copy-Item -Force -Path $finalExe -Destination $gitShim
try { Unblock-File -Path $gitShim -ErrorAction SilentlyContinue } catch { }

# Create a shim so calling `git-og` invokes the standard Git
$gitOgShim = Join-Path $installDir 'git-og.cmd'
$gitOgShimContent = "@echo off`r`n\"$stdGitPath\" %*`r`n"
Set-Content -Path $gitOgShim -Value $gitOgShimContent -Encoding ASCII
try { Unblock-File -Path $gitOgShim -ErrorAction SilentlyContinue } catch { }

# Install hooks
Write-Host 'Setting up IDE/agent hooks...'
try {
    & $finalExe install-hooks | Out-Host
    Write-Success 'Successfully set up IDE/agent hooks'
} catch {
    Write-Warning "Warning: Failed to set up IDE/agent hooks. Please try running 'git-ai install-hooks' manually."
}

# Update PATH so our shim takes precedence over any Git entries
$scope = Set-PathPrependBeforeGit -PathToAdd $installDir
if ($scope -eq 'Machine') {
    Write-Success 'Successfully added git-ai to the system PATH.'
} elseif ($scope -eq 'Error') {
    Write-Host 'PATH update failed: system PATH unchanged.' -ForegroundColor Red
}

Write-Success "Successfully installed git-ai into $installDir"
Write-Success "You can now run 'git-ai' from your terminal"

# Write JSON config at %USERPROFILE%\.git-ai\config.json
try {
    $configDir = Join-Path $HOME '.git-ai'
    $configJsonPath = Join-Path $configDir 'config.json'
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null

    $cfg = @{ git_path = $stdGitPath; ignore_prompts = $false } | ConvertTo-Json -Compress
    $cfg | Out-File -FilePath $configJsonPath -Encoding UTF8 -Force
} catch {
    Write-Host "Warning: Failed to write config.json: $($_.Exception.Message)" -ForegroundColor Yellow
}