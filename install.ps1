#Requires -Version 5.1
#
# podup installer for Windows — downloads a release binary, verifies it and
# installs it.
#
# Usage:
#   irm https://glyndor.net/install/podup.ps1 | iex
#
# Environment:
#   PODUP_VERSION              Release tag to install (e.g. v0.3.0). Default: latest.
#   PODUP_INSTALL_DIR          Installation directory. Default: %LOCALAPPDATA%\Programs\podup.
#   PODUP_RELEASE_PUBKEY_B64   Override the baked-in Ed25519 release public key (for forks).
#   PODUP_INSECURE_SKIP_VERIFY Set to 1 to accept checksum-only verification.

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# PowerShell 7.3+ turns a non-zero native exit into a terminating error under
# ErrorActionPreference='Stop'. We branch on $LASTEXITCODE ourselves (a failed
# signature check is expected control flow, not a fatal error), so opt out.
# Harmless no-op on Windows PowerShell 5.1, which lacks this variable.
$PSNativeCommandUseErrorActionPreference = $false

$Repo = 'Glyndor/podup'
$Version = if ($env:PODUP_VERSION) { $env:PODUP_VERSION } else { 'latest' }
$InstallDir = if ($env:PODUP_INSTALL_DIR) { $env:PODUP_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\podup' }

function Write-LogInfo($msg)  { Write-Host "[info] $msg" -ForegroundColor Blue }
function Write-LogOk($msg)    { Write-Host "[ ok ] $msg" -ForegroundColor Green }
function Write-LogError($msg) { Write-Host "[fail] $msg" -ForegroundColor Red }
function Fail($msg) { Write-LogError $msg; exit 1 }

# --- Platform detection ------------------------------------------------------

$osArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
switch ($osArch) {
	'X64'   { $Arch = 'x86_64' }
	'Arm64' { $Arch = 'arm64' }
	default { Fail "Unsupported architecture: $osArch (supported: x86_64, arm64)" }
}

$Artifact = "podup-windows-$Arch.exe"

# --- Resolve download URL ----------------------------------------------------

if ($Version -eq 'latest') {
	$BaseUrl = "https://github.com/$Repo/releases/latest/download"
} elseif ($Version -match '^v[0-9]+\.[0-9]+\.[0-9]+$') {
	$BaseUrl = "https://github.com/$Repo/releases/download/$Version"
} else {
	Fail "PODUP_VERSION must be 'latest' or a semver tag like v1.2.3, got: $Version"
}

# Windows PowerShell 5.1 defaults to TLS 1.0/1.1; force TLS 1.2 for GitHub.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$TmpDir = New-Item -ItemType Directory -Path (Join-Path ([System.IO.Path]::GetTempPath()) ([System.IO.Path]::GetRandomFileName()))

try {
	# --- Download ------------------------------------------------------------

	function Get-ReleaseFile($name) {
		$dest = Join-Path $TmpDir $name
		$url = "$BaseUrl/$name"
		try {
			Invoke-WebRequest -Uri $url -OutFile $dest -UseBasicParsing
		} catch {
			Fail "Download failed: $url"
		}
		return $dest
	}

	Write-LogInfo "Downloading $Artifact ($Version) ..."
	$artifactPath = Get-ReleaseFile $Artifact
	$sumsPath = Get-ReleaseFile 'SHA256SUMS'
	$sigPath  = Get-ReleaseFile 'SHA256SUMS.sig'

	# --- Verify --------------------------------------------------------------

	# Checksum alone is not a trust anchor: a tampered release can ship a matching
	# SHA256SUMS. The binary is trusted only after at least one cryptographic proof
	# tied to the release key or the repository's build identity succeeds — the
	# Ed25519 signature over SHA256SUMS, or the GitHub build-provenance attestation.
	# If neither verifier can run, the install fails closed. Set
	# PODUP_INSECURE_SKIP_VERIFY=1 to explicitly opt out (checksum only).

	# Baked-in base64 (unpadded) raw Ed25519 public keys (32 bytes each) matching
	# the release signing key (RELEASE_SIGN_KEY). Up to two are accepted: the
	# second is empty except during a key rotation, when it holds the new key so a
	# release signed by either key verifies. The signature passes if any key
	# validates. Override for a fork via PODUP_RELEASE_PUBKEY_B64 / _PUBKEY2_B64.
	$PubKeyB64  = if ($env:PODUP_RELEASE_PUBKEY_B64) { $env:PODUP_RELEASE_PUBKEY_B64 } else { 'APh+kh61dJeT0HzG+KQXELzDjK4ccvqY9K+FptOZ3+Y' }
	$PubKey2B64 = if ($env:PODUP_RELEASE_PUBKEY2_B64) { $env:PODUP_RELEASE_PUBKEY2_B64 } else { '' }
	$PubKeys = @($PubKeyB64, $PubKey2B64 | Where-Object { $_ })

	$verified = $false

	# Locate a python interpreter that has the 'cryptography' package. Each
	# candidate carries any leading args (the 'py' launcher needs '-3').
	function Find-Python {
		$candidates = @(
			@{ Exe = 'python3'; Pre = @() },
			@{ Exe = 'python';  Pre = @() },
			@{ Exe = 'py';      Pre = @('-3') }
		)
		foreach ($c in $candidates) {
			if (-not (Get-Command $c.Exe -ErrorAction SilentlyContinue)) { continue }
			$probeArgs = $c.Pre + @('-c', 'from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey')
			& $c.Exe @probeArgs 2>$null
			if ($LASTEXITCODE -eq 0) { return $c }
		}
		return $null
	}

	Write-LogInfo 'Verifying SHA256SUMS signature ...'
	if ($PubKeys.Count -gt 0) {
		$python = Find-Python
		if ($python) {
			$pyScript = Join-Path $TmpDir 'verify_ed25519.py'
			# Python source — indentation is significant, keep as-is.
			$pySource = @'
import base64, sys
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from cryptography.exceptions import InvalidSignature
sig_file, data_file = sys.argv[1], sys.argv[2]
sig = open(sig_file, "rb").read()
data = open(data_file, "rb").read()
for pubkey_b64 in sys.argv[3:]:
    try:
        Ed25519PublicKey.from_public_bytes(base64.b64decode(pubkey_b64 + "==")).verify(sig, data)
        sys.exit(0)
    except (InvalidSignature, ValueError):
        continue
sys.exit(1)
'@
			Set-Content -Path $pyScript -Value $pySource -Encoding ASCII
			$pyArgs = $python.Pre + @($pyScript, $sigPath, $sumsPath) + $PubKeys
			& $python.Exe @pyArgs
			if ($LASTEXITCODE -eq 0) {
				Write-LogOk 'SHA256SUMS signature verified'
				$verified = $true
			} else {
				Fail 'SHA256SUMS signature verification failed — release may be tampered'
			}
		} else {
			Write-LogInfo 'python3+cryptography not available — cannot check Ed25519 signature'
		}
	} else {
		Write-LogInfo 'no release public key configured — skipping Ed25519 signature check'
	}

	# Build-provenance attestation: proves the binary was produced by this repo's
	# release workflow (GitHub OIDC). Strong even without the release public key.
	$ghAttestation = $false
	if (Get-Command gh -ErrorAction SilentlyContinue) {
		& gh attestation --help *> $null
		if ($LASTEXITCODE -eq 0) { $ghAttestation = $true }
	}
	if ($ghAttestation) {
		Write-LogInfo 'Verifying artifact attestation ...'
		& gh attestation verify $artifactPath --repo $Repo | Out-Null
		if ($LASTEXITCODE -ne 0) { Fail "Attestation verification failed for $Artifact" }
		Write-LogOk 'Attestation verified'
		$verified = $true
	} else {
		Write-LogInfo 'GitHub CLI with attestation support not found — cannot check attestation'
	}

	# Fail closed unless a strong proof succeeded or the user explicitly opts out.
	if (-not $verified) {
		if ($env:PODUP_INSECURE_SKIP_VERIFY -eq '1') {
			Write-LogInfo 'PODUP_INSECURE_SKIP_VERIFY=1 — proceeding with checksum verification only'
		} else {
			Fail "No signature or attestation verifier available. Install 'gh' (>= 2.49) or python3 with the 'cryptography' package, set PODUP_RELEASE_PUBKEY_B64, or re-run with PODUP_INSECURE_SKIP_VERIFY=1 to accept checksum-only verification."
		}
	}

	Write-LogInfo 'Verifying SHA-256 checksum ...'
	$expectedLine = Select-String -Path $sumsPath -Pattern ("\s" + [regex]::Escape($Artifact) + "$") | Select-Object -First 1
	if (-not $expectedLine) { Fail "No checksum entry for $Artifact in SHA256SUMS" }
	$expected = ($expectedLine.Line -split '\s+')[0].ToLower()
	$actual = (Get-FileHash -Path $artifactPath -Algorithm SHA256).Hash.ToLower()
	if ($expected -ne $actual) { Fail "Checksum verification failed for $Artifact" }
	Write-LogOk 'Checksum verified'

	# --- Install -------------------------------------------------------------

	if (-not (Test-Path $InstallDir)) {
		New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
	}
	$target = Join-Path $InstallDir 'podup.exe'
	Copy-Item -Path $artifactPath -Destination $target -Force

	# Add the install dir to the user PATH if it is not already there.
	$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
	$onPath = ($userPath -split ';') -contains $InstallDir
	if (-not $onPath) {
		$newPath = if ([string]::IsNullOrEmpty($userPath)) { $InstallDir } else { "$userPath;$InstallDir" }
		[Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
		$env:Path = "$env:Path;$InstallDir"
		Write-LogInfo "Added $InstallDir to your user PATH (restart your shell to pick it up)"
	}

	$installed = & $target --version
	Write-LogOk "podup installed: $installed"
} finally {
	Remove-Item -Path $TmpDir -Recurse -Force -ErrorAction SilentlyContinue
}
