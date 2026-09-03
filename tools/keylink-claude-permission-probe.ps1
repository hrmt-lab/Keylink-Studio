<#
.SYNOPSIS
    Claude Code の PermissionRequest hook（type: http）が「レスポンスをブロックして
    決定を返す」ことを許容するかどうかを、実機のClaude Codeに対して検証するための
    独立プローブ。

.DESCRIPTION
    Keylink Studio は Claude Code のイベントを observer として受信しているだけで、
    PermissionRequest hookに対しては常に 204 No Content を即座に返している
    （crates/rawhid-host-core/src/claude_observer.rs の handle_connection、
    および crates/rawhid-host-core/src/claude_hooks.rs の hooks_json を参照）。
    つまり Studio 自身は「決定を返す」実装を一切持っていない。

    このスクリプトは Studio のコードには一切手を入れず、使い捨てのテスト用プロジェクト
    ディレクトリに独自の .claude/settings.json を書き、そこで宣言した PermissionRequest
    http hook の宛先として、このスクリプト自身が System.Net.HttpListener で起動する
    ローカルHTTPサーバを指定する。

    人間が別ターミナルでそのプロジェクトディレクトリに入り claude を起動し、承認が
    必要な操作を依頼すると、Claude Code がこのサーバへ PermissionRequest hookを
    POSTしてくる。受信内容を全文ログへ記録したうえで、指定秒数待ってから、指定モードの
    レスポンス（204空、または 200 + allow/deny 決定JSON）を返し、Claude Code / ターミナル
    がどう振る舞うかを人間が目視で確認する。

    モードは5種類:
        Observe     : 204を待たずに即返す（現在のStudioと同じ挙動。ベースライン）
        Block       : 204をDelaySeconds待ってから返す（決定は返さない）
        Allow       : 200 + {"behavior":"allow"} をDelaySeconds待ってから返す
        Deny        : 200 + {"behavior":"deny", message} をDelaySeconds待ってから返す
        AllowAlways : 200 + allow。受信bodyに permission_suggestions があれば
                      updatedPermissions としてそのまま載せる

    Ctrl+C で終了するまで待ち受け続けるため、同一起動で複数回リクエストを受けられる。
    待受は GetContextAsync() を200ms間隔でポーリングする方式で実装しており、これは
    単純化ではなく必須の実装（理由は実装コード側のコメントを参照）。1回のリクエストだけ
    測って自動終了したい場合は -MaxRequests 1 を使うとCtrl+Cを押さずに済む。

.PARAMETER Mode
    レスポンスの種類。Observe / Block / Allow / Deny / AllowAlways のいずれか。既定は Block。

.PARAMETER DelaySeconds
    リクエスト受信からレスポンス送出までの待機秒数。既定は10秒。
    Observeモードのみこの待機を無視して即時応答する（現在のStudioの挙動を再現するため）。

.PARAMETER HookTimeout
    生成する settings.json に書き込む hookの "timeout"（秒）。既定は120秒。
    Studio本番の PermissionRequest hookは1秒だが、このプローブでは意図的に長い
    DelaySeconds を試せるよう既定値を大きくしてある。

.PARAMETER ProjectDir
    テスト用プロジェクトディレクトリ。省略時は %TEMP% 配下に自動生成する。

.PARAMETER Port
    ローカルHTTPサーバの待受ポート。省略時（0）は空きポートを自動選択する。

.PARAMETER List
    何もせず（プロジェクトディレクトリの作成やサーバ起動を行わず）、生成予定の
    settings.json の内容とパスだけを表示して終了する。

.PARAMETER MaxRequests
    この件数のリクエストを処理したら自動的にサーバを終了する。既定は0（無制限、
    Ctrl+Cで手動終了）。1件だけ測りたいときは -MaxRequests 1 を指定するとCtrl+Cを
    押す必要がなくなる。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\keylink-claude-permission-probe.ps1 -List

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\keylink-claude-permission-probe.ps1 -Mode Block -DelaySeconds 15

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File tools\keylink-claude-permission-probe.ps1 -Mode Allow -DelaySeconds 0 -MaxRequests 1
#>
[CmdletBinding()]
param(
    [ValidateSet('Observe', 'Block', 'Allow', 'Deny', 'AllowAlways')]
    [string]$Mode = 'Block',

    [int]$DelaySeconds = 10,

    [int]$HookTimeout = 120,

    [string]$ProjectDir,

    [int]$Port = 0,

    [switch]$List,

    [int]$MaxRequests = 0
)

$ErrorActionPreference = 'Stop'

function Get-FreeTcpPort {
    $probe = New-Object System.Net.Sockets.TcpListener([System.Net.IPAddress]::Loopback, 0)
    try {
        $probe.Start()
        return $probe.LocalEndpoint.Port
    } finally {
        $probe.Stop()
    }
}

function Get-Iso8601Ms {
    (Get-Date).ToString('yyyy-MM-ddTHH:mm:ss.fffK')
}

function Format-JsonField {
    param(
        $Value,
        [switch]$AsJson
    )
    if ($null -eq $Value) {
        return '(absent)'
    }
    if ($AsJson) {
        try {
            return ($Value | ConvertTo-Json -Depth 20)
        } catch {
            return [string]$Value
        }
    }
    return [string]$Value
}

function Get-JsonProperty {
    param(
        $Object,
        [string]$Name
    )
    if ($null -eq $Object) {
        return $null
    }
    if ($Object -is [System.Management.Automation.PSCustomObject] -and
        ($Object.PSObject.Properties.Name -contains $Name)) {
        return $Object.$Name
    }
    return $null
}

function Get-RawJsonValue {
    <#
        受信した生のJSON本文から、トップレベルキー $Key の値部分を「ConvertFrom-Json/
        ConvertTo-Jsonの往復を一切せず」に文字列としてそのまま切り出す。

        目的: PowerShell 5.1 の ConvertFrom-Json / ConvertTo-Json は要素数1個の配列を
        スカラーへ畳んでしまう既知の問題があり、往復させると "permission_suggestions"
        のような配列が壊れる（実機で確認済み）。オブジェクトを経由せず原文の該当箇所を
        そのまま部分文字列として取り出すことで、配列・null・深いネスト・キー順序を
        原文どおりに保つ。

        実装: キー名（"key"）の直後の ':' を見つけ、値の先頭文字が '{' または '[' なら
        対応する閉じ括弧までを、文字列リテラル内（エスケープ \" を考慮）は無視しながら
        深さカウントで特定する。値が文字列ならその閉じクオートまで、それ以外
        （数値/true/false/null）なら区切り文字（, } ] または空白）までを値とみなす。
        見つからない/対応が取れない場合は $null を返す。
    #>
    param(
        [string]$Json,
        [string]$Key
    )
    if ([string]::IsNullOrEmpty($Json)) {
        return $null
    }

    $needle = '"' + $Key + '"'
    $keyIndex = $Json.IndexOf($needle)
    if ($keyIndex -lt 0) {
        return $null
    }

    $cursor = $keyIndex + $needle.Length
    while ($cursor -lt $Json.Length -and [char]::IsWhiteSpace($Json[$cursor])) {
        $cursor++
    }
    if ($cursor -ge $Json.Length -or $Json[$cursor] -ne ':') {
        return $null
    }
    $cursor++
    while ($cursor -lt $Json.Length -and [char]::IsWhiteSpace($Json[$cursor])) {
        $cursor++
    }
    if ($cursor -ge $Json.Length) {
        return $null
    }

    $valueStart = $cursor
    $firstChar = $Json[$cursor]

    if ($firstChar -eq '{' -or $firstChar -eq '[') {
        $openChar = $firstChar
        if ($openChar -eq '{') {
            $closeChar = '}'
        } else {
            $closeChar = ']'
        }
        $depth = 0
        $inString = $false
        $escaped = $false
        for ($i = $cursor; $i -lt $Json.Length; $i++) {
            $ch = $Json[$i]
            if ($inString) {
                if ($escaped) {
                    $escaped = $false
                } elseif ($ch -eq '\') {
                    $escaped = $true
                } elseif ($ch -eq '"') {
                    $inString = $false
                }
                continue
            }
            if ($ch -eq '"') {
                $inString = $true
                continue
            }
            if ($ch -eq $openChar) {
                $depth++
            } elseif ($ch -eq $closeChar) {
                $depth--
                if ($depth -eq 0) {
                    return $Json.Substring($valueStart, $i - $valueStart + 1)
                }
            }
        }
        # 対応する閉じ括弧が見つからなかった（JSONが壊れている等）。
        return $null
    } elseif ($firstChar -eq '"') {
        $escaped = $false
        for ($i = $cursor + 1; $i -lt $Json.Length; $i++) {
            $ch = $Json[$i]
            if ($escaped) {
                $escaped = $false
                continue
            }
            if ($ch -eq '\') {
                $escaped = $true
                continue
            }
            if ($ch -eq '"') {
                return $Json.Substring($valueStart, $i - $valueStart + 1)
            }
        }
        return $null
    } else {
        # 数値 / true / false / null。区切り文字か空白までを値とみなす。
        $i = $cursor
        while ($i -lt $Json.Length -and
            $Json[$i] -ne ',' -and $Json[$i] -ne '}' -and $Json[$i] -ne ']' -and
            -not [char]::IsWhiteSpace($Json[$i])) {
            $i++
        }
        if ($i -eq $valueStart) {
            return $null
        }
        return $Json.Substring($valueStart, $i - $valueStart)
    }
}

function Get-ProbeResponse {
    param(
        [string]$Mode,
        # NOTE: 型を [string] にしないこと。[string] にすると $null を渡した際に
        # PowerShellが空文字列 '' へ暗黙変換してしまい、下の「見つからなかった」判定
        # ($null -ne $PermissionSuggestionsRaw) が常に真になってしまう
        # （実際にこの型注釈のバグを単体テストで踏んだ。$null と '' は別物として
        # 扱う必要があるため、意図的に無型のままにしている）。
        $PermissionSuggestionsRaw
    )

    switch ($Mode) {
        'Observe' {
            return @{ StatusCode = 204; BodyText = $null }
        }
        'Block' {
            return @{ StatusCode = 204; BodyText = $null }
        }
        'Allow' {
            $payload = [ordered]@{
                hookSpecificOutput = [ordered]@{
                    hookEventName = 'PermissionRequest'
                    decision      = [ordered]@{
                        behavior = 'allow'
                    }
                }
            }
            return @{ StatusCode = 200; BodyText = ($payload | ConvertTo-Json -Depth 20) }
        }
        'Deny' {
            $payload = [ordered]@{
                hookSpecificOutput = [ordered]@{
                    hookEventName = 'PermissionRequest'
                    decision      = [ordered]@{
                        behavior = 'deny'
                        message  = 'KO-3 probe denied this request.'
                    }
                }
            }
            return @{ StatusCode = 200; BodyText = ($payload | ConvertTo-Json -Depth 20) }
        }
        'AllowAlways' {
            # NOTE: ここだけは ConvertTo-Json で組み立てない。permission_suggestions を
            # 一度 ConvertFrom-Json でPowerShellオブジェクト化してから ConvertTo-Json で
            # 組み直すと、要素数1個の配列がスカラーに畳まれて壊れる（PowerShell 5.1の
            # 既知の挙動。実機で確認済み）。そのため Get-RawJsonValue で切り出した
            # 生のJSON文字列をそのまま文字列連結で埋め込む。
            if (-not [string]::IsNullOrEmpty($PermissionSuggestionsRaw)) {
                $bodyText = '{' + "`n" +
                    '  "hookSpecificOutput": {' + "`n" +
                    '    "hookEventName": "PermissionRequest",' + "`n" +
                    '    "decision": {' + "`n" +
                    '      "behavior": "allow",' + "`n" +
                    '      "updatedPermissions": ' + $PermissionSuggestionsRaw + "`n" +
                    '    }' + "`n" +
                    '  }' + "`n" +
                    '}'
                return @{ StatusCode = 200; BodyText = $bodyText; UpdatedPermissionsFound = $true }
            }
            $payload = [ordered]@{
                hookSpecificOutput = [ordered]@{
                    hookEventName = 'PermissionRequest'
                    decision      = [ordered]@{
                        behavior = 'allow'
                    }
                }
            }
            return @{ StatusCode = 200; BodyText = ($payload | ConvertTo-Json -Depth 20); UpdatedPermissionsFound = $false }
        }
    }
}

# --- パス・ポートの決定 ---------------------------------------------------

if (-not $ProjectDir) {
    $ProjectDir = Join-Path $env:TEMP ('keylink-claude-permission-probe-' + [guid]::NewGuid().ToString('N').Substring(0, 10))
}
$ProjectDir = [System.IO.Path]::GetFullPath($ProjectDir)

if ($Port -eq 0) {
    $Port = Get-FreeTcpPort
}

$claudeDir = Join-Path $ProjectDir '.claude'
$settingsPath = Join-Path $claudeDir 'settings.json'
$logPath = Join-Path $ProjectDir 'permission-probe.log'
$hookUrl = "http://127.0.0.1:$Port/permission"
$listenPrefix = "http://127.0.0.1:$Port/"

$settingsObject = [ordered]@{
    hooks = [ordered]@{
        PermissionRequest = @(
            [ordered]@{
                matcher = '*'
                hooks   = @(
                    [ordered]@{
                        type    = 'http'
                        url     = $hookUrl
                        timeout = $HookTimeout
                    }
                )
            }
        )
    }
}
$settingsJson = $settingsObject | ConvertTo-Json -Depth 10

# --- -List: 何も作らず表示だけして終了 -------------------------------------

if ($List) {
    Write-Host '=== -List: 生成予定の内容（何も作成/起動していません） ==='
    Write-Host ("ProjectDir    : {0}" -f $ProjectDir)
    Write-Host ("settings.json : {0}" -f $settingsPath)
    Write-Host ("log file      : {0}" -f $logPath)
    Write-Host ("hook URL      : {0}" -f $hookUrl)
    Write-Host ("Mode          : {0}" -f $Mode)
    Write-Host ("DelaySeconds  : {0}" -f $DelaySeconds)
    Write-Host ("HookTimeout   : {0}" -f $HookTimeout)
    Write-Host ''
    Write-Host '--- settings.json ---'
    Write-Host $settingsJson
    return
}

# --- プロジェクトディレクトリと settings.json を用意 ------------------------

New-Item -ItemType Directory -Force -Path $claudeDir | Out-Null

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($settingsPath, $settingsJson + "`n", $utf8NoBom)

# NOTE: ログは削除せず追記する。-MaxRequests 1 で1件ずつ測るため、1つの調査は
# 「同じ ProjectDir に対してモードを変えて複数回起動する」形になる。起動のたびに
# 削除すると直前の実行の記録が消え、実機では実際に AllowAlways の要求と応答を
# 失って検証をやり直す羽目になった。追記にしたうえで、実行ごとの区切りを入れる。
if (-not (Test-Path $logPath)) {
    New-Item -ItemType File -Path $logPath -Force | Out-Null
}

function Write-ProbeLog {
    param([string[]]$Lines)
    $Lines | Out-File -FilePath $logPath -Append -Encoding utf8
    $Lines | ForEach-Object { Write-Host $_ }
}

# 追記式なので、どのリクエストがどの起動によるものかを後から追えるようにする。
Write-ProbeLog @(
    '',
    '################################################################',
    ("# probe run started : {0}" -f (Get-Iso8601Ms)),
    ("# Mode              : {0}" -f $Mode),
    ("# DelaySeconds      : {0}" -f $DelaySeconds),
    ("# HookTimeout       : {0}" -f $HookTimeout),
    ("# MaxRequests       : {0}" -f $MaxRequests),
    ("# hook URL          : {0}" -f $hookUrl),
    '################################################################'
)

# --- 起動案内 ---------------------------------------------------------------

Write-Host '======================================================================='
Write-Host ' Claude Code PermissionRequest hook プローブ'
Write-Host '======================================================================='
Write-Host ("プロジェクトディレクトリ : {0}" -f $ProjectDir)
Write-Host ("待ち受けURL              : {0}" -f $hookUrl)
Write-Host ("ログファイル             : {0}" -f $logPath)
Write-Host ("モード                   : {0}" -f $Mode)
Write-Host ("応答までの待機秒数       : {0}" -f $DelaySeconds)
Write-Host ("settings.json のhook timeout: {0}" -f $HookTimeout)
if ($MaxRequests -gt 0) {
    Write-Host ("自動終了                 : {0}件処理したら終了します" -f $MaxRequests)
} else {
    Write-Host '自動終了                 : 無効（Ctrl+Cで手動終了してください）'
}
Write-Host ''
Write-Host '実行手順:'
Write-Host '  1. 別のターミナルを開いてください'
Write-Host ("  2. cd `"{0}`"" -f $ProjectDir)
Write-Host '  3. claude を起動してください'
Write-Host '  4. 承認が必要な操作を頼んでください（例: Bash で git status を実行させる、など）'
Write-Host '  5. ターミナルに何が表示されるかをよく観察してください'
Write-Host ''
Write-Host '注意:'
Write-Host '  - Keylink Studio が監視中だと Studio 側の hook も同時に発火する可能性があるため、'
Write-Host '    Studio の監視を停止してから実施することを推奨します。'
Write-Host '  - .claude/settings.json の hook 使用について Claude Code が確認を求めた場合は許可してください。'
Write-Host ''
Write-Host 'Ctrl+C で終了します（このウィンドウはサーバとして待ち受け続けます）。'
Write-Host '======================================================================='
Write-Host ''

# --- HTTPサーバ本体 ----------------------------------------------------------

$listener = New-Object System.Net.HttpListener
$listener.Prefixes.Add($listenPrefix)

$startTime = Get-Date
$requestCount = 0
$heartbeatIntervalSeconds = 10

try {
    try {
        $listener.Start()
    } catch {
        Write-Error ("HttpListener の起動に失敗しました（{0}）: {1}" -f $listenPrefix, $_.Exception.Message)
        throw
    }

    $lastHeartbeat = Get-Date

    while ($true) {
        # NOTE: 意図的に GetContext()（同期ブロッキング呼び出し）ではなく
        # GetContextAsync() + 短い待機のポーリングを使っている。
        # GetContext() は呼び出しスレッドを完全にブロックし、リクエストが来るまで
        # 制御が戻らない。PowerShellはCtrl+C（停止要求）を「文の境界」でしか
        # 処理できないため、GetContext()でブロック中はCtrl+Cが一切効かず、
        # try/finallyのfinallyにも到達できない（＝リスナーを閉じられず、
        # ターミナルを閉じるまで終了できない）。実機で確認済みの不具合のため、
        # 「単純だから」という理由でGetContext()に戻さないこと。
        # WaitOne(200)で200msごとにループへ戻すことで、PowerShellがCtrl+Cを
        # 処理できる安全地点を定期的に作っている。
        $task = $listener.GetContextAsync()
        while (-not $task.AsyncWaitHandle.WaitOne(200)) {
            $now = Get-Date
            if (((New-TimeSpan -Start $lastHeartbeat -End $now).TotalSeconds) -ge $heartbeatIntervalSeconds) {
                Write-Host 'リクエスト待機中... (Ctrl+C で終了)'
                $lastHeartbeat = $now
            }
        }

        try {
            $context = $task.GetAwaiter().GetResult()
        } catch [System.Net.HttpListenerException] {
            break
        } catch [System.ObjectDisposedException] {
            break
        }
        $lastHeartbeat = Get-Date

        $requestCount++
        $receivedAt = Get-Date
        $receivedAtIso = Get-Iso8601Ms
        $elapsedMs = [Math]::Round(((New-TimeSpan -Start $startTime -End $receivedAt).TotalMilliseconds), 1)

        $request = $context.Request
        $reader = New-Object System.IO.StreamReader($request.InputStream, [System.Text.Encoding]::UTF8)
        $rawBody = $reader.ReadToEnd()
        $reader.Close()

        $headerLines = @()
        foreach ($key in $request.Headers.AllKeys) {
            $headerLines += ("  {0}: {1}" -f $key, $request.Headers[$key])
        }
        if ($headerLines.Count -eq 0) {
            $headerLines = @('  (none)')
        }

        $parsedBody = $null
        if ([string]::IsNullOrEmpty($rawBody)) {
            $prettyBody = '(empty)'
        } else {
            try {
                $parsedBody = $rawBody | ConvertFrom-Json -ErrorAction Stop
                $prettyBody = $parsedBody | ConvertTo-Json -Depth 20
            } catch {
                $prettyBody = $rawBody
            }
        }

        $hookEventName = Format-JsonField (Get-JsonProperty -Object $parsedBody -Name 'hook_event_name')
        $toolName = Format-JsonField (Get-JsonProperty -Object $parsedBody -Name 'tool_name')
        $toolInputFormatted = Format-JsonField -AsJson (Get-JsonProperty -Object $parsedBody -Name 'tool_input')
        $permissionSuggestionsFormatted = Format-JsonField -AsJson (Get-JsonProperty -Object $parsedBody -Name 'permission_suggestions')
        $suppressAlwaysAllowRule = Format-JsonField (Get-JsonProperty -Object $parsedBody -Name 'suppress_always_allow_rule')

        # 生テキストからの直接切り出し（ConvertFrom-Json/ConvertTo-Jsonの往復を経由しない）。
        # AllowAlwaysで送信する updatedPermissions は必ずこちらの値を使う（配列が
        # スカラーに畳まれる既知の問題を避けるため。詳細は Get-RawJsonValue のコメント参照）。
        $toolInputRaw = Get-RawJsonValue -Json $rawBody -Key 'tool_input'
        $permissionSuggestionsRaw = Get-RawJsonValue -Json $rawBody -Key 'permission_suggestions'
        $toolInputRawDisplay = $toolInputRaw
        if ($null -eq $toolInputRawDisplay) {
            $toolInputRawDisplay = '(absent)'
        }
        $permissionSuggestionsRawDisplay = $permissionSuggestionsRaw
        if ($null -eq $permissionSuggestionsRawDisplay) {
            $permissionSuggestionsRawDisplay = '(absent)'
        }

        $logLines = @()
        $logLines += ('=== request #{0} ===' -f $requestCount)
        $logLines += ('received at        : {0}' -f $receivedAtIso)
        $logLines += ('elapsed since start: {0}ms' -f $elapsedMs)
        $logLines += ('method / path      : {0} {1}' -f $request.HttpMethod, $request.Url.AbsolutePath)
        $logLines += 'headers            :'
        $logLines += $headerLines
        $logLines += 'body               :'
        $logLines += $prettyBody
        $logLines += '--- 抽出した重要フィールド ---'
        $logLines += ('hook_event_name           : {0}' -f $hookEventName)
        $logLines += ('tool_name                 : {0}' -f $toolName)
        $logLines += 'tool_input (raw)          :'
        $logLines += $toolInputRawDisplay
        $logLines += 'tool_input (formatted)    :'
        $logLines += $toolInputFormatted
        $logLines += 'permission_suggestions (raw)      :'
        $logLines += $permissionSuggestionsRawDisplay
        $logLines += 'permission_suggestions (formatted):'
        $logLines += $permissionSuggestionsFormatted
        $logLines += ('suppress_always_allow_rule: {0}' -f $suppressAlwaysAllowRule)

        Write-ProbeLog -Lines $logLines

        # --- 待機してからレスポンス ---
        $waitSeconds = $DelaySeconds
        if ($Mode -eq 'Observe') {
            $waitSeconds = 0
        }
        $delayStart = Get-Date
        if ($waitSeconds -gt 0) {
            Start-Sleep -Seconds $waitSeconds
        }

        $probeResponse = Get-ProbeResponse -Mode $Mode -PermissionSuggestionsRaw $permissionSuggestionsRaw
        $response = $context.Response
        try {
            $response.StatusCode = $probeResponse.StatusCode
            if ($null -ne $probeResponse.BodyText) {
                $response.ContentType = 'application/json'
                $bodyBytes = [System.Text.Encoding]::UTF8.GetBytes($probeResponse.BodyText)
                $response.ContentLength64 = $bodyBytes.Length
                $response.OutputStream.Write($bodyBytes, 0, $bodyBytes.Length)
            } else {
                $response.ContentLength64 = 0
            }
        } finally {
            $response.OutputStream.Close()
        }

        $respondedAt = Get-Date
        $actualDelayMs = [Math]::Round(((New-TimeSpan -Start $delayStart -End $respondedAt).TotalMilliseconds), 1)

        $responseLogLines = @()
        $responseLogLines += '--- レスポンス ---'
        $responseLogLines += ('mode               : {0}' -f $Mode)
        if ($Mode -eq 'AllowAlways') {
            if ($probeResponse.UpdatedPermissionsFound) {
                $responseLogLines += 'permission_suggestions found — embedded raw text into updatedPermissions'
            } else {
                $responseLogLines += 'permission_suggestions not found — sent allow without updatedPermissions'
            }
        }
        $responseLogLines += ('delayed            : {0}s' -f $DelaySeconds)
        $responseLogLines += ('responded at       : {0}' -f (Get-Iso8601Ms))
        $responseLogLines += ('actual delay       : {0}ms' -f $actualDelayMs)
        $responseLogLines += ('status             : {0}' -f $probeResponse.StatusCode)
        if ($null -ne $probeResponse.BodyText) {
            # 送信した本文は切り詰めずに全文を記録する（AllowAlwaysのupdatedPermissionsの
            # 中身が正しく埋め込まれているかをここで必ず確認できるようにするため）。
            $responseLogLines += 'body               :'
            $responseLogLines += $probeResponse.BodyText
        } else {
            $responseLogLines += 'body               : (empty)'
        }
        $responseLogLines += ''

        Write-ProbeLog -Lines $responseLogLines

        if ($MaxRequests -gt 0 -and $requestCount -ge $MaxRequests) {
            Write-Host ''
            Write-Host ("-MaxRequests {0} に到達したため自動終了します。" -f $MaxRequests)
            break
        }
    }
} finally {
    try {
        if ($listener.IsListening) {
            $listener.Stop()
        }
    } catch {
        # 終了処理中の例外は無視する。
    }
    $listener.Close()
    Write-Host ''
    Write-Host 'サーバを停止しました。'
}
