<#
.SYNOPSIS
Delete stale remote branches of PodUtils/podnet with the gh CLI.

.DESCRIPTION
Give the branches with the verdict "delete" of docs/reports/2026-11-20-remote-branches.md
(or a newer report). The script prints each branch, asks once for a confirmation
(Read-Host) unless -Force is given, and deletes each branch with
`gh api -X DELETE repos/PodUtils/podnet/git/refs/heads/<name>`. A name that holds
"/" is URL-encoded.

The script refuses: main; a protected branch; a branch with an open pull request;
a branch with ahead_by greater than 0 (unless -AllowAhead is given); a branch that
does not exist. Only the gh CLI is used. gh must be logged in with a token that
can delete refs of the repository.

.PARAMETER Branch
The names of the branches to delete. With no name, the script does nothing.

.PARAMETER Force
Do not ask for the confirmation.

.PARAMETER WhatIf
Print what the script would delete. Delete nothing.

.PARAMETER AllowAhead
Delete a branch that has commits that main has not. Use it only when the last
commit of the branch is older than 30 days, and write the reason in the report.

.PARAMETER Repo
The repository. The default is PodUtils/podnet.

.EXAMPLE
pwsh scripts/agent/delete-stale-branches.ps1 -Branch old-branch,dependabot/cargo/x -WhatIf

.EXAMPLE
pwsh scripts/agent/delete-stale-branches.ps1 -Branch old-branch -Force
#>
param(
    [string[]]$Branch = @(),
    [switch]$Force,
    [switch]$WhatIf,
    [switch]$AllowAhead,
    [string]$Repo = 'PodUtils/podnet'
)

$ErrorActionPreference = 'Stop'

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    Write-Error 'gh is not on PATH.'
    exit 2
}

$Branch = @($Branch | Where-Object { $_ -and $_.Trim() } | ForEach-Object { $_.Trim() } | Select-Object -Unique)
if ($Branch.Count -eq 0) {
    Write-Host 'No branch given. Nothing to do.'
    exit 0
}

# Read-only call of the API. Return $null when the call fails (a 404).
function Get-Api([string]$Path) {
    $text = & gh api $Path 2>$null
    if ($LASTEXITCODE -ne 0) { return $null }
    return $text | ConvertFrom-Json
}

$default = (Get-Api "repos/$Repo").default_branch
if (-not $default) { $default = 'main' }

$todo = @()
foreach ($name in $Branch) {
    $enc = [uri]::EscapeDataString($name)
    if ($name -eq 'main' -or $name -eq $default) {
        Write-Warning "skip ${name}: the default branch is never deleted"
        continue
    }
    $info = Get-Api "repos/$Repo/branches/$enc"
    if (-not $info) {
        Write-Warning "skip ${name}: the branch does not exist"
        continue
    }
    if ($info.protected) {
        Write-Warning "skip ${name}: the branch is protected"
        continue
    }
    $owner = $Repo.Split('/')[0]
    $pulls = Get-Api "repos/$Repo/pulls?state=open&head=$([uri]::EscapeDataString("${owner}:$name"))"
    if ($pulls -and @($pulls).Count -gt 0) {
        Write-Warning "skip ${name}: an open pull request uses the branch (#$(@($pulls)[0].number))"
        continue
    }
    $cmp = Get-Api "repos/$Repo/compare/$default...$enc"
    if (-not $cmp) {
        Write-Warning "skip ${name}: the comparison with $default failed"
        continue
    }
    if ($cmp.ahead_by -gt 0 -and -not $AllowAhead) {
        Write-Warning "skip ${name}: $($cmp.ahead_by) commit(s) are not in $default (use -AllowAhead after the 30 day rule)"
        continue
    }
    Write-Host ("delete {0} (ahead_by {1}, behind_by {2})" -f $name, $cmp.ahead_by, $cmp.behind_by)
    $todo += , @($name, $enc)
}

if ($todo.Count -eq 0) {
    Write-Host 'No branch passed the checks. Nothing to delete.'
    exit 0
}

if ($WhatIf) {
    foreach ($pair in $todo) {
        Write-Host "What if: gh api -X DELETE repos/$Repo/git/refs/heads/$($pair[1])"
    }
    exit 0
}

if (-not $Force) {
    $answer = Read-Host "Delete $($todo.Count) branch(es) of $Repo? Type yes to go on"
    if ($answer -ne 'yes') {
        Write-Host 'Stopped. Nothing was deleted.'
        exit 1
    }
}

$failed = 0
foreach ($pair in $todo) {
    & gh api -X DELETE "repos/$Repo/git/refs/heads/$($pair[1])" | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Write-Host "deleted $($pair[0])"
    } else {
        Write-Warning "the delete of $($pair[0]) failed (exit $LASTEXITCODE)"
        $failed++
    }
}
exit ([int]($failed -gt 0))
