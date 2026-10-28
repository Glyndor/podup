#Requires -Version 5.1
<#
The installer's python probe must not import from the current directory.

`python -c` puts the current directory at the front of sys.path, so a
`cryptography/` package planted in the directory the installer was started
from is imported instead of the real one. The probe then reports a
usable interpreter that cannot verify a signature, and the planted code
runs with the installer's privileges.

The isolated mode (-I) takes the current directory (and the user site
directory) out of the search path, so only what the system installed can
be imported. The probe has to carry it.

The test replaces python with a stub that records the arguments it was
called with, and asserts the isolated flag is there and comes before
the `-c` that carries the probe program: python only accepts the
isolated flag before the program text.

Shape mirrors tests/scripts/powershell/check-hardening.Tests.ps1: a
function-based harness with explicit Assert-* helpers and exit codes,
rather than Pester, since this repository's PowerShell fixtures all use
the same shape.

Requires: nothing beyond PowerShell 5.1. CI runs this on windows-latest
with `shell: pwsh`, where pwsh is on PATH.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# PowerShell 7.3+ turns a non-zero native exit into a terminating error
# under ErrorActionPreference='Stop'. Find-Python probes several
# interpreters and reads the exit code itself, so opt out. Harmless
# no-op on Windows PowerShell 5.1, which lacks this variable.
$PSNativeCommandUseErrorActionPreference = $false

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$InstallerPath = Join-Path $RepoRoot 'install.ps1'

if (-not (Test-Path -LiteralPath $InstallerPath)) {
	Write-Host "FAIL: $InstallerPath not found" -ForegroundColor Red
	exit 1
}

$pass = 0
$fail = 0

function Assert-Eq {
	param([string]$Desc, [string]$Expected, [string]$Actual)
	if ($Expected -eq $Actual) {
		Write-Host "ok    $Desc"
		$script:pass++
	} else {
		Write-Host "FAIL  $Desc"
		Write-Host ("        expected: {0}" -f $Expected)
		Write-Host ("        actual:   {0}" -f $Actual)
		$script:fail++
	}
}

function Assert-True {
	param([bool]$Condition, [string]$Desc)
	if ($Condition) {
		Write-Host "ok    $Desc"
		$script:pass++
	} else {
		Write-Host "FAIL  $Desc" -ForegroundColor Red
		$script:fail++
	}
}

# Extract Find-Python via AST so the test drives the real code, not a
# copy. Fail closed if the function is missing: a regression that drops
# the probe from install.ps1 must break this test.
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
	$InstallerPath, [ref]$null, [ref]$null)
$probeFn = $ast.Find({ param($n)
	$n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
	$n.Name -eq 'Find-Python'
}, $true)
if (-not $probeFn) {
	Write-Host 'FAIL: Find-Python not found in install.ps1' -ForegroundColor Red
	exit 1
}
Invoke-Expression $probeFn.Extent.Text

$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName())
[System.IO.Directory]::CreateDirectory($tmp) | Out-Null
try {
	$onWindows = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
		[System.Runtime.InteropServices.OSPlatform]::Windows)
	# On Windows the stub has to be a .cmd, because that is what the
	# command lookup resolves `python3` to. On the other platforms the
	# name has to carry no extension and be executable, for the same
	# reason.
	$stubName = if ($onWindows) { 'python3.cmd' } else { 'python3' }
	$stubPath = Join-Path $tmp $stubName
	$logPath = Join-Path $tmp 'probe-args.log'

	if ($onWindows) {
		# The stub writes the arguments it was called with, on one line.
		# A single-quoted here-string keeps the batch syntax literal:
		# nothing in it is a PowerShell escape. No labels and no loops,
		# because the file is written with LF endings and cmd.exe reads
		# a label in an LF-only batch file as a different label.
		$template = @'
@echo off
echo %*>>"__LOG__"
exit /b 0
'@
		Set-Content -LiteralPath $stubPath -Value $template.Replace('__LOG__', $logPath) -Encoding ASCII -NoNewline
	} else {
		$template = @'
#!/bin/sh
printf '%s\n' "$*" >> '__LOG__'
exit 0
'@
		Set-Content -LiteralPath $stubPath -Value $template.Replace('__LOG__', $logPath) -Encoding ASCII
		& chmod +x $stubPath
	}

	# The stub dir goes first on PATH so the lookup finds it before any
	# python the runner happens to carry.
	$oldPath = $env:PATH
	$env:PATH = "$tmp$([System.IO.Path]::PathSeparator)$oldPath"
	try {
		$found = Find-Python
	} finally {
		$env:PATH = $oldPath
	}

	Assert-True ($null -ne $found) 'the probe finds the interpreter on PATH'

	$probeArgs = @()
	if (Test-Path -LiteralPath $logPath) {
		# The stub writes every argument on one line; the flags are
		# whole tokens in it.
		$probeArgs = @((Get-Content -LiteralPath $logPath) -split '\s+')
	}

	$isoIndex = [Array]::IndexOf($probeArgs, '-I')
	$codeIndex = [Array]::IndexOf($probeArgs, '-c')

	Assert-True ($isoIndex -ge 0) 'the probe runs python in isolated mode (-I)'
	Assert-True ($codeIndex -ge 0) 'the probe still passes its program with -c'
	Assert-True (($isoIndex -ge 0) -and ($codeIndex -ge 0) -and ($isoIndex -lt $codeIndex)) `
		'the isolated flag comes before the program text'
} finally {
	Remove-Item -LiteralPath $tmp -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "passed: $pass  failed: $fail"
if ($fail -eq 0) { exit 0 } else { exit 1 }
