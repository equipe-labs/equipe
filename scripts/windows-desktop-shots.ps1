# TP-8 on a Windows runner: the desktop app against the fake daemon (a named
# pipe), one picture per screen, light and dark, at 100% and 200%.
#
#   pwsh scripts/windows-desktop-shots.ps1 <out-dir>
#
# Build first: cargo build -p equipe-desktop --features snapshot --bins --example fake_daemon
param([Parameter(Mandatory = $true)][string]$Out)
$ErrorActionPreference = 'Stop'
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
$app = Join-Path $target 'debug\equipe-desktop.exe'
$daemon = Join-Path $target 'debug\examples\fake_daemon.exe'
New-Item -ItemType Directory -Force -Path $Out | Out-Null
$home_dir = Join-Path ([IO.Path]::GetTempPath()) ("equipe-shot-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $home_dir | Out-Null
'{"consent":"off"}' | Set-Content (Join-Path $home_dir 'telemetry.json')
$env:EQUIPE_HOME = $home_dir
$env:EQUIPE_SQUAD = 'demo'
$d = Start-Process $daemon -PassThru -RedirectStandardError (Join-Path $home_dir 'daemon.log')
try {
    Start-Sleep -Seconds 5
    foreach ($mode in 'light', 'dark') {
        foreach ($scale in 1, 2) {
            $env:EQUIPE_SNAPSHOT_SCALE = "$scale"
            $png = Join-Path $Out "$mode-${scale}00.png"
            $chat = Join-Path $home_dir "chat.png"; $board = Join-Path $home_dir "board.png"; $set = Join-Path $home_dir "settings.png"
            $env:EQUIPE_SCRIPT = "theme:$mode;size:1000x640;wait:2500;shot:$chat;board;wait:1500;shot:$board;settings;wait:1500;shot:$set;quit"
            $p = Start-Process $app -PassThru -RedirectStandardError (Join-Path $home_dir 'app.log')
            if (-not $p.WaitForExit(90000)) { $p.Kill(); throw "equipe-desktop still running after 90 s" }
            foreach ($f in $chat, $board, $set) { if (-not (Test-Path $f)) { Get-Content (Join-Path $home_dir 'app.log') -Tail 20; throw "no picture: $f" } }
            foreach ($name in 'chat', 'board', 'settings') {
                Move-Item (Join-Path $home_dir "$name.png") (Join-Path $Out "$name-$mode-${scale}00.png") -Force
            }
        }
    }
} finally {
    if (-not $d.HasExited) { $d.Kill() }
}
