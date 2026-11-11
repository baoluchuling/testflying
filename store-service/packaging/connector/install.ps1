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
    $configuration = Get-Content -LiteralPath $file -Raw -Encoding UTF8 | ConvertFrom-Json
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
    if (Get-ChildItem -LiteralPath $root -Directory -Force -Filter '.install-*') {
        throw 'An earlier installation needs recovery. Its backup directory was preserved.'
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

    $updateConfiguration = Test-Path -LiteralPath $config
    if ($updateConfiguration) {
        Assert-RegularFile $config
        Write-Host '[2/5] Preparing selected platform settings; other installed settings will be kept.'
    } else {
        Write-Host '[2/5] Keeping the installed node configuration and shop credentials unchanged.'
    }

    $stage = Join-Path $root ('.install-' + [guid]::NewGuid().ToString('N'))
    [void] [IO.Directory]::CreateDirectory($stage)
    $newProgram = Join-Path $stage 'new-program'
    $oldProgram = Join-Path $stage 'old-program'
    $prepared = Join-Path $stage 'prepared-data'
    $oldData = Join-Path $stage 'old-data'
    $appliedFiles = [Collections.Generic.List[string]]::new()
    $updates = @()
    $createdData = $false
    $createdCredentials = $false
    $changedProgram = $false
    $registeredTask = $false
    $taskChanged = $false
    $keepStage = $false
    $finished = $false
    $rollbackFinished = $false
    try {
        Write-Host '[3/5] Preparing the updated program files...'
        [void] [IO.Directory]::CreateDirectory($newProgram)
        foreach ($name in 'store-connector.exe', 'start.cmd', 'testflying-prod-ca.crt') {
            $source = Join-Path $PSScriptRoot $name
            Assert-RegularFile $source
            Copy-Item -LiteralPath $source -Destination $newProgram
        }
        if ($updateConfiguration) {
            $credentials = Join-Path $PSScriptRoot 'credentials'
            Assert-RegularDirectory $credentials
            $prepareArguments = '--prepare-install "{0}" "{1}" "{2}"' -f $PSScriptRoot, $data, $prepared
            $preparation = Start-Process -FilePath (Join-Path $newProgram 'store-connector.exe') `
                -ArgumentList $prepareArguments -NoNewWindow -Wait -PassThru
            if ($preparation.ExitCode -ne 0) {
                throw 'Cannot prepare platform settings. The installed configuration was preserved.'
            }
            $updates = @('connector.json')
            foreach ($relative in 'credentials\apple-private-key.p8', 'credentials\google-service-account.json') {
                if (Test-Path -LiteralPath (Join-Path $prepared $relative)) { $updates += $relative }
            }
            foreach ($relative in $updates) {
                Assert-RegularFile (Join-Path $prepared $relative)
                $target = Join-Path $data $relative
                if ($hasData -and (Test-Path -LiteralPath $target)) {
                    if ($relative -ne 'connector.json') { Assert-RegularDirectory (Join-Path $data 'credentials') }
                    Assert-RegularFile $target
                    $backup = Join-Path $oldData $relative
                    [void] [IO.Directory]::CreateDirectory((Split-Path -Parent $backup))
                    Copy-Item -LiteralPath $target -Destination $backup
                }
            }
        }
        if (-not $hasData -and -not $updateConfiguration) {
            throw 'No full configuration is available for the initial installation.'
        }

        Write-Host '[4/5] Updating the background task and program...'
        if ($oldTask) {
            Stop-ConnectorTask
            $taskChanged = $true
        }
        if ($updateConfiguration) {
            if (-not $hasData) {
                Move-Item -LiteralPath $prepared -Destination $data
                $createdData = $true
            } else {
                $credentials = Join-Path $data 'credentials'
                if (Test-Path -LiteralPath $credentials) { Assert-RegularDirectory $credentials }
                else {
                    [void] [IO.Directory]::CreateDirectory($credentials)
                    $createdCredentials = $true
                }
                # The task is stopped; replace only selected files and preserve everything else.
                foreach ($relative in $updates) {
                    $appliedFiles.Add($relative)
                    Move-Item -LiteralPath (Join-Path $prepared $relative) -Destination (Join-Path $data $relative) -Force
                }
            }
        }
        $logs = Join-Path $data 'logs'
        if (Test-Path -LiteralPath $logs) { Assert-RegularDirectory $logs }
        else { [void] [IO.Directory]::CreateDirectory($logs) }
        $logSecurity = Get-Acl -LiteralPath $logs
        $logRule = [Security.AccessControl.FileSystemAccessRule]::new(
            [Security.Principal.SecurityIdentifier]::new('S-1-5-19'), 'Modify',
            'ContainerInherit,ObjectInherit', 'None', 'Allow')
        $logSecurity.SetAccessRule($logRule)
        Set-Acl -LiteralPath $logs -AclObject $logSecurity
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
        $finished = $true
    } catch {
        $failure = $_
        try {
            if ($registeredTask -or ($taskChanged -and $changedProgram)) { Stop-ConnectorTask }
            if ($registeredTask) { Unregister-ScheduledTask -TaskName $taskName -TaskPath $taskPath -Confirm:$false }
            if ($createdData) {
                Assert-RegularDirectory $data
                Move-Item -LiteralPath $data -Destination (Join-Path $stage 'failed-data')
            } else {
                foreach ($relative in $appliedFiles) {
                    $target = Join-Path $data $relative
                    $backup = Join-Path $oldData $relative
                    if (Test-Path -LiteralPath $backup) {
                        Move-Item -LiteralPath $backup -Destination $target -Force
                    } elseif (Test-Path -LiteralPath $target) {
                        Assert-RegularFile $target
                        Remove-Item -LiteralPath $target -Force
                    }
                }
                if ($createdCredentials) { Remove-Item -LiteralPath (Join-Path $data 'credentials') }
            }
            if ($changedProgram) { Move-Item -LiteralPath $program -Destination (Join-Path $stage 'failed-program') }
            if (Test-Path -LiteralPath $oldProgram) { Move-Item -LiteralPath $oldProgram -Destination $program }
            if ($taskChanged -and $oldTask.State -eq 'Disabled') {
                Disable-ScheduledTask -TaskName $taskName -TaskPath $taskPath | Out-Null
            } elseif ($taskChanged -and $oldTask.State -in 'Running', 'Queued') {
                Start-ScheduledTask -TaskName $taskName -TaskPath $taskPath
            }
            $rollbackFinished = $true
        } catch {
            $keepStage = $true
            Write-Warning "Installer rollback needs attention: $stage"
        }
        throw $failure
    } finally {
        if (-not $finished -and -not $rollbackFinished -and
            ($changedProgram -or $taskChanged -or $createdData -or $appliedFiles.Count -gt 0)) {
            $keepStage = $true
            Write-Warning "Installation interrupted; recovery files were preserved: $stage"
        }
        if (-not $keepStage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    }
} finally {
    if ($locked) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
