<#
.SYNOPSIS
    HUD（承認要求パネル）の見た目を任意のタイミングで確認するための PermissionRequest 投入ツール。

.DESCRIPTION
    Codex はこのプロジェクトが trust_level = "trusted" かつ sandbox = "elevated" のため、
    ほとんどの操作が承認なしで通ってしまい、実際の承認待ちを意図的に発生させるのが難しい。
    Claude Code 側も同様で、承認が要求される場面を狙って再現するのは難しい。

    本スクリプトは稼働中の Claude observer receiver へ合成した PermissionRequest hook を
    直接POSTし、HUDに承認要求パネルを表示させて見た目を確認できるようにする。
    `tools/screenkey-elicitation-probe.ps1`（WAITING_INPUT確認用）と同じ理由・同じ方式で
    作られたツールであり、observerの探索方法・Send-Hook関数・ヘッダコメントの書式を踏襲する。

    送信順:
        SessionStart -> UserPromptSubmit -> PermissionRequest
        （HoldSeconds 待機。この間にHUDを目視で確認する）
        PermissionDenied -> Stop -> SessionEnd

    observer.json は %LOCALAPPDATA%\Keylink Studio\data\claude-observer\<launch_id>\ にある。
    アプリが異常終了した分の残骸も残るため、ポートがLISTEN中のものだけを対象にする。

    PermissionRequest の body は `docs/claude-permission-hook-gate-results.md` §4 の実測を
    そのまま使う。session_id / cwd / prompt_id / permission_mode / hook_event_name /
    tool_name / tool_input{command, description} / permission_suggestions を含む。
    実測どおり tool_use_id は含めない（PendingApprovalStoreは(launch_id, session_id)で
    キーを作るため不要。`crates/rawhid-host-core/src/pending_approval.rs`のclaude_key参照）。

.PARAMETER List
    稼働中の launch を一覧表示して終了する。副作用なし。

.PARAMETER LaunchId
    対象 launch_id。省略時は稼働中で最も新しいものを使う。

.PARAMETER HoldSeconds
    PermissionRequest を維持する秒数（HUDを表示したままにする時間）。既定30秒。

.PARAMETER Command
    tool_input.command に入れる文字列。既定は短いサンプル。
    -Oversized 指定時は無視され、意図的に肥大化させた文字列で上書きされる。

.PARAMETER Description
    tool_input.description に入れる文字列。既定は短いサンプル。

.PARAMETER ToolName
    tool_name に入れる文字列。既定は "PowerShell"
    （このプロジェクトの環境ではシェル実行のtool_nameは"Bash"ではなく"PowerShell"。
    `docs/claude-permission-hook-gate-results.md` §4 補足）。

.PARAMETER Long
    Command / Description / cwd を、レイアウトの限界を確認するための長文サンプルに差し替える。
    -Command / -Description を明示的に指定した場合はそちらを優先する。
    中段（command/description）だけがスクロールし、見出しと選択肢が固定されるかを
    目視確認する用途。

.PARAMETER Oversized
    tool_input.command を 1 MiB を超える文字列にして送る。
    `crates/rawhid-host-core/src/pending_approval.rs` の MAX_PENDING_APPROVAL_BODY_BYTES
    (1 MiB) を PendingApprovalBody のシリアライズ後サイズが超えると、HUDの本文は保持されず
    Oversized マーカーに置き換わる（ui/src/hud/Hud.tsx の "内容が大きすぎるため表示できません"）。
    PendingApprovalBody は command を primary_text と full_command の2箇所に複製して
    保持するため、しきい値の半分（512KiB）強のcommandで超過する。
    一方、HTTP受信側の上限（`crates/rawhid-host-core/src/claude_observer.rs`の
    MAX_BODY_BYTES）も同じ1 MiBのため、command を1 MiBちょうど超えにすると
    リクエスト全体が413で弾かれてOversized表示まで届かない。
    そのためこのスイッチは command を約600KiBに設定する
    （複製後は約1.17MiBでPendingApprovalBody側のしきい値を超え、
    HTTPリクエスト全体は600KiB強でHTTP側の上限には収まる）。
    permission_suggestions.ruleContent は command を複製せず短いプレースホルダにする
    （command をそのまま複製するとHTTPリクエスト全体が1 MiBを超えてしまうため）。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\keylink-hud-preview-probe.ps1 -List

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\keylink-hud-preview-probe.ps1 -HoldSeconds 60

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\keylink-hud-preview-probe.ps1 -Long

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\keylink-hud-preview-probe.ps1 -Oversized
#>
[CmdletBinding()]
param(
    [switch]$List,
    [string]$LaunchId,
    [int]$HoldSeconds = 30,
    [string]$Command = 'Get-ChildItem -Path . -Filter *.log | Select-Object -First 5',
    [string]$Description = 'ログファイルの一覧を確認します',
    [string]$ToolName = 'PowerShell',
    [switch]$Long,
    [switch]$Oversized
)

$ErrorActionPreference = 'Stop'

function Get-ObserverRoot {
    Join-Path $env:LOCALAPPDATA 'Keylink Studio\data\claude-observer'
}

function Test-PortListening {
    param([int]$Port, [int]$TimeoutMs = 300)

    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $async = $client.BeginConnect('127.0.0.1', $Port, $null, $null)
        if (-not $async.AsyncWaitHandle.WaitOne($TimeoutMs, $false)) {
            return $false
        }
        $client.EndConnect($async)
        return $true
    } catch {
        return $false
    } finally {
        $client.Close()
    }
}

function Get-LiveObservers {
    $root = Get-ObserverRoot
    if (-not (Test-Path $root)) {
        return @()
    }

    $found = @()
    foreach ($dir in Get-ChildItem -Path $root -Directory) {
        $file = Join-Path $dir.FullName 'observer.json'
        if (-not (Test-Path $file)) { continue }

        try {
            $cfg = Get-Content -Path $file -Raw -Encoding UTF8 | ConvertFrom-Json
        } catch {
            continue
        }
        if (-not $cfg.endpoint -or -not $cfg.bearer_token) { continue }

        $port = ([uri]$cfg.endpoint).Port
        if (-not (Test-PortListening -Port $port)) { continue }

        $found += [pscustomobject]@{
            LaunchId = $cfg.launch_id
            Endpoint = $cfg.endpoint
            Token    = $cfg.bearer_token
            Port     = $port
            Updated  = $dir.LastWriteTime
        }
    }
    $found | Sort-Object Updated -Descending
}

function Send-Hook {
    param(
        [Parameter(Mandatory = $true)] $Observer,
        [Parameter(Mandatory = $true)] [hashtable]$Body
    )

    $json = $Body | ConvertTo-Json -Depth 6 -Compress

    # PowerShellのConvertTo-Jsonは要素1個の配列をスカラーに畳むことがある
    # （`docs/claude-permission-hook-gate-results.md` §7.2）。
    # permission_suggestions（とその中のrules）は配列である必要があるため、
    # 送信直前にシリアライズ後の文字列を見て配列表記になっているかを確認する。
    if ($Body.ContainsKey('permission_suggestions')) {
        if ($json -notmatch '"permission_suggestions"\s*:\s*\[') {
            throw 'permission_suggestions が配列としてシリアライズされていません（単一要素配列つぶれの疑い）。'
        }
        if ($json -notmatch '"rules"\s*:\s*\[') {
            throw 'permission_suggestions[].rules が配列としてシリアライズされていません（単一要素配列つぶれの疑い）。'
        }
    }

    $headers = @{ Authorization = "Bearer $($Observer.Token)" }
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
    Invoke-WebRequest -Method Post -Uri $Observer.Endpoint -Headers $headers `
        -ContentType 'application/json' -Body $bytes -TimeoutSec 5 -UseBasicParsing | Out-Null
    Write-Host ("  -> {0}" -f $Body['hook_event_name'])
}

$observers = Get-LiveObservers

if ($List) {
    if ($observers.Count -eq 0) {
        Write-Host '稼働中のClaude observerはありません。Keylink StudioからClaude Codeを起動してください。'
    } else {
        $observers | Format-Table LaunchId, Port, Updated -AutoSize
    }
    return
}

if ($observers.Count -eq 0) {
    throw '稼働中のClaude observerが見つかりません。Keylink StudioからClaude Codeを起動してから再実行してください。'
}

if ($LaunchId) {
    $target = $observers | Where-Object { $_.LaunchId -eq $LaunchId } | Select-Object -First 1
    if (-not $target) {
        throw "launch_id '$LaunchId' は稼働中のobserverに見つかりません。-List で確認してください。"
    }
} else {
    $target = $observers[0]
    if ($observers.Count -gt 1) {
        Write-Host ("稼働中のobserverが{0}件あります。最新を使用します（-List / -LaunchId で切替可能）。" -f $observers.Count)
    }
}

$SessionId = 'keylink-hud-preview-probe-' + [guid]::NewGuid().ToString('N').Substring(0, 12)
$promptId = [guid]::NewGuid().ToString()

# -Long: レイアウトの限界確認用の長文サンプル。
# -Command / -Description を明示的に指定した場合はそちらを優先する。
$longCommand = 'Get-ChildItem -Path "C:\01.keyboards\OriginalKeyboards\02.SW\Keylink-Studio\crates" -Recurse -Include "*.rs" | Where-Object { $_.Length -gt 20000 -and $_.Name -notlike "*generated*" } | Sort-Object Length -Descending | Select-Object -First 10 FullName, Length | Format-Table -AutoSize'
$longDescription = 'crates 配下の Rust ソースファイルをサイズの大きい順に並べ替えて一覧表示し、肥大化してレビューが難しくなっているファイルがないかを確認するためのコマンドです。実行結果はログに残して後で参照します。'
$longCwd = 'C:\01.keyboards\OriginalKeyboards\02.SW\Keylink-Studio\crates\rawhid-host-tauri\src\hud\preview\fixtures\long-path-for-layout-check\nested\deeper'

$effectiveCommand = $Command
$effectiveDescription = $Description
$effectiveCwd = (Get-Location).Path

if ($Long) {
    if (-not $PSBoundParameters.ContainsKey('Command')) { $effectiveCommand = $longCommand }
    if (-not $PSBoundParameters.ContainsKey('Description')) { $effectiveDescription = $longDescription }
    $effectiveCwd = $longCwd
}

$ruleContent = $effectiveCommand

if ($Oversized) {
    # PendingApprovalBody は command を primary_text と full_command の2箇所に複製するため、
    # しきい値(1 MiB)を超えるにはcommand単体が512KiB強あれば足りる。
    # 一方HTTPリクエスト全体の上限も1 MiBなので、command をそのまま複製した
    # permission_suggestions.ruleContent は付けない（付けるとHTTP層の413で弾かれてしまう）。
    $oversizedCommandBytes = 600 * 1024
    $effectiveCommand = 'X' * $oversizedCommandBytes
    $ruleContent = '<oversized probe: ruleContent omitted to stay under the HTTP body limit>'
    Write-Host ("-Oversized: tool_input.command を{0}バイトにします（複製後 約{1}バイトでPendingApprovalBodyのしきい値を超えます）。" -f $effectiveCommand.Length, ($effectiveCommand.Length * 2))
}

Write-Host ("launch_id  : {0}" -f $target.LaunchId)
Write-Host ("endpoint   : {0}" -f $target.Endpoint)
Write-Host ("session_id : {0}" -f $SessionId)
Write-Host ''

$cleanupNeeded = $false
try {
    Write-Host 'セッションを登録します...'
    Send-Hook -Observer $target -Body @{
        hook_event_name = 'SessionStart'
        session_id      = $SessionId
        source          = 'startup'
        cwd             = $effectiveCwd
    }
    $cleanupNeeded = $true

    Send-Hook -Observer $target -Body @{
        hook_event_name = 'UserPromptSubmit'
        session_id      = $SessionId
        prompt          = 'Keylink HUD preview probe'
    }

    Write-Host ''
    Write-Host 'PermissionRequest を送ります（HUDに承認要求パネルが表示されるか確認してください）...'
    Send-Hook -Observer $target -Body @{
        hook_event_name = 'PermissionRequest'
        session_id      = $SessionId
        cwd             = $effectiveCwd
        prompt_id       = $promptId
        permission_mode = 'acceptEdits'
        tool_name       = $ToolName
        tool_input      = @{
            command     = $effectiveCommand
            description = $effectiveDescription
        }
        permission_suggestions = @(
            @{
                type        = 'addRules'
                rules       = @(
                    @{ toolName = $ToolName; ruleContent = $ruleContent }
                )
                behavior    = 'allow'
                destination = 'localSettings'
            }
        )
    }

    Write-Host ''
    Write-Host ("{0}秒間 PermissionRequest を維持します..." -f $HoldSeconds)
    Start-Sleep -Seconds $HoldSeconds
} finally {
    if ($cleanupNeeded) {
        Write-Host ''
        Write-Host '後片付けします...'
        try {
            Send-Hook -Observer $target -Body @{
                hook_event_name = 'PermissionDenied'
                session_id      = $SessionId
            }
            Send-Hook -Observer $target -Body @{
                hook_event_name = 'Stop'
                session_id      = $SessionId
            }
            Send-Hook -Observer $target -Body @{
                hook_event_name = 'SessionEnd'
                session_id      = $SessionId
                reason          = 'other'
            }
            Write-Host '完了しました。'
        } catch {
            Write-Warning ("後片付けに失敗しました: {0}" -f $_.Exception.Message)
        }
    }
}
