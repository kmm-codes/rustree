# Local release build - produces the distributable Windows setup.
#
# Runs the gate suite (cargo fmt --check, cargo clippy -D warnings, cargo test),
# builds the optimised executable, compiles the setup from installer/rustree.nsi
# with NSIS and collects it (plus its SHA-256 checksum) under release/<version>/.
#
# For a quick refresh of the installed build see scripts/update.ps1 - it skips
# gates and setup and only replaces the installed executable.
#
# The version is checked first, before the gates, and never raised here: this
# script builds what Cargo.toml declares and creates no tag. Raising the number
# is an edit of `version` in the [package] table of Cargo.toml plus a tag
# v<version> on the commit.
#
# makensis is looked up on PATH, in the standard NSIS install directory and in
# the copy Tauri keeps under %LOCALAPPDATA%\tauri\NSIS; `winget install
# NSIS.NSIS` provides it otherwise.
#
# Usage:
#   pwsh scripts/release.ps1              # version guard + gates + build + setup
#   pwsh scripts/release.ps1 -SkipGates   # rebuild only (gates already green)
#
# Output: release/<version>/rustree_<version>_x64-setup.exe - per-user install
# under %LOCALAPPDATA%\Programs\rustree, Start-menu entry, uninstaller wired
# into the Windows "Apps" settings - and SHA256SUMS.txt next to it.

param([switch]$SkipGates)

$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

function Invoke-Gate([string]$Name, [scriptblock]$Command) {
  Write-Host "==> $Name" -ForegroundColor Cyan
  & $Command
  if ($LASTEXITCODE -ne 0) { throw "gate failed: $Name" }
}

# git calls whose failure is an answer, not an error - a missing tag is the
# normal case before the first release. Neutralizes $ErrorActionPreference for
# the call so a shell with $PSNativeCommandUseErrorActionPreference enabled
# cannot turn the expected non-zero exit into a terminating error.
function Invoke-Git([string[]]$Arguments) {
  $previous = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try { $output = & git @Arguments 2>$null } finally { $ErrorActionPreference = $previous }
  [pscustomobject]@{ ExitCode = $LASTEXITCODE; Output = ($output | Out-String).Trim() }
}

# Only the [package] table of Cargo.toml counts - every dependency carries a
# `version` key too.
function Get-DeclaredVersion([string]$RepoRoot) {
  $cargo = [IO.File]::ReadAllText((Join-Path $RepoRoot 'Cargo.toml'))
  $package = [regex]::Match($cargo, '(?ms)^\[package\]\r?\n.*?(?=^\[|\z)').Value
  [regex]::Match($package, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
}

# Refuses to build a number that is already out: a setup carrying an old
# number would silently overwrite a newer install with older code. A tag
# pointing at HEAD is the opposite case - the deliberate rebuild of exactly
# that release - and passes. A missing tag also passes; this script never
# creates one. Returns the version being built.
function Assert-ReleaseVersion([string]$RepoRoot) {
  $version = Get-DeclaredVersion $RepoRoot
  if (-not $version) { throw 'no version found in the [package] table of Cargo.toml' }
  if ($version -notmatch '^\d+\.\d+\.\d+$') {
    throw "version '$version' is not MAJOR.MINOR.PATCH (digits only) - the setup's version resource takes no pre-release suffix"
  }
  Write-Host "==> version $version" -ForegroundColor Cyan

  $tag = "v$version"
  $tagged = Invoke-Git @('-C', $RepoRoot, 'rev-parse', '-q', '--verify', "refs/tags/$tag^{commit}")
  $head = Invoke-Git @('-C', $RepoRoot, 'rev-parse', '-q', '--verify', 'HEAD')
  if ($tagged.ExitCode -eq 0) {
    if ($tagged.Output -ne $head.Output) {
      $parts = $version.Split('.')
      $next = '{0}.{1}.{2}' -f $parts[0], $parts[1], ([int]$parts[2] + 1)
      throw "version $version is already released as tag $tag (at $($tagged.Output.Substring(0, 7))); raise it first, e.g. to $next"
    }
    Write-Host "    tag $tag points at HEAD - rebuilding exactly that release"
  } else {
    Write-Host "    no tag $tag yet - release.ps1 never creates one"
  }

  # A release built from uncommitted work matches no commit anyone can return
  # to; worth saying out loud, but the build itself stays the caller's call.
  $status = Invoke-Git @('-C', $RepoRoot, 'status', '--porcelain')
  $dirty = @($status.Output -split "`n" | Where-Object { $_.Trim() })
  if ($dirty.Count -gt 0) {
    Write-Host "    working tree is dirty ($($dirty.Count) change(s)) - the setup will match no commit" -ForegroundColor Yellow
  }

  $version
}

function Find-MakeNsis {
  $candidates = @(
    (Get-Command makensis -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    foreach ($base in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
      if ($base) { Join-Path $base 'NSIS\makensis.exe' }
    }
    Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'
  )
  $found = $candidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
  if (-not $found) {
    throw 'makensis not found - install NSIS (winget install NSIS.NSIS) or put makensis on PATH'
  }
  $found
}

# Both run regardless of -SkipGates: a wrong number invalidates the setup no
# matter how green the gates are, and a missing compiler should fail before
# minutes of building.
$version = Assert-ReleaseVersion -RepoRoot $repo
$makensis = Find-MakeNsis
Write-Host "    makensis: $makensis"

if (-not $SkipGates) {
  Invoke-Gate 'cargo fmt' { cargo fmt --check }
  Invoke-Gate 'cargo clippy' { cargo clippy --all-targets -- -D warnings }
  Invoke-Gate 'cargo test' { cargo test }
}

Invoke-Gate 'cargo build --release' { cargo build --release }

$exe = Join-Path $repo 'target\release\rustree.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw "no executable at $exe" }

$out = Join-Path $repo "release\$version"
New-Item -ItemType Directory -Force $out | Out-Null
$setup = Join-Path $out "rustree_${version}_x64-setup.exe"
$script = Join-Path $repo 'installer\rustree.nsi'

Invoke-Gate 'makensis' {
  & $makensis /V2 "/DVERSION=$version" "/DEXE=$exe" "/DOUTFILE=$setup" $script
}
if (-not (Test-Path -LiteralPath $setup)) { throw "makensis produced no setup at $setup" }

$hash = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLower()
Set-Content -Path (Join-Path $out 'SHA256SUMS.txt') -Value "$hash  $(Split-Path -Leaf $setup)"

Write-Host "`nRelease $version ready:" -ForegroundColor Green
Get-ChildItem $out | ForEach-Object { Write-Host "  $($_.Name)  $([math]::Round($_.Length / 1MB, 1)) MB" }
