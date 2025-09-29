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
    if ($cmd -and $cmd.Path) {
        if ($cmd.Path -notmatch "\\\.git-ai\\") {
            return $cmd.Path
        }
    }
    return $null
}

function Add-ToUserPath {
    param(
        [Parameter(Mandatory = $true)][string]$PathToAdd
    )
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $sep = ';'
    $entries = @()
    if ($current) { $entries = ($current -split $sep) | Where-Object { $_ -and $_.Trim() -ne '' } }

    $exists = $false
    foreach ($entry in $entries) {
        try {
            if ([IO.Path]::GetFullPath($entry.TrimEnd('\')) -ieq [IO.Path]::GetFullPath($PathToAdd.TrimEnd('\'))) {
                $exists = $true
                break
            }
        } catch { }
    }

    if (-not $exists) {
        $newPath = if ($current) { ($current.TrimEnd($sep) + $sep + $PathToAdd) } else { $PathToAdd }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        return $true
    }
    return $false
}

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

# Detect standard Git path to avoid recursion and set env var
$stdGitPath = Get-StdGitPath
if ($stdGitPath) {
    [Environment]::SetEnvironmentVariable('GIT_AI_GIT_PATH', $stdGitPath, 'User')
    $env:GIT_AI_GIT_PATH = $stdGitPath
}

# TODO Install hooks
# Write-Host 'Setting up IDE/agent hooks...'
# try {
#     & $finalExe install-hooks | Out-Host
#     Write-Success 'Successfully set up IDE/agent hooks'
# } catch {
#     Write-Host 'Warning: Failed to set up IDE/agent hooks; continuing without IDE/agent hooks.' -ForegroundColor Yellow
# }

Write-Success "Successfully installed git-ai into $installDir"
Write-Success "You can now run 'git-ai' from your terminal"

# Update PATH for the user if needed
$added = Add-ToUserPath -PathToAdd $installDir
if ($added) {
    if ($env:PATH -notmatch [Regex]::Escape($installDir)) {
        $env:PATH = "$installDir;" + $env:PATH
    }
    Write-Success "Updated your user PATH to include $installDir"
    Write-Host 'Restart your terminal for the change to take effect.'
}


