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
$updater = Join-Path $root 'updater'
$updaterTaskName = 'Testflying Connector Updater'
$taskName = 'Testflying Connector'
$taskPath = '\'
$cmd = Join-Path ([Environment]::GetFolderPath('System')) 'cmd.exe'
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

function Assert-ProtectedPath([string] $path) {
    $item = Get-Item -LiteralPath $path -Force
    while ($null -ne $item) {
        if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Unexpected reparse installation path.' }
        $parent = Split-Path -Parent $item.FullName
        if (-not $parent -or $parent -eq $item.FullName) { break }
        $item = Get-Item -LiteralPath $parent -Force
    }
    $acl = Get-Acl -LiteralPath $path
    foreach ($ace in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
        if ($ace.AccessControlType -eq 'Allow' -and
            $ace.IdentityReference.Value -notin 'S-1-5-18', 'S-1-5-32-544' -and
            ([int] $ace.FileSystemRights -band 0x000d0156) -ne 0) {
            throw 'An installation path can be modified by an unexpected account.'
        }
    }
    $owner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
    if ($owner -notin 'S-1-5-18', 'S-1-5-32-544' -and $owner -ne $identity.User.Value) {
        throw 'Unexpected installation owner.'
    }
    $acl.SetOwner([Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'))
    Set-Acl -LiteralPath $path -AclObject $acl
}

function Assert-TaskAccount($task, [string] $sid) {
    try { $actual = ([Security.Principal.NTAccount]::new($task.Principal.UserId)).Translate([Security.Principal.SecurityIdentifier]).Value }
    catch { $actual = $task.Principal.UserId }
    if ($actual -ne $sid -or $task.Principal.LogonType -ne 'ServiceAccount') {
        throw 'An existing task has an unexpected service account.'
    }
}

function Stop-ConnectorTask([string] $name = $taskName) {
    Disable-ScheduledTask -TaskName $name -TaskPath $taskPath | Out-Null
    Stop-ScheduledTask -TaskName $name -TaskPath $taskPath
    $executable = if ($name -eq $updaterTaskName) { Join-Path $updater 'store-connector.exe' } else { Join-Path $program 'store-connector.exe' }
    for ($i = 0; $i -lt 150; $i++) {
        $task = Get-ScheduledTask -TaskName $name -TaskPath $taskPath
        $processes = @(Get-CimInstance Win32_Process -Filter "Name = 'store-connector.exe'" |
            Where-Object { $_.ExecutablePath -ieq $executable })
        if ($task.State -notin 'Running', 'Queued' -and $processes.Count -eq 0) { return }
        Start-Sleep -Milliseconds 200
    }
    throw 'The previous Connector did not stop within 30 seconds.'
}

function Remove-InstallerStage([string] $directory, [string] $ownerMarker) {
    $full = [IO.Path]::GetFullPath($directory)
    if ((Split-Path -Parent $full) -ine $root -or
        (Split-Path -Leaf $full) -cnotmatch '^\.install-[a-f0-9]{32}$') {
        throw 'Installer cleanup ownership cannot be established.'
    }
    Assert-RegularDirectory $full
    Assert-ProtectedPath $full
    $marker = Join-Path $full '.installer-owner'
    Assert-RegularFile $marker
    if ((Get-Content -LiteralPath $marker -Raw -Encoding UTF8) -cne $ownerMarker) {
        throw 'Installer cleanup owner marker does not match.'
    }
    # Enumerate one level at a time. Never recurse through a junction before
    # checking it, and finish checking the whole tree before deleting anything.
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push($full)
    $visited = 0
    while ($pending.Count -gt 0) {
        $current = $pending.Pop()
        Assert-RegularDirectory $current
        foreach ($item in Get-ChildItem -LiteralPath $current -Force) {
            $visited++
            if ($visited -gt 10000 -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw 'Installer cleanup found a reparse point or exceeded its entry limit.'
            }
            Assert-ProtectedPath $item.FullName
            if ($item.PSIsContainer) { $pending.Push($item.FullName) }
        }
    }
    Remove-Item -LiteralPath $full -Recurse -Force
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
        $security.SetOwner([Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'))
        foreach ($sid in 'S-1-5-18', 'S-1-5-32-544', 'S-1-5-19') {
            $rights = if ($sid -eq 'S-1-5-19') { 'ReadAndExecute' } else { 'FullControl' }
            $rule = [Security.AccessControl.FileSystemAccessRule]::new(
                [Security.Principal.SecurityIdentifier]::new($sid), $rights,
                'ContainerInherit,ObjectInherit', 'None', 'Allow')
            $security.AddAccessRule($rule)
        }
        [void] [IO.Directory]::CreateDirectory($root, $security)
    }
    Assert-ProtectedPath $root
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
    if (Test-Path -LiteralPath $program) { Assert-RegularDirectory $program; Assert-ProtectedPath $program }
    if (Test-Path -LiteralPath $updater) { Assert-RegularDirectory $updater; Assert-ProtectedPath $updater }

    $oldTask = Get-ScheduledTask -TaskName $taskName -TaskPath $taskPath -ErrorAction SilentlyContinue
    if ($oldTask -and ($oldTask.Actions.Count -ne 1 -or $oldTask.Actions[0].Execute -ne $cmd -or
        $oldTask.Actions[0].Arguments -ne $arguments -or $oldTask.Actions[0].WorkingDirectory -ne $data)) {
        throw 'An existing scheduled task does not match this installation.'
    }
    if ($oldTask) { Assert-TaskAccount $oldTask 'S-1-5-19' }
    $oldUpdaterTask = Get-ScheduledTask -TaskName $updaterTaskName -TaskPath $taskPath -ErrorAction SilentlyContinue
    if ($oldUpdaterTask) {
        Assert-TaskAccount $oldUpdaterTask 'S-1-5-18'
        if ($oldUpdaterTask.Actions.Count -ne 1 -or
            $oldUpdaterTask.Actions[0].Execute -ne (Join-Path $updater 'store-connector.exe') -or
            $oldUpdaterTask.Actions[0].Arguments -cne '--upgrade-manager' -or
            $oldUpdaterTask.Actions[0].WorkingDirectory -ne $updater) {
            throw 'An existing updater task does not match this installation.'
        }
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
    $stageOwner = [guid]::NewGuid().ToString('N')
    [IO.File]::WriteAllText((Join-Path $stage '.installer-owner'), $stageOwner, [Text.UTF8Encoding]::new($false))
    $newProgram = Join-Path $stage 'new-program'
    $oldProgram = Join-Path $stage 'old-program'
    $prepared = Join-Path $stage 'prepared-data'
    $oldData = Join-Path $stage 'old-data'
    $oldUpdater = Join-Path $stage 'old-updater'
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
    $createdUpdater = $false
    $changedUpdater = $false
    $registeredUpdaterTask = $false
    $updaterTaskChanged = $false
    $updaterFiles = @('store-connector.exe', 'trust.json', 'testflying-prod-ca.crt')
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
        $trustSource = Join-Path $PSScriptRoot 'update-trust.json'
        Assert-RegularFile $trustSource
        $trustConfig = if ($updateConfiguration) { Join-Path $prepared 'connector.json' } else { Join-Path $data 'connector.json' }
        $trustArguments = '--validate-update-trust "{0}" "{1}"' -f $trustConfig, $trustSource
        $validation = Start-Process -FilePath (Join-Path $newProgram 'store-connector.exe') `
            -ArgumentList $trustArguments -NoNewWindow -Wait -PassThru
        if ($validation.ExitCode -ne 0) { throw 'The upgrade trust does not match this node, center, or platform.' }
        if (Test-Path -LiteralPath $updater) {
            [void] [IO.Directory]::CreateDirectory($oldUpdater)
            foreach ($name in $updaterFiles) {
                $file = Join-Path $updater $name
                if (Test-Path -LiteralPath $file) {
                    Assert-RegularFile $file
                    Assert-ProtectedPath $file
                    Copy-Item -LiteralPath $file -Destination $oldUpdater
                }
            }
        }

        Write-Host '[4/5] Updating the background task and program...'
        if ($oldUpdaterTask) {
            $updaterTaskChanged = $true
            Stop-ConnectorTask $updaterTaskName
        }
        if ($oldTask) {
            $taskChanged = $true
            Stop-ConnectorTask
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
        Assert-ProtectedPath $data
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
        Assert-ProtectedPath $program
        foreach ($name in 'store-connector.exe', 'start.cmd', 'testflying-prod-ca.crt') { Assert-ProtectedPath (Join-Path $program $name) }
        if (-not (Test-Path -LiteralPath $updater)) {
            [void] [IO.Directory]::CreateDirectory($updater)
            $createdUpdater = $true
        }
        $changedUpdater = $true
        Copy-Item -LiteralPath (Join-Path $program 'store-connector.exe') -Destination (Join-Path $updater 'store-connector.exe') -Force
        Copy-Item -LiteralPath $trustSource -Destination (Join-Path $updater 'trust.json') -Force
        Copy-Item -LiteralPath (Join-Path $program 'testflying-prod-ca.crt') -Destination (Join-Path $updater 'testflying-prod-ca.crt') -Force
        Assert-ProtectedPath $updater
        foreach ($name in $updaterFiles) { Assert-ProtectedPath (Join-Path $updater $name) }
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
        if ($oldUpdaterTask) {
            Enable-ScheduledTask -TaskName $updaterTaskName -TaskPath $taskPath | Out-Null
        } else {
            $action = New-ScheduledTaskAction -Execute (Join-Path $updater 'store-connector.exe') `
                -Argument '--upgrade-manager' -WorkingDirectory $updater
            $trigger = New-ScheduledTaskTrigger -AtStartup
            $account = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
            $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Seconds 0) -StartWhenAvailable `
                -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew `
                -RestartCount 999 -RestartInterval (New-TimeSpan -Minutes 1)
            Register-ScheduledTask -TaskName $updaterTaskName -TaskPath $taskPath -Action $action -Trigger $trigger `
                -Principal $account -Settings $settings | Out-Null
            $registeredUpdaterTask = $true
        }
        Write-Host '[5/5] Requesting the background task to start...'
        Start-ScheduledTask -TaskName $taskName -TaskPath $taskPath
        Start-ScheduledTask -TaskName $updaterTaskName -TaskPath $taskPath
        Write-Host 'Connector and upgrade manager installed. Both background startups and system boot starts are configured.'
        Write-Host 'This does not confirm a connection. Check the node connection status in the center.'
        $finished = $true
    } catch {
        $failure = $_
        try {
            if ($registeredUpdaterTask -or $updaterTaskChanged) { Stop-ConnectorTask $updaterTaskName }
            if ($registeredUpdaterTask) { Unregister-ScheduledTask -TaskName $updaterTaskName -TaskPath $taskPath -Confirm:$false }
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
            if ($createdUpdater) {
                Move-Item -LiteralPath $updater -Destination (Join-Path $stage 'failed-updater')
            } elseif ($changedUpdater) {
                foreach ($name in $updaterFiles) {
                    $file = Join-Path $updater $name
                    $backup = Join-Path $oldUpdater $name
                    if (Test-Path -LiteralPath $backup) {
                        Move-Item -LiteralPath $backup -Destination $file -Force
                        Assert-ProtectedPath $file
                    }
                    elseif (Test-Path -LiteralPath $file) { Assert-RegularFile $file; Remove-Item -LiteralPath $file -Force }
                }
            }
            if ($taskChanged -and $oldTask.State -eq 'Disabled') {
                Disable-ScheduledTask -TaskName $taskName -TaskPath $taskPath | Out-Null
            } elseif ($taskChanged) {
                Enable-ScheduledTask -TaskName $taskName -TaskPath $taskPath | Out-Null
                if ($oldTask.State -in 'Running', 'Queued') { Start-ScheduledTask -TaskName $taskName -TaskPath $taskPath }
            }
            if ($updaterTaskChanged) {
                if ($oldUpdaterTask.State -eq 'Disabled') { Disable-ScheduledTask -TaskName $updaterTaskName -TaskPath $taskPath | Out-Null }
                else {
                    Enable-ScheduledTask -TaskName $updaterTaskName -TaskPath $taskPath | Out-Null
                    if ($oldUpdaterTask.State -in 'Running', 'Queued') { Start-ScheduledTask -TaskName $updaterTaskName -TaskPath $taskPath }
                }
            }
            $rollbackFinished = $true
        } catch {
            $keepStage = $true
            Write-Warning "Installer rollback needs attention: $stage"
        }
        throw $failure
    } finally {
        if (-not $finished -and -not $rollbackFinished -and
            ($changedProgram -or $taskChanged -or $updaterTaskChanged -or $changedUpdater -or $createdData -or $appliedFiles.Count -gt 0)) {
            $keepStage = $true
            Write-Warning "Installation interrupted; recovery files were preserved: $stage"
        }
        if (-not $keepStage) {
            try { Remove-InstallerStage $stage $stageOwner }
            catch {
                $keepStage = $true
                Write-Warning "Installer cleanup needs attention; files were preserved: $stage"
                Write-Warning 'The upgrade manager remains paused by this installer recovery directory until it is reviewed.'
            }
        }
    }
} finally {
    if ($locked) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
