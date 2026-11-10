$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host 'Requesting administrator permission...'
    $powershell = Join-Path $PSHOME 'powershell.exe'
    $arguments = '-NoLogo -NoProfile -ExecutionPolicy Bypass -File "{0}"' -f $PSCommandPath
    $elevated = Start-Process -FilePath $powershell -ArgumentList $arguments -Verb RunAs -Wait -PassThru
    exit $elevated.ExitCode
}
Write-Host '[1/5] Checking installation paths and node configuration...'

$root = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'Testflying Connector'
$data = Join-Path $root 'data'
$program = Join-Path $root 'program'
$taskName = 'Testflying Connector'
$taskPath = '\'
$cmd = Join-Path $env:SystemRoot 'System32\cmd.exe'
$arguments = '/d /s /c ""{0}" --background"' -f (Join-Path $program 'start.cmd')

function Assert-RegularFile([string] $file) {
    $item = Get-Item -LiteralPath $file -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'The package contains an unexpected file type.'
    }
}

function Read-NodeId([string] $file) {
    Assert-RegularFile $file
    $configuration = Get-Content -LiteralPath $file -Raw | ConvertFrom-Json
    return [string] $configuration.connectorId
}

function Assert-RegularDirectory([string] $directory) {
    $item = Get-Item -LiteralPath $directory -Force
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'An installation path is not a regular directory.'
    }
}

function Stop-ConnectorTask {
    $task = Get-ScheduledTask -TaskName $taskName -TaskPath $taskPath
    if ($task.State -in 'Running', 'Queued') {
        Stop-ScheduledTask -TaskName $taskName -TaskPath $taskPath
        for ($i = 0; $i -lt 30; $i++) {
            $task = Get-ScheduledTask -TaskName $taskName -TaskPath $taskPath
            if ($task.State -notin 'Running', 'Queued') { return }
            Start-Sleep -Milliseconds 200
        }
        throw 'The previous Connector did not stop.'
    }
}

$mutex = [Threading.Mutex]::new($false, 'Global\Testflying.Connector.Install')
try {
    try { $locked = $mutex.WaitOne(0) }
    catch [Threading.AbandonedMutexException] { $locked = $true }
    if (-not $locked) { throw 'An installation is already running.' }

    if (Test-Path -LiteralPath $root) {
        Assert-RegularDirectory $root
        if (-not (Get-Acl -LiteralPath $root).AreAccessRulesProtected) {
            throw 'The existing installation directory does not have a private access list.'
        }
    } else {
        $security = [Security.AccessControl.DirectorySecurity]::new()
        $security.SetAccessRuleProtection($true, $false)
        foreach ($sid in 'S-1-5-18', 'S-1-5-32-544', 'S-1-5-19') {
            $rights = if ($sid -eq 'S-1-5-19') { 'ReadAndExecute' } else { 'FullControl' }
            $rule = [Security.AccessControl.FileSystemAccessRule]::new(
                [Security.Principal.SecurityIdentifier]::new($sid), $rights,
                'ContainerInherit,ObjectInherit', 'None', 'Allow')
            $security.AddAccessRule($rule)
        }
        [void] [IO.Directory]::CreateDirectory($root, $security)
    }

    $config = Join-Path $PSScriptRoot 'connector.json'
    $idFile = Join-Path $PSScriptRoot 'node-id.txt'
    $expected = if (Test-Path -LiteralPath $idFile) {
        Assert-RegularFile $idFile
        (Get-Content -LiteralPath $idFile -Raw).Trim()
    } else { Read-NodeId $config }
    if ($expected -notmatch '^[a-zA-Z0-9_-]{1,128}$') { throw 'Invalid node ID.' }

    $hasData = Test-Path -LiteralPath $data
    if ($hasData) {
        Assert-RegularDirectory $data
        $existing = Read-NodeId (Join-Path $data 'connector.json')
        if ($existing -cne $expected) {
            throw 'This package is for a different node. The installed configuration was preserved.'
        }
    } else {
        if (-not (Test-Path -LiteralPath $config)) {
            throw 'No local configuration. For a manual installation, unpack this upgrade over the old directory.'
        }
        if ((Read-NodeId $config) -cne $expected) { throw 'The configuration is for a different node.' }
    }
    if (Test-Path -LiteralPath $program) { Assert-RegularDirectory $program }

    $oldTask = Get-ScheduledTask -TaskName $taskName -TaskPath $taskPath -ErrorAction SilentlyContinue
    if ($oldTask -and ($oldTask.Actions.Count -ne 1 -or $oldTask.Actions[0].Execute -ne $cmd -or
        $oldTask.Actions[0].Arguments -ne $arguments -or $oldTask.Actions[0].WorkingDirectory -ne $data)) {
        throw 'An existing scheduled task does not match this installation.'
    }

    if ($hasData) {
        Write-Host '[2/5] Keeping the installed node configuration and shop credentials.'
    } else {
        Write-Host '[2/5] Preparing the node configuration and shop credentials.'
    }

    $stage = Join-Path $root ('.install-' + [guid]::NewGuid().ToString('N'))
    [void] [IO.Directory]::CreateDirectory($stage)
    $newProgram = Join-Path $stage 'new-program'
    $oldProgram = Join-Path $stage 'old-program'
    $changedProgram = $false
    $registeredTask = $false
    $taskChanged = $false
    $keepStage = $false
    try {
        Write-Host '[3/5] Preparing the updated program files...'
        [void] [IO.Directory]::CreateDirectory($newProgram)
        foreach ($name in 'store-connector.exe', 'start.cmd', 'testflying-prod-ca.crt') {
            $source = Join-Path $PSScriptRoot $name
            Assert-RegularFile $source
            Copy-Item -LiteralPath $source -Destination $newProgram
        }
        if (-not $hasData) {
            $credentials = Join-Path $PSScriptRoot 'credentials'
            Assert-RegularDirectory $credentials
            $newData = Join-Path $stage 'data'
            $newCredentials = Join-Path $newData 'credentials'
            [void] [IO.Directory]::CreateDirectory($newCredentials)
            Copy-Item -LiteralPath $config -Destination $newData
            $count = 0
            foreach ($name in 'apple-private-key.p8', 'google-service-account.json') {
                $source = Join-Path $credentials $name
                if (Test-Path -LiteralPath $source) {
                    Assert-RegularFile $source
                    Copy-Item -LiteralPath $source -Destination $newCredentials
                    $count++
                }
            }
            if ($count -eq 0) { throw 'The package has no shop credentials.' }
            Move-Item -LiteralPath $newData -Destination $data
        }

        Write-Host '[4/5] Updating the background task and program...'
        if ($oldTask) {
            Stop-ConnectorTask
            $taskChanged = $true
        }
        if (Test-Path -LiteralPath $program) { Move-Item -LiteralPath $program -Destination $oldProgram }
        Move-Item -LiteralPath $newProgram -Destination $program
        $changedProgram = $true
        if ($oldTask) {
            Enable-ScheduledTask -TaskName $taskName -TaskPath $taskPath | Out-Null
        } else {
            $action = New-ScheduledTaskAction -Execute $cmd -Argument $arguments -WorkingDirectory $data
            $trigger = New-ScheduledTaskTrigger -AtStartup
            $account = New-ScheduledTaskPrincipal -UserId 'LOCALSERVICE' -LogonType ServiceAccount
            $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Seconds 0) -StartWhenAvailable `
                -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew `
                -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
            Register-ScheduledTask -TaskName $taskName -TaskPath $taskPath -Action $action -Trigger $trigger `
                -Principal $account -Settings $settings | Out-Null
            $registeredTask = $true
        }
        Write-Host '[5/5] Requesting the background task to start...'
        Start-ScheduledTask -TaskName $taskName -TaskPath $taskPath
        Write-Host 'Installation complete. Background startup requested; startup at system boot is configured.'
        Write-Host 'This does not confirm a connection. Check the node connection status in the center.'
    } catch {
        $failure = $_
        try {
            if ($registeredTask -or ($taskChanged -and $changedProgram)) { Stop-ConnectorTask }
            if ($registeredTask) { Unregister-ScheduledTask -TaskName $taskName -TaskPath $taskPath -Confirm:$false }
            if ($changedProgram) { Move-Item -LiteralPath $program -Destination (Join-Path $stage 'failed-program') }
            if (Test-Path -LiteralPath $oldProgram) { Move-Item -LiteralPath $oldProgram -Destination $program }
            if ($taskChanged -and $oldTask.State -eq 'Disabled') {
                Disable-ScheduledTask -TaskName $taskName -TaskPath $taskPath | Out-Null
            } elseif ($taskChanged -and $oldTask.State -in 'Running', 'Queued') {
                Start-ScheduledTask -TaskName $taskName -TaskPath $taskPath
            }
        } catch {
            $keepStage = $true
            Write-Warning "Installer rollback needs attention: $stage"
        }
        throw $failure
    } finally {
        if (-not $keepStage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    }
} finally {
    if ($locked) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
