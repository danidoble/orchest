[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:ProgramFiles 'Orchest'),
    [switch]$SkipVCRedist
)

$ErrorActionPreference = 'Stop'
$sourceDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$cli = Join-Path $sourceDir 'orchest.exe'
$api = Join-Path $sourceDir 'orchest-api.exe'
if (-not (Test-Path -LiteralPath $cli -PathType Leaf) -or -not (Test-Path -LiteralPath $api -PathType Leaf)) {
    throw 'orchest.exe and orchest-api.exe must be next to install-windows.ps1'
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run PowerShell as Administrator to install into Program Files and update the machine PATH.'
}

if (-not $SkipVCRedist) {
    $redist = Join-Path $env:TEMP ("orchest-vc-redist-$([guid]::NewGuid()).exe")
    try {
        Invoke-WebRequest -Uri 'https://aka.ms/vc14/vc_redist.x64.exe' -OutFile $redist
        $signature = Get-AuthenticodeSignature -LiteralPath $redist
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft Corporation') {
            throw 'The Visual C++ Redistributable does not have a valid Microsoft signature.'
        }
        $process = Start-Process -FilePath $redist -ArgumentList '/install', '/quiet', '/norestart' -Wait -PassThru
        if ($process.ExitCode -notin @(0, 3010, 1638)) {
            throw "Visual C++ Redistributable installation failed with exit code $($process.ExitCode)."
        }
        if ($process.ExitCode -eq 3010) {
            Write-Warning 'Windows requested a restart to complete the Visual C++ Redistributable installation.'
        }
    }
    finally {
        Remove-Item -LiteralPath $redist -Force -ErrorAction SilentlyContinue
    }
}

New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Copy-Item -LiteralPath $cli -Destination (Join-Path $InstallDir 'orchest.exe') -Force
Copy-Item -LiteralPath $api -Destination (Join-Path $InstallDir 'orchest-api.exe') -Force

$currentPath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
$entries = @($currentPath -split ';' | Where-Object { $_ })
if (-not ($entries | Where-Object { $_.TrimEnd('\') -ieq $InstallDir.TrimEnd('\') })) {
    [Environment]::SetEnvironmentVariable('Path', (($entries + $InstallDir) -join ';'), 'Machine')
}

Write-Host "Installed Orchest commands in $InstallDir"
Write-Host 'Open a new terminal as your normal user, then run: orchest init'
