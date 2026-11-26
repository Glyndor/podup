#Requires -Version 5.1
<#
The installer must not write more than its ceiling, whatever a response says.

A download is read from the response in blocks (Save-HttpDownload, over
Save-LimitedStream), so a body of unknown length stops at the ceiling instead
of landing on disk whole. This test extracts both functions and
Invoke-HttpHop from install.ps1 by AST, so it drives the real code:

  1. Save-LimitedStream over memory streams: exactly the ceiling is kept, one
     byte more is refused and leaves no file, and a body shorter than its
     declared length is refused.
  2. Save-HttpDownload against a local HTTP listener: a complete body, a
     declared length over the ceiling, a chunked body over the ceiling, a body
     cut short, a redirect (returned, not followed) and a body that stalls.
  3. Invoke-HttpHop sends a request that saves to a file through
     Save-HttpDownload, with the installer's ceiling.

Shape mirrors tests/scripts/powershell/install-https-redirects.Tests.ps1: a
function-based harness with explicit Assert-* helpers and exit codes.

Requires: nothing beyond PowerShell 5.1. CI runs this on windows-latest
with `shell: pwsh`.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..\..')).Path
$InstallerPath = Join-Path $RepoRoot 'install.ps1'

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

# Extract the real functions via AST; a missing one must fail the fixture.
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
	$InstallerPath, [ref]$null, [ref]$null)
foreach ($name in 'Save-LimitedStream', 'Save-HttpDownload', 'Invoke-HttpHop') {
	$fn = $ast.Find({ param($n)
		$n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
		$n.Name -eq $name
	}, $true)
	if (-not $fn) {
		Write-Host "FAIL: $name not found in install.ps1" -ForegroundColor Red
		exit 1
	}
	Invoke-Expression $fn.Extent.Text
}

$Work = New-Item -ItemType Directory -Path (Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName()))
$Ceiling = 4096

# "<outcome>|<bytes on disk>" of one call: outcome is ok or the exception message.
function Read-Outcome {
	param([scriptblock]$Call, [string]$Path)
	$outcome = 'ok'
	try { $null = & $Call } catch { $outcome = $_.Exception.Message }
	$size = 'none'
	if (Test-Path -LiteralPath $Path) { $size = (Get-Item -LiteralPath $Path).Length }
	return "$outcome|$size"
}

# 1. The stream copy.
function Invoke-Copy {
	param([string]$Name, [int]$Bytes, $Expected = $null)
	$path = Join-Path $Work $Name
	$source = New-Object System.IO.MemoryStream -ArgumentList @(, (New-Object byte[] $Bytes))
	return Read-Outcome { Save-LimitedStream -Source $source -Path $path -MaxBytes $Ceiling -TimeoutSec 5 -ExpectedBytes $Expected } $path
}

Assert-Eq 'a body of exactly the ceiling is kept' "ok|$Ceiling" (Invoke-Copy 'exact' $Ceiling)
Assert-Eq 'one byte over the ceiling is refused and leaves no file' `
	"Download too large (over $Ceiling bytes)|none" (Invoke-Copy 'over' ($Ceiling + 1))
Assert-Eq 'a body many blocks over the ceiling is refused and leaves no file' `
	"Download too large (over $Ceiling bytes)|none" (Invoke-Copy 'far' 500000)
Assert-Eq 'a body shorter than declared is refused and leaves no file' `
	'Incomplete download: 10 of 100 bytes|none' (Invoke-Copy 'short' 10 100)
Assert-Eq 'a body of its declared length is kept' 'ok|100' (Invoke-Copy 'declared' 100 100)

# 2. The request, against a local listener.
$probe = New-Object System.Net.Sockets.TcpListener -ArgumentList ([System.Net.IPAddress]::Loopback), 0
$probe.Start()
$port = $probe.LocalEndpoint.Port
$probe.Stop()
$base = "http://localhost:$port"
$listener = New-Object System.Net.HttpListener
$listener.Prefixes.Add("$base/")
$listener.Start()

$server = [PowerShell]::Create()
$null = $server.AddScript({
	param($listener)
	while ($listener.IsListening) {
		try { $context = $listener.GetContext() } catch { break }
		$response = $context.Response
		try {
			$route = $context.Request.Url.AbsolutePath
			if ($route -eq '/ok') {
				$response.ContentLength64 = 100
				$response.OutputStream.Write((New-Object byte[] 100), 0, 100)
				$response.Close()
			} elseif ($route -eq '/declared') {
				$response.ContentLength64 = 5000
				$response.OutputStream.Write((New-Object byte[] 5000), 0, 5000)
				$response.Close()
			} elseif ($route -eq '/chunked') {
				$response.SendChunked = $true
				$response.OutputStream.Write((New-Object byte[] 200000), 0, 200000)
				$response.Close()
			} elseif ($route -eq '/short') {
				$response.ContentLength64 = 1000
				$response.OutputStream.Write((New-Object byte[] 10), 0, 10)
				$response.OutputStream.Flush()
				Start-Sleep -Milliseconds 300
				$response.Abort()
			} elseif ($route -eq '/moved') {
				$response.StatusCode = 302
				$response.AddHeader('Location', 'https://example.net/next')
				$response.Close()
			} elseif ($route -eq '/stall') {
				$response.SendChunked = $true
				$response.OutputStream.Write((New-Object byte[] 10), 0, 10)
				$response.OutputStream.Flush()
				Start-Sleep -Seconds 6
				$response.Abort()
			} else {
				$response.StatusCode = 404
				$response.Close()
			}
		} catch {
			# The client closes early on a refusal, which makes the write fail.
			$response.Abort()
		}
	}
}).AddArgument($listener)
$running = $server.BeginInvoke()

function Invoke-Download {
	param([string]$Route, [int]$TimeoutSec = 20)
	$path = Join-Path $Work ($Route.TrimStart('/') + '.bin')
	return Read-Outcome { Save-HttpDownload -Uri "$base$Route" -OutFile $path -TimeoutSec $TimeoutSec -MaxBytes $Ceiling } $path
}

try {
	Assert-Eq 'a complete download is kept' 'ok|100' (Invoke-Download '/ok')
	Assert-Eq 'a declared length over the ceiling is refused' `
		"Download too large (over $Ceiling bytes)|none" (Invoke-Download '/declared')
	Assert-Eq 'a body of unknown length over the ceiling is refused' `
		"Download too large (over $Ceiling bytes)|none" (Invoke-Download '/chunked')
	$short = Invoke-Download '/short'
	Assert-Eq 'a body cut short is refused and leaves no file' 'refused|none' `
		$(if ($short.StartsWith('ok|')) { $short } else { 'refused|' + $short.Split('|')[-1] })
	$moved = Save-HttpDownload -Uri "$base/moved" -OutFile (Join-Path $Work 'moved.bin') -TimeoutSec 20 -MaxBytes $Ceiling
	Assert-Eq 'a redirect is returned and not followed' '302 https://example.net/next' `
		"$($moved.StatusCode) $($moved.Location)"
	$stall = Invoke-Download '/stall' 2
	Assert-Eq 'a stalled body is given up on and leaves no file' 'refused|none' `
		$(if ($stall.StartsWith('ok|')) { $stall } else { 'refused|' + $stall.Split('|')[-1] })
} finally {
	$listener.Stop()
	$server.Dispose()
}

# 3. A request that saves to a file goes through Save-HttpDownload with the ceiling.
$script:Sent = ''
function Save-HttpDownload {
	param([string]$Uri, [string]$OutFile, [hashtable]$Headers, [int]$TimeoutSec, [long]$MaxBytes)
	$script:Sent = "$Uri $OutFile $TimeoutSec $MaxBytes"
	return [pscustomobject]@{ StatusCode = 200; Location = $null; Content = $null }
}
$MaxDownloadBytes = 209715200
$null = Invoke-HttpHop -Uri 'https://example.net/a' -OutFile 'out.bin' -TimeoutSec 7
Assert-Eq 'Invoke-HttpHop saves a file under the installer ceiling' `
	'https://example.net/a out.bin 7 209715200' $script:Sent

Remove-Item -LiteralPath $Work -Recurse -Force

Write-Host ""
Write-Host "passed: $pass  failed: $fail"
if ($fail -eq 0) { exit 0 } else { exit 1 }
