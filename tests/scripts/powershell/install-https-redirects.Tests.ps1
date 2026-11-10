#Requires -Version 5.1
<#
The installer must not follow a redirect that leaves https.

Invoke-WebRequest follows redirects on its own and puts no limit on the
scheme, so a release URL that answered 302 to an http:// address would
have the download fetched in the clear. install.sh pins curl to https for
redirects and the Rust updater refuses them as well; install.ps1 has to
hold the same line.

The installer now follows redirects by hand in Invoke-HttpsRequest, one
hop at a time through Invoke-HttpHop, which is the only function that
touches the network. This test extracts Invoke-HttpsRequest from
install.ps1 by AST, so it drives the real code, and replaces
Invoke-HttpHop with a stub that answers from a table. It needs no network.

  1. https to https is followed and the final response is returned.
  2. https to http is refused, and the http address is never requested.
  3. A relative Location is resolved against the URL that returned it.
  4. More hops than the cap is refused, and the cap is exact.
  5. A start URL that is not https, a missing Location and an unexpected
     3xx status are refused.
  6. Every Invoke-WebRequest in install.ps1 sits inside Invoke-HttpHop,
     and that call turns redirects off, so nothing can bypass the loop.

Shape mirrors tests/scripts/powershell/install-python-isolation.Tests.ps1:
a function-based harness with explicit Assert-* helpers and exit codes,
rather than Pester.

Requires: nothing beyond PowerShell 5.1. CI runs this on windows-latest
with `shell: pwsh`.
#>

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

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

# Extract the real functions via AST; a missing one must fail the fixture.
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
	$InstallerPath, [ref]$null, [ref]$null)
function Find-InstallerFunction {
	param([string]$Name)
	return $ast.Find({ param($n)
		$n -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
		$n.Name -eq $Name
	}, $true)
}
$loopFn = Find-InstallerFunction 'Invoke-HttpsRequest'
$hopFn = Find-InstallerFunction 'Invoke-HttpHop'
if (-not $loopFn -or -not $hopFn) {
	Write-Host 'FAIL: Invoke-HttpsRequest or Invoke-HttpHop not found in install.ps1' -ForegroundColor Red
	exit 1
}
Invoke-Expression $loopFn.Extent.Text

# The stub answers from $script:Routes (url -> status and Location) and
# records every URL it was asked for. An address with no route is a test
# bug, so it throws instead of inventing an answer.
$script:Routes = @{}
$script:Requested = @()
function Invoke-HttpHop {
	param([string]$Uri, [string]$OutFile, [hashtable]$Headers, [int]$TimeoutSec)
	$script:Requested += $Uri
	if (-not $script:Routes.ContainsKey($Uri)) { throw "no stub route for $Uri" }
	$route = $script:Routes[$Uri]
	return [pscustomobject]@{
		StatusCode = $route.Status
		Location = $route.Location
		Content = $route.Content
	}
}

function Reset-Stub {
	param([hashtable]$Routes)
	$script:Routes = $Routes
	$script:Requested = @()
}

# Run the loop and report whether it threw, with the message or the result.
function Invoke-Loop {
	param([string]$Uri, [int]$MaxRedirects = 5)
	try {
		$result = Invoke-HttpsRequest -Uri $Uri -TimeoutSec 5 -MaxRedirects $MaxRedirects
		return [pscustomobject]@{ Threw = $false; Message = ''; Result = $result }
	} catch {
		return [pscustomobject]@{ Threw = $true; Message = $_.Exception.Message; Result = $null }
	}
}

# 1. https to https is followed.
Reset-Stub @{
	'https://github.com/o/r/releases/latest/download/a' =
		@{ Status = 302; Location = 'https://objects.example.net/blob/a'; Content = $null }
	'https://objects.example.net/blob/a' = @{ Status = 200; Location = $null; Content = 'payload' }
}
$out = Invoke-Loop 'https://github.com/o/r/releases/latest/download/a'
Assert-True (-not $out.Threw) 'an https to https redirect is followed'
Assert-Eq 'the final response is returned' 'payload' ([string]$out.Result.Content)
Assert-Eq 'both hops were requested, in order' `
	'https://github.com/o/r/releases/latest/download/a|https://objects.example.net/blob/a' `
	($script:Requested -join '|')

# No redirect at all is a plain success.
Reset-Stub @{ 'https://example.net/a' = @{ Status = 200; Location = $null; Content = 'x' } }
$out = Invoke-Loop 'https://example.net/a'
Assert-True ((-not $out.Threw) -and ($script:Requested.Count -eq 1)) 'a direct 200 makes one request'

# 2. https to http is refused and the http address is never requested.
Reset-Stub @{
	'https://example.net/a' = @{ Status = 302; Location = 'http://evil.example.net/a'; Content = $null }
	'http://evil.example.net/a' = @{ Status = 200; Location = $null; Content = 'clear' }
}
$out = Invoke-Loop 'https://example.net/a'
Assert-True $out.Threw 'an https to http redirect is refused'
Assert-Eq 'the http address was never requested' 'https://example.net/a' ($script:Requested -join '|')

# The scheme is case-insensitive, so an upper-case one is the same refusal.
Reset-Stub @{
	'https://example.net/a' = @{ Status = 301; Location = 'HTTP://evil.example.net/a'; Content = $null }
}
$out = Invoke-Loop 'https://example.net/a'
Assert-True ($out.Threw -and ($script:Requested.Count -eq 1)) 'an upper-case HTTP redirect is refused'

# A redirect to http in the middle of a chain is refused too.
Reset-Stub @{
	'https://example.net/a' = @{ Status = 302; Location = 'https://example.net/b'; Content = $null }
	'https://example.net/b' = @{ Status = 307; Location = 'http://example.net/c'; Content = $null }
}
$out = Invoke-Loop 'https://example.net/a'
Assert-True ($out.Threw -and ($script:Requested.Count -eq 2)) 'an http hop later in the chain is refused'

# 3. A relative Location is resolved against the URL that returned it.
Reset-Stub @{
	'https://example.net/dir/a' = @{ Status = 302; Location = 'b'; Content = $null }
	'https://example.net/dir/b' = @{ Status = 302; Location = '/root/c'; Content = $null }
	'https://example.net/root/c' = @{ Status = 308; Location = '//cdn.example.net/d?x=1'; Content = $null }
	'https://cdn.example.net/d?x=1' = @{ Status = 200; Location = $null; Content = 'done' }
}
$out = Invoke-Loop 'https://example.net/dir/a'
Assert-True (-not $out.Threw) 'relative, root-relative and scheme-relative Locations are followed'
Assert-Eq 'each relative Location resolved against the previous URL' `
	'https://example.net/dir/a|https://example.net/dir/b|https://example.net/root/c|https://cdn.example.net/d?x=1' `
	($script:Requested -join '|')

# 4. The hop cap. A redirect loop is refused after exactly cap + 1 requests,
# and a chain of exactly the cap is still allowed.
Reset-Stub @{
	'https://example.net/loop' = @{ Status = 302; Location = '/loop'; Content = $null }
}
$out = Invoke-Loop 'https://example.net/loop' -MaxRedirects 3
Assert-True $out.Threw 'a redirect loop is refused'
Assert-Eq 'the loop stopped after the cap plus the first request' '4' ([string]$script:Requested.Count)
Assert-True ($out.Message -match 'Too many redirects') 'the refusal names the cause'

Reset-Stub @{
	'https://example.net/h0' = @{ Status = 302; Location = '/h1'; Content = $null }
	'https://example.net/h1' = @{ Status = 302; Location = '/h2'; Content = $null }
	'https://example.net/h2' = @{ Status = 302; Location = '/h3'; Content = $null }
	'https://example.net/h3' = @{ Status = 200; Location = $null; Content = 'ok' }
}
$out = Invoke-Loop 'https://example.net/h0' -MaxRedirects 3
Assert-True (-not $out.Threw) 'a chain of exactly the cap is allowed'
$out = Invoke-Loop 'https://example.net/h0' -MaxRedirects 2
Assert-True $out.Threw 'a chain one over the cap is refused'

# 5. Other refusals.
Reset-Stub @{ 'http://example.net/a' = @{ Status = 200; Location = $null; Content = 'clear' } }
$out = Invoke-Loop 'http://example.net/a'
Assert-True ($out.Threw -and ($script:Requested.Count -eq 0)) 'a start URL that is not https is refused before any request'

Reset-Stub @{ 'https://example.net/a' = @{ Status = 302; Location = ''; Content = $null } }
$out = Invoke-Loop 'https://example.net/a'
Assert-True $out.Threw 'a redirect with no Location is refused'

Reset-Stub @{ 'https://example.net/a' = @{ Status = 304; Location = 'https://example.net/b'; Content = $null } }
$out = Invoke-Loop 'https://example.net/a'
Assert-True ($out.Threw -and ($script:Requested.Count -eq 1)) 'a 3xx that is not a redirect is not followed'

# 6. Nothing else in the installer may call Invoke-WebRequest, and the
# one call that does must turn redirects off.
$calls = @($ast.FindAll({ param($n)
	$n -is [System.Management.Automation.Language.CommandAst] -and
	$n.GetCommandName() -eq 'Invoke-WebRequest'
}, $true))
Assert-Eq 'install.ps1 calls Invoke-WebRequest exactly once' '1' ([string]$calls.Count)
$outside = @($calls | Where-Object {
	$parent = $_.Parent
	while ($parent -and -not ($parent -is [System.Management.Automation.Language.FunctionDefinitionAst])) {
		$parent = $parent.Parent
	}
	-not ($parent -and $parent.Name -eq 'Invoke-HttpHop')
})
Assert-Eq 'that call sits inside Invoke-HttpHop' '0' ([string]$outside.Count)
Assert-True ($hopFn.Extent.Text -match 'MaximumRedirection\s*=\s*0\b') 'Invoke-HttpHop switches redirects off'

Write-Host ""
Write-Host "passed: $pass  failed: $fail"
if ($fail -eq 0) { exit 0 } else { exit 1 }
