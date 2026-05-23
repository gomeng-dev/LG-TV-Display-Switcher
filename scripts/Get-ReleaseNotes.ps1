param(
    [Parameter(Mandatory = $true)]
    [string] $Version,

    [string] $ChangelogPath = "CHANGELOG.md",
    [string] $OutputPath = "dist/release-notes.md"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $ChangelogPath)) {
    throw "Changelog not found: $ChangelogPath"
}

$currentTag = if ($Version.StartsWith("v")) { $Version } else { "v$Version" }
$plainVersion = $currentTag.TrimStart("v")
$headingNames = @($currentTag, $plainVersion)

$lines = Get-Content -LiteralPath $ChangelogPath -Encoding UTF8

function Get-ChangelogSection {
    param(
        [string[]] $Names,
        [string[]] $Lines
    )

    $start = $null
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i] -match '^##\s+\[?([^\]\r\n]+)\]?\s*$') {
            $name = $Matches[1].Trim()
            if ($Names -contains $name) {
                $start = $i + 1
                break
            }
        }
    }

    if ($null -eq $start) {
        return $null
    }

    $end = $Lines.Count
    for ($i = $start; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i] -match '^##\s+') {
            $end = $i
            break
        }
    }

    return ($Lines[$start..($end - 1)] -join "`n").Trim()
}

$notes = Get-ChangelogSection -Names $headingNames -Lines $lines
if ([string]::IsNullOrWhiteSpace($notes)) {
    $notes = Get-ChangelogSection -Names @("Unreleased") -Lines $lines
}

if ([string]::IsNullOrWhiteSpace($notes)) {
    throw "No changelog section found for $currentTag or Unreleased."
}

$previousTag = $null
try {
    $tags = @(git tag --sort=-v:refname)
    $currentIndex = [Array]::IndexOf($tags, $currentTag)
    if ($currentIndex -ge 0 -and ($currentIndex + 1) -lt $tags.Count) {
        $previousTag = $tags[$currentIndex + 1]
    } elseif ($tags.Count -gt 0) {
        $previousTag = $tags[0]
    }
} catch {
    $previousTag = $null
}

$body = New-Object System.Collections.Generic.List[string]
$body.Add("## Changes")
$body.Add("")

if ($previousTag) {
    $body.Add("Changes since ``$previousTag``.")
    $repository = $env:GITHUB_REPOSITORY
    if (-not [string]::IsNullOrWhiteSpace($repository)) {
        $body.Add("")
        $body.Add("[Compare $previousTag...$currentTag](https://github.com/$repository/compare/$previousTag...$currentTag)")
    }
    $body.Add("")
}

$body.Add($notes)

$outputDir = Split-Path -Parent $OutputPath
if (-not [string]::IsNullOrWhiteSpace($outputDir)) {
    New-Item -ItemType Directory -Force -Path $outputDir | Out-Null
}

Set-Content -LiteralPath $OutputPath -Encoding UTF8 -Value ($body -join "`n")
