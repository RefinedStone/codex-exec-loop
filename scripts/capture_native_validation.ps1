[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Frontend,

    [string]$Terminal,
    [string]$Shell,
    [string]$Term = $env:TERM,
    [string]$Result = "pending",
    [string]$Notes = "",
    [string]$Commit,
    [string]$Os,
    [string]$Date,
    [string]$OutputPath,
    [string]$OutputDir
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Get-DetectedTerminal {
    if ($Terminal) {
        return $Terminal
    }

    if ($env:WT_SESSION) {
        return "Windows Terminal"
    }

    if ($env:TERM_PROGRAM) {
        return $env:TERM_PROGRAM
    }

    return "unknown"
}

function Get-DetectedShell {
    if ($Shell) {
        return $Shell
    }

    if ($env:WSL_DISTRO_NAME) {
        return "WSL bash"
    }

    if ($PSVersionTable.PSEdition) {
        return "PowerShell $($PSVersionTable.PSEdition)"
    }

    return "unknown"
}

function Get-DetectedOs {
    if ($Os) {
        return $Os
    }

    try {
        $instance = Get-CimInstance Win32_OperatingSystem
        return "$($instance.Caption) $($instance.Version)"
    } catch {
        return [System.Environment]::OSVersion.VersionString
    }
}

function Get-SlugValue {
    param([string]$Value)

    $slug = $Value.ToLowerInvariant() -replace '[^a-z0-9]+', '-'
    $slug = $slug.Trim('-')
    if ([string]::IsNullOrWhiteSpace($slug)) {
        return "unknown"
    }
    return $slug
}

$repoRoot = git rev-parse --show-toplevel
if ($OutputPath -and $OutputDir) {
    throw "-OutputPath and -OutputDir cannot be used together."
}

if (-not $Commit) {
    $Commit = git -C $repoRoot rev-parse HEAD
}

if (-not $Date) {
    $Date = Get-Date -Format "yyyy-MM-dd"
}

$detectedOs = Get-DetectedOs
$detectedTerminal = Get-DetectedTerminal
$detectedShell = Get-DetectedShell

if ($OutputDir) {
    $fileName = @(
        $Date
        (Get-SlugValue $detectedOs)
        (Get-SlugValue $detectedTerminal)
        (Get-SlugValue $detectedShell)
        (Get-SlugValue $Frontend)
    ) -join "-"
    $OutputPath = Join-Path $OutputDir "$fileName.txt"
}

$report = @(
    "date: $Date"
    "commit: $Commit"
    "os: $detectedOs"
    "terminal: $detectedTerminal"
    "shell: $detectedShell"
    "frontend: $Frontend"
    "term: $Term"
    "checks:"
    "- launch and exit"
    "- frontend selection"
    "- input editing"
    "- overlay flow"
    "- streaming visibility"
    "- resize and scrollback"
    "- failure and recovery"
    "result: $Result"
    "notes: $Notes"
) -join [Environment]::NewLine

if ($OutputPath) {
    $parent = Split-Path -Parent $OutputPath
    if ($parent) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    Set-Content -Path $OutputPath -Value $report
    Write-Output "wrote $OutputPath"
} else {
    Write-Output $report
}
