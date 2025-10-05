# OAuth2 Playground Test Script
# Run this to test with your OAuth Playground credentials

$env:OAUTH_CLIENT_ID="AxIKkzEyzIqNUvLUvftnL57O"
$env:OAUTH_CLIENT_SECRET="iO5-nArlk4J_5bGjV1ags2UGvCs1ZdqMywLVV7VGk7ZPjhKF"
$env:OAUTH_AUTH_ENDPOINT="https://authorization-server.com/authorize"
$env:OAUTH_TOKEN_ENDPOINT="https://authorization-server.com/token"
$env:OAUTH_REDIRECT_URI="https://www.oauth.com/playground/authorization-code.html"
$env:OAUTH_USE_PKCE="true"

Write-Host "=== OAuth2 Playground Configuration ===" -ForegroundColor Green
Write-Host "Client ID: $env:OAUTH_CLIENT_ID"
Write-Host "Redirect URI: $env:OAUTH_REDIRECT_URI"
Write-Host "PKCE: $env:OAUTH_USE_PKCE"
Write-Host ""

Write-Host "Running interactive OAuth2 test..." -ForegroundColor Cyan
cargo run --example oauth2_interactive --quiet
