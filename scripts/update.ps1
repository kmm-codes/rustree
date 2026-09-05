# Fast refresh of the installed build - maintains the per-user install.
#
# The counterpart for distributable output is scripts/release.ps1 (gate suite,
# optimised build, NSIS setup); this script is the iterate-and-look loop
# instead: an incremental debug build, stop the running app, copy the fresh
# executable into `%LOCALAPPDATA%\Programs\rustree`, and start it again. It
# runs no gates and produces no installer - it is not a substitute for the
# gate suite before a commit.
#
# The install directory is the one the setup from release.ps1 uses as well, so
# an update replaces whatever the setup put there while the Start-menu entry
# and the "Apps" uninstall entry of the setup stay valid. Without a prior setup
# the script creates the directory and a Start-menu shortcut itself.
#
# rustree runs elevated (rustree.manifest requests administrator rights), and
# only an elevated process can stop such an instance. The build therefore runs
# as the current user; when an instance has to be stopped, the
# stop-replace-relaunch step re-runs this script elevated behind one UAC
# prompt, and the relaunched app inherits those rights. When nothing is
# running the copy happens in place and the app shows its own UAC prompt at
# start, exactly as from the Start menu - one prompt per update either way.
#
# Usage:
#   pwsh scripts/update.ps1              # build + replace + relaunch
#   pwsh scripts/update.ps1 -Release     # the same with an optimised build (slower to compile)
#   pwsh scripts/update.ps1 -SkipBuild   # reuse the exe already in target/
#   pwsh scripts/update.ps1 -NoLaunch    # replace only, do not start the app
#   pwsh scripts/update.ps1 -DryRun      # report the plan, change nothing (implies no build)
#
param(
  [switch]$SkipBuild,
  [switch]$Release,
  [switch]$NoLaunch,
  [switch]$DryRun,
  # Internal, set by the elevated re-invocation: no build, no report, and
  # every line mirrored into -LogFile for the waiting parent to print.
  [Parameter(DontShow)][switch]$Elevated,
  [Parameter(DontShow)][string]$LogFile
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$productName = 'rustree'
$exeName = 'rustree.exe'                        # cargo package name, identical in target/ and install dir
$windowTitle = 'rustree - Disk Space Analyzer'  # MainWindow.title in ui/main.slint
$buildProfile = if ($Release) { 'release' } else { 'debug' }
$sourceExe = Join-Path $repo "target\$buildProfile\$exeName"
$installDir = Join-Path $env:LOCALAPPDATA "Programs\$productName"
$destExe = Join-Path $installDir $exeName

if ($Elevated) { $SkipBuild = $true }

# Every line goes through here so the elevated re-invocation, which runs with
# a hidden console, can hand its output to the parent through the log file.
function Out-Line([string]$Text, [string]$Color) {
  if ($Color) { Write-Host $Text -ForegroundColor $Color } else { Write-Host $Text }
  if ($LogFile) { Add-Content -LiteralPath $LogFile -Value "$Color`t$Text" }
}
function Write-Step([string]$Text) { Out-Line "==> $Text" 'Cyan' }
function Write-Note([string]$Text) { Out-Line "    $Text" '' }
function Write-Warn([string]$Text) { Out-Line "    $Text" 'Yellow' }

# A failure inside the elevated re-invocation would vanish with its hidden
# console; it goes into the log instead and the parent reports it.
trap {
  if ($Elevated) { Out-Line "    error: $($_.Exception.Message)" 'Red'; exit 1 }
  break
}

function Test-Elevated {
  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  ([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-ExeInfo([string]$Path) {
  $item = Get-Item -LiteralPath $Path
  [pscustomobject]@{
    Path    = $item.FullName
    Version = if ($item.VersionInfo.FileVersion) { $item.VersionInfo.FileVersion } else { 'unversioned' }
    Hash    = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLower()
    Written = $item.LastWriteTime
  }
}

function Format-ExeInfo($Info) {
  '{0}  sha256 {1}  ({2:yyyy-MM-dd HH:mm})' -f $Info.Version, $Info.Hash.Substring(0, 12), $Info.Written
}

# Only an instance started from the install directory is ours to stop; a build
# launched from target/ must survive an update untouched. Windows withholds
# the Path of an elevated process from a non-elevated caller, so the report
# falls back to the main window title there - the elevated step sees the full
# path and decides for real.
function Get-InstalledInstance {
  $prefix = $installDir.TrimEnd('\') + '\'
  Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($exeName)) -ErrorAction SilentlyContinue |
    Where-Object {
      $path = $null
      try { $path = $_.Path } catch { $path = $null }
      if ($path) { return $path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) }
      $_.MainWindowTitle -eq $windowTitle
    }
}

# Same path and name as the shortcut the setup creates, so both stay one entry.
function Install-UserShortcut([string]$Executable) {
  $programs = [Environment]::GetFolderPath([Environment+SpecialFolder]::Programs)
  $shortcutPath = Join-Path $programs "$productName.lnk"
  $shell = New-Object -ComObject WScript.Shell
  $shortcut = $shell.CreateShortcut($shortcutPath)
  $shortcut.TargetPath = $Executable
  $shortcut.WorkingDirectory = Split-Path -Parent $Executable
  $shortcut.IconLocation = "$Executable,0"
  $shortcut.Save()
}

function Wait-ProcessExit($Processes, [int]$Seconds) {
  $deadline = (Get-Date).AddSeconds($Seconds)
  while ((Get-Date) -lt $deadline) {
    if (@($Processes | Where-Object { -not $_.HasExited }).Count -eq 0) { return @() }
    Start-Sleep -Milliseconds 200
  }
  @($Processes | Where-Object { -not $_.HasExited })
}

# Closing the window first lets the app shut down on its own; the forced kill
# is the fallback for an unresponsive instance. An open handle on the exe
# blocks the copy, so a process that survives both is a hard failure.
function Stop-InstalledInstance($Processes) {
  foreach ($process in $Processes) {
    try { [void]$process.CloseMainWindow() } catch { }
  }
  $alive = @(Wait-ProcessExit $Processes 5)
  if ($alive.Count -gt 0) { $alive | Stop-Process -Force -ErrorAction SilentlyContinue }
  $alive = @(Wait-ProcessExit $Processes 5)
  if ($alive.Count -gt 0) {
    throw "could not stop $exeName (pid $($alive.Id -join ', ')) - close it manually and rerun"
  }
}

# Re-runs this script with administrator rights for the replace step and
# prints what it logged. -Wait blocks until it is done; a declined UAC prompt
# surfaces as an exception from Start-Process.
function Invoke-ElevatedInstall {
  $log = Join-Path ([IO.Path]::GetTempPath()) "$productName-update-$PID.log"
  Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
  $arguments = @(
    '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"",
    '-Elevated', '-LogFile', "`"$log`""
  )
  if ($Release) { $arguments += '-Release' }
  if ($NoLaunch) { $arguments += '-NoLaunch' }

  Write-Step 'elevating to stop and replace the running app (UAC prompt)'
  try {
    $process = Start-Process -FilePath (Get-Process -Id $PID).Path -ArgumentList $arguments `
      -Verb RunAs -WindowStyle Hidden -Wait -PassThru
  } catch {
    throw "elevation declined or failed: $($_.Exception.Message)"
  }

  if (Test-Path -LiteralPath $log) {
    foreach ($line in Get-Content -LiteralPath $log) {
      $color, $text = $line -split "`t", 2
      if ($color) { Write-Host $text -ForegroundColor $color } else { Write-Host $text }
    }
    Remove-Item -LiteralPath $log -Force -ErrorAction SilentlyContinue
  }
  if ($process.ExitCode -ne 0) {
    throw "the elevated replace step failed (exit code $($process.ExitCode))"
  }
}

# Stop, copy, shortcut, verify, launch - the part that may need elevation.
function Invoke-Install($Source, $Installed) {
  $running = @(Get-InstalledInstance)
  if ($running.Count -gt 0) {
    Write-Step "stopping $($running.Count) running instance(s)"
    Stop-InstalledInstance $running
  }

  Write-Step "installing into $installDir"
  New-Item -ItemType Directory -Path $installDir -Force | Out-Null
  Copy-Item -LiteralPath $Source.Path -Destination $destExe -Force
  Install-UserShortcut $destExe

  # The destination hash decides whether the copy really took effect.
  $updated = Get-ExeInfo $destExe
  if ($updated.Hash -ne $Source.Hash) {
    throw "copy did not take effect - '$destExe' still differs from the build"
  }

  if (-not $NoLaunch) {
    # Elevated here: the app inherits the rights its manifest asks for. Not
    # elevated: Windows shows the app's own UAC prompt, as from the Start menu.
    Write-Step 'starting the updated app'
    Start-Process -FilePath $destExe -WorkingDirectory $installDir
  }

  Out-Line '' ''
  Out-Line "$productName updated: $(if ($Installed) { $Installed.Version } else { 'new per-user copy' }) -> $($updated.Version)" 'Green'
  Out-Line "  $destExe  ($([math]::Round((Get-Item -LiteralPath $destExe).Length / 1MB, 1)) MB, sha256 $($updated.Hash.Substring(0, 12)))" ''
}

# --- resolve the installation -------------------------------------------------

$installed = if (Test-Path -LiteralPath $destExe) { Get-ExeInfo $destExe } else { $null }

# --- produce the executable to install ---------------------------------------

$buildArgs = @('build') + $(if ($Release) { @('--release') } else { @() })
$buildCommand = "cargo $($buildArgs -join ' ')"

if ($DryRun) {
  if (-not $SkipBuild) { Write-Step "would run: $buildCommand" }
} elseif (-not $SkipBuild) {
  # The debug profile keeps Rust incremental for this iteration-only
  # executable; -Release trades compile time for scan speed.
  Write-Step $buildCommand
  & cargo @buildArgs
  if ($LASTEXITCODE -ne 0) { throw "build failed: $buildCommand" }
}

if (-not (Test-Path -LiteralPath $sourceExe)) {
  if ($DryRun) {
    Write-Step 'dry run - nothing was changed'
    Write-Note "install dir : $installDir"
    Write-Note "installed   : $(if ($installed) { Format-ExeInfo $installed } else { 'not created yet' })"
    Write-Note "built       : none yet at $sourceExe"
    Write-Note 'a real run would build it first (omit -SkipBuild)'
    exit 0
  }
  throw "no build found at '$sourceExe' - rerun without -SkipBuild"
}

$source = Get-ExeInfo $sourceExe
$staleHint = $null

if ($SkipBuild -and -not $Elevated) {
  $newest = @(
    Get-ChildItem -Path 'src', 'ui' -Recurse -File -ErrorAction SilentlyContinue
    Get-Item -Path 'Cargo.toml', 'build.rs', 'rustree.manifest' -ErrorAction SilentlyContinue
  ) | Sort-Object LastWriteTime | Select-Object -Last 1
  if ($newest -and $newest.LastWriteTime -gt $source.Written) { $staleHint = $newest.FullName }
}

# --- report -------------------------------------------------------------------

$running = @(Get-InstalledInstance)
$isElevated = Test-Elevated
$needsElevation = $running.Count -gt 0

if (-not $Elevated) {
  $elevationNote = if ($isElevated) { 'not needed (already elevated)' }
    elseif ($needsElevation) { 'one UAC prompt, to stop and restart the running app' }
    elseif ($NoLaunch) { 'not required' }
    else { "not required for the copy; the app asks itself at start" }

  Write-Step "$productName update"
  Write-Note "install dir : $installDir"
  Write-Note "installed   : $(if ($installed) { Format-ExeInfo $installed } else { 'not created yet' })"
  Write-Note "built       : $(Format-ExeInfo $source)  [$buildProfile]"
  Write-Note "running     : $(if ($running.Count -gt 0) { "$($running.Count) instance(s), pid $($running.Id -join ', ')" } else { 'no' })"
  Write-Note "elevation   : $elevationNote"
  if ($staleHint) { Write-Warn "the build is older than $staleHint - rerun without -SkipBuild" }
}

if ($installed -and $source.Hash -eq $installed.Hash) {
  Write-Host "`nAlready up to date - the installed executable is identical to the build." -ForegroundColor Green
  if (-not $NoLaunch -and -not $DryRun -and $running.Count -eq 0) {
    Write-Step 'starting the installed app'
    Start-Process -FilePath $destExe -WorkingDirectory $installDir
  }
  exit 0
}

if ($DryRun) {
  Write-Step 'dry run - nothing was changed'
  if ($needsElevation -and -not $isElevated) { Write-Note 'would elevate : yes (UAC prompt)' }
  if ($running.Count -gt 0) { Write-Note "would stop    : pid $($running.Id -join ', ')" }
  Write-Note "would copy    : $sourceExe"
  Write-Note "             -> $destExe"
  Write-Note "would launch  : $(if ($NoLaunch) { 'no (-NoLaunch)' } else { $destExe })"
  exit 0
}

# --- replace ------------------------------------------------------------------

if ($needsElevation -and -not $isElevated) {
  Invoke-ElevatedInstall
} else {
  Invoke-Install $source $installed
}
