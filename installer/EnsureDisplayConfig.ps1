$ErrorActionPreference = 'Stop'

if (Get-Module -ListAvailable -Name DisplayConfig | Select-Object -First 1) {
    Write-Host 'DisplayConfig is already installed.'
    exit 0
}

try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
} catch {
}

if (-not (Get-PackageProvider -Name NuGet -ErrorAction SilentlyContinue)) {
    Install-PackageProvider -Name NuGet -Scope CurrentUser -Force | Out-Null
}

$gallery = Get-PSRepository -Name PSGallery -ErrorAction SilentlyContinue
if ($gallery -and $gallery.InstallationPolicy -ne 'Trusted') {
    Set-PSRepository -Name PSGallery -InstallationPolicy Trusted
}

Install-Module -Name DisplayConfig -Scope CurrentUser -Force -AllowClobber -Repository PSGallery
Write-Host 'DisplayConfig was installed.'
