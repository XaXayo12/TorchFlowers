<#
.SYNOPSIS
    Authenticates a Microsoft account with TorchFlower and saves the
    ProvisionedBedrockSession JSON that torchflower-lite-bot needs.

.PARAMETER Email
    Microsoft account e-mail address (mandatory).

.PARAMETER BotId
    Identifier used as the output filename: $OutDir/$BotId.json.
    Defaults to "donutsmp-bot".

.PARAMETER OutDir
    Directory where the session JSON is written.
    Defaults to ".torchflower/accounts".

.EXAMPLE
    .\scripts\login-and-save.ps1 -Email "your@email.com"

.EXAMPLE
    .\scripts\login-and-save.ps1 -Email "your@email.com" -BotId "my-bot" -OutDir "sessions"
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$Email,

    [string]$BotId  = 'donutsmp-bot',
    [string]$OutDir = '.torchflower/accounts'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# ---------------------------------------------------------------------------
# 1. Load .env from the current directory
# ---------------------------------------------------------------------------
$envFile = Join-Path (Get-Location) '.env'
if (Test-Path $envFile) {
    Get-Content $envFile | Where-Object { $_ -match '^\s*[^#]' -and $_ -match '=' } | ForEach-Object {
        # Split on the FIRST '=' only so base64 values with trailing '=' survive.
        $parts = $_ -split '=', 2
        if ($parts.Count -eq 2) {
            $key   = $parts[0].Trim()
            $value = $parts[1].Trim()
            [System.Environment]::SetEnvironmentVariable($key, $value, 'Process')
        }
    }
    Write-Host "[env] Loaded .env" -ForegroundColor DarkGray
} else {
    Write-Warning "[env] .env file not found - falling back to existing environment variables."
}

# ---------------------------------------------------------------------------
# 2. Resolve engine base URL and API key from environment
# ---------------------------------------------------------------------------
$bindAddr = [System.Environment]::GetEnvironmentVariable('RUST_ENGINE_BIND')
if (-not $bindAddr) { $bindAddr = '127.0.0.1:9080' }
$baseUrl  = "http://$bindAddr/api"

$apiKey   = [System.Environment]::GetEnvironmentVariable('TORCHFLOWER_API_KEY')
$headers  = @{ 'Content-Type' = 'application/json' }
if ($apiKey) { $headers['Authorization'] = "Bearer $apiKey" }

Write-Host ""
Write-Host "TorchFlower Login Helper" -ForegroundColor Cyan
Write-Host "  Engine : $baseUrl" -ForegroundColor DarkGray
Write-Host "  Email  : $Email"   -ForegroundColor DarkGray
Write-Host "  BotId  : $BotId"   -ForegroundColor DarkGray
Write-Host ""

# ---------------------------------------------------------------------------
# 3. POST /api/accounts -- start device auth
# ---------------------------------------------------------------------------
Write-Host "[1/4] Starting device authentication..." -ForegroundColor Yellow
$body    = @{ email = $Email } | ConvertTo-Json
$startResp = Invoke-RestMethod `
    -Uri     "$baseUrl/accounts" `
    -Method  Post `
    -Headers $headers `
    -Body    $body

$sessionId = $startResp.session.id
$verifyUri = $startResp.session.verification_uri
$userCode  = $startResp.session.user_code
$accountId = $startResp.session.account_id

Write-Host ""
Write-Host "  +--------------------------------------------------+" -ForegroundColor Green
Write-Host "  |  Open this URL in your browser:                  |" -ForegroundColor Green
Write-Host "  |  $verifyUri" -ForegroundColor Green
Write-Host "  |                                                   |" -ForegroundColor Green
Write-Host "  |  Enter code:  $userCode                              |" -ForegroundColor Green
Write-Host "  +--------------------------------------------------+" -ForegroundColor Green
Write-Host ""
Write-Host "Waiting for you to sign in..." -ForegroundColor Yellow

# ---------------------------------------------------------------------------
# 4. Poll POST /api/auth/sessions/{session_id}/poll every 5 seconds
# ---------------------------------------------------------------------------
Write-Host "[2/4] Polling for login completion (every 5 s)..." -ForegroundColor Yellow
$pollUrl = "$baseUrl/auth/sessions/$sessionId/poll"
$status  = 'pending'

while ($status -eq 'pending') {
    Start-Sleep -Seconds 5
    try {
        $pollResp = Invoke-RestMethod `
            -Uri     $pollUrl `
            -Method  Post `
            -Headers $headers
        $status    = $pollResp.status
        $accountId = if ($pollResp.account.id) { $pollResp.account.id } else { $accountId }
    } catch {
        # 4xx from a transient network blip - keep polling
        Write-Host "  (poll error - retrying: $_)" -ForegroundColor DarkGray
    }
    Write-Host "  Status: $status" -ForegroundColor DarkGray
}

if ($status -ne 'authenticated') {
    Write-Error "Login did not complete successfully. Final status: $status"
    exit 1
}

Write-Host "[2/4] Authenticated!" -ForegroundColor Green

# ---------------------------------------------------------------------------
# 5. GET /api/accounts/{account_id}/session -- export ProvisionedBedrockSession
# ---------------------------------------------------------------------------
Write-Host "[3/4] Fetching provisioned Bedrock session..." -ForegroundColor Yellow
$sessionData = Invoke-RestMethod `
    -Uri     "$baseUrl/accounts/$accountId/session" `
    -Method  Get `
    -Headers $headers

Write-Host "[3/4] Session exported." -ForegroundColor Green

# ---------------------------------------------------------------------------
# 6. Save JSON -- utf8NoBOM so the Rust JSON parser does not choke on the BOM
# ---------------------------------------------------------------------------
Write-Host "[4/4] Saving session file..." -ForegroundColor Yellow

if (-not (Test-Path $OutDir)) {
    New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
}

$outFile = Join-Path $OutDir "$BotId.json"
$sessionData | ConvertTo-Json -Depth 20 | Set-Content -Path $outFile -Encoding utf8NoBOM

Write-Host "[4/4] Saved to: $outFile" -ForegroundColor Green

# ---------------------------------------------------------------------------
# 7. Next-step instructions
# ---------------------------------------------------------------------------
Write-Host ""
Write-Host "=====================================================" -ForegroundColor Cyan
Write-Host " Login complete! Next steps:" -ForegroundColor Cyan
Write-Host ""
Write-Host "  1. Make sure your bots.toml has the correct account_id:"
Write-Host "       account_id = `"$BotId`"" -ForegroundColor White
Write-Host ""
Write-Host "  2. Run the lite-bot from the workspace root:"
Write-Host "       torchflower-lite-bot run --config bots.toml" -ForegroundColor White
Write-Host ""
Write-Host "  To re-authenticate later, just re-run this script."
Write-Host "=====================================================" -ForegroundColor Cyan
Write-Host ""
