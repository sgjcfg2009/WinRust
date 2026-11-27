# >>> topbar >>>
# managed by plugin-launcher: removed together with topbar
if (Get-Command zoxide -ErrorAction SilentlyContinue) { Invoke-Expression (& { (zoxide init powershell | Out-String) }) }
if (Get-Command rg -ErrorAction SilentlyContinue) { Set-Alias grep rg }
if (Get-Command eza -ErrorAction SilentlyContinue) { function ll { eza -la --git @args } }
if (Get-Command bat -ErrorAction SilentlyContinue) { Set-Alias cat bat -Option AllScope -Force; function cats { bat @args } }
# <<< topbar <<<
