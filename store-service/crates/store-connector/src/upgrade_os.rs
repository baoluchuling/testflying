//! Service controls restricted to the supported, privileged installations.
use std::path::{Path, PathBuf};

pub fn installed_root() -> Result<PathBuf, String> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return Ok(PathBuf::from(
        "/Library/Application Support/Testflying Connector",
    ));
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return windows::root();
    #[allow(unreachable_code)]
    Err("unsupported_installation".into())
}

pub fn ensure_manager_identity(root: &Path) -> Result<(), String> {
    if installed_root()? != root {
        return Err("unsupported_installation: unexpected installation root".into());
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        if std::env::current_exe().map_err(|_| "Cannot identify upgrade manager")?
            != root.join(if cfg!(windows) {
                "updater/store-connector.exe"
            } else {
                "updater/store-connector"
            })
        {
            return Err("Upgrade manager must run from the installed helper".into());
        }
    }
    #[cfg(target_os = "macos")]
    return macos::validate(root);
    #[cfg(target_os = "windows")]
    return windows::control("validate");
    #[allow(unreachable_code)]
    Err("unsupported_installation".into())
}

pub fn stop_worker(root: &Path) -> Result<(), String> {
    ensure_manager_identity(root)?;
    #[cfg(target_os = "macos")]
    return macos::stop();
    #[cfg(target_os = "windows")]
    return windows::control("stop");
    #[allow(unreachable_code)]
    Err("unsupported_installation".into())
}

pub fn start_worker(root: &Path) -> Result<(), String> {
    ensure_manager_identity(root)?;
    #[cfg(target_os = "macos")]
    return macos::start(root);
    #[cfg(target_os = "windows")]
    return windows::control("start");
    #[allow(unreachable_code)]
    Err("unsupported_installation".into())
}

/// Give the unchanged worker account access to a verified program after private staging.
pub fn prepare_program(root: &Path, path: &Path) -> Result<(), String> {
    ensure_manager_identity(root)?;
    let candidate = root.join("updater/private/candidate/program");
    if path != root.join("program") && path != candidate {
        return Err("Unexpected upgrade program path".into());
    }
    #[cfg(target_os = "macos")]
    return macos::prepare(path);
    #[cfg(target_os = "windows")]
    return windows::control(if path == candidate {
        "prepare-candidate"
    } else {
        "prepare-program"
    });
    #[allow(unreachable_code)]
    Err("unsupported_installation".into())
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::{
        fs,
        os::unix::fs::MetadataExt,
        process::Command,
        thread,
        time::{Duration, Instant},
    };
    const WORKER: &str = "system/com.testflying.connector";
    const PLIST: &str = "/Library/LaunchDaemons/com.testflying.connector.plist";
    const MANAGER: &str = "/Library/LaunchDaemons/com.testflying.connector.updater.plist";

    fn output(program: &str, args: &[&str]) -> Result<String, String> {
        let out = Command::new(program)
            .args(args)
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .output()
            .map_err(|_| "Cannot inspect installation".to_string())?;
        if !out.status.success() {
            return Err("Installation service validation failed".into());
        }
        String::from_utf8(out.stdout)
            .map(|s| s.trim().to_owned())
            .map_err(|_| "Invalid service metadata".into())
    }
    fn protected(path: &Path, directory: bool) -> Result<(), String> {
        for ancestor in path.ancestors() {
            let m = fs::symlink_metadata(ancestor)
                .map_err(|_| "Missing installation path".to_string())?;
            if m.file_type().is_symlink() || m.uid() != 0 || m.mode() & 0o022 != 0 {
                return Err("Unsafe installation ownership or permissions".into());
            }
            protected_acl(ancestor)?;
        }
        let m = fs::symlink_metadata(path).map_err(|_| "Missing installation path".to_string())?;
        if (directory && !m.is_dir()) || (!directory && (!m.is_file() || m.nlink() != 1)) {
            return Err("Unexpected installation file type".into());
        }
        Ok(())
    }
    fn protected_acl(path: &Path) -> Result<(), String> {
        let output = Command::new("/bin/ls")
            .arg("-lde")
            .arg(path)
            .env("LC_ALL", "C")
            .output()
            .map_err(|_| "Cannot inspect installation ACL")?;
        if !output.status.success() {
            return Err("Cannot inspect installation ACL".into());
        }
        let listing = String::from_utf8(output.stdout).map_err(|_| "Invalid installation ACL")?;
        let mut lines = listing.lines();
        let header = lines.next().ok_or("Missing installation ACL metadata")?;
        let permissions = header
            .split_whitespace()
            .next()
            .ok_or("Missing ACL permissions")?;
        let installation = Path::new("/Library/Application Support/Testflying Connector");
        if path.starts_with(installation) || path == Path::new(PLIST) || path == Path::new(MANAGER)
        {
            // Never let an extended allow ACE bypass the privileged tree's mode bits.
            if permissions.contains('+') || lines.next().is_some() {
                return Err("Extended ACL on privileged installation path".into());
            }
        } else {
            // macOS can ship shared system parents with deny-delete ACLs. Preserve
            // those; reject allow or unrecognized entries without modifying them.
            for line in lines {
                let (index, entry) = line
                    .trim()
                    .split_once(": ")
                    .ok_or("Unknown system ACL format")?;
                let (_, rights) = entry
                    .rsplit_once(" deny ")
                    .ok_or("Allow ACL on installation parent")?;
                if index.parse::<u32>().is_err()
                    || entry.contains(" allow ")
                    || rights.is_empty()
                    || !rights
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'_' || b == b',')
                {
                    return Err("Unsafe or unknown system parent ACL".into());
                }
            }
        }
        Ok(())
    }
    fn field(plist: &str, key: &str) -> Result<String, String> {
        output(
            "/usr/bin/plutil",
            &["-extract", key, "raw", "-o", "-", "--", plist],
        )
    }
    fn plist(
        plist: &str,
        binary: &Path,
        directory: &Path,
        user: &str,
        second: &str,
        label: &str,
    ) -> Result<(), String> {
        protected(Path::new(plist), false)?;
        let args = output(
            "/usr/bin/plutil",
            &[
                "-extract",
                "ProgramArguments",
                "json",
                "-o",
                "-",
                "--",
                plist,
            ],
        )?;
        let args: Vec<String> =
            serde_json::from_str(&args).map_err(|_| "Invalid launchd arguments".to_string())?;
        if args != [binary.to_string_lossy().as_ref(), second]
            || field(plist, "WorkingDirectory")? != directory.to_string_lossy()
            || field(plist, "UserName")? != user
            || field(plist, "Label")? != label
            || field(plist, "RunAtLoad")? != "true"
            || field(plist, "KeepAlive")? != "true"
        {
            return Err("Installation launchd job does not match".into());
        }
        if Command::new("/usr/bin/plutil")
            .args(["-extract", "Program", "raw", "-o", "-", "--", plist])
            .output()
            .map_err(|_| "Cannot inspect launchd job")?
            .status
            .success()
        {
            return Err("Unexpected launchd executable override".into());
        }
        Ok(())
    }
    pub fn validate(root: &Path) -> Result<(), String> {
        if output("/usr/bin/id", &["-u"])? != "0" {
            return Err("Upgrade manager requires root".into());
        }
        protected(root, true)?;
        // The worker program may be absent between the two atomic renames.
        // Recovery must remain available from the independent, stable helper.
        protected(&root.join("updater"), true)?;
        for file in [
            "updater/store-connector",
            "updater/trust.json",
            "updater/testflying-prod-ca.crt",
        ] {
            protected(&root.join(file), false)?;
        }
        let data = root.join("data");
        let meta = fs::symlink_metadata(&data).map_err(|_| "Missing installation data")?;
        if !meta.is_dir() || meta.file_type().is_symlink() || meta.uid() == 0 {
            return Err("Unsafe worker data directory".into());
        }
        let user = output(
            "/usr/bin/stat",
            &[
                "-f",
                "%Su",
                data.to_str().ok_or("Invalid installation path")?,
            ],
        )?;
        plist(
            PLIST,
            &root.join("program/store-connector"),
            &data,
            &user,
            "connector.json",
            "com.testflying.connector",
        )?;
        plist(
            MANAGER,
            &root.join("updater/store-connector"),
            &root.join("updater"),
            "root",
            "--upgrade-manager",
            "com.testflying.connector.updater",
        )
    }
    fn validate_program(path: &Path) -> Result<(), String> {
        protected(path, true)?;
        for file in ["store-connector", "testflying-prod-ca.crt"] {
            protected(&path.join(file), false)?;
        }
        for entry in fs::read_dir(path).map_err(|_| "Cannot inspect worker program")? {
            let entry = entry.map_err(|_| "Cannot inspect worker program file")?;
            let name = entry.file_name();
            let name = name.to_str().ok_or("Invalid program file name")?;
            if ![
                "store-connector",
                "start.sh",
                "testflying-prod-ca.crt",
                ".store-update-owner.json",
            ]
            .contains(&name)
            {
                return Err("Unexpected program file".into());
            }
            protected(&entry.path(), false)?;
        }
        Ok(())
    }
    pub fn prepare(path: &Path) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;
        validate_program(path)?;
        for (name, mode) in [
            ("store-connector", 0o755),
            ("start.sh", 0o755),
            ("testflying-prod-ca.crt", 0o644),
        ] {
            let file = path.join(name);
            if name == "start.sh" && !file.exists() {
                continue;
            }
            protected(&file, false)?;
            fs::set_permissions(&file, fs::Permissions::from_mode(mode))
                .map_err(|_| "Cannot set program permissions")?;
        }
        let marker = path.join(".store-update-owner.json");
        if marker.exists() {
            fs::set_permissions(marker, fs::Permissions::from_mode(0o600))
                .map_err(|_| "Cannot protect program ownership marker")?;
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .map_err(|_| "Cannot set program directory permissions".into())
    }
    fn loaded() -> Result<bool, String> {
        let result = Command::new("/bin/launchctl")
            .args(["print", WORKER])
            .output()
            .map_err(|_| "Cannot inspect worker")?;
        Ok(result.status.success())
    }
    pub fn stop() -> Result<(), String> {
        let pid = if loaded()? {
            let snapshot = output("/bin/launchctl", &["print", WORKER])?;
            let pid = snapshot.lines().find_map(|line| {
                let (key, value) = line.trim().split_once('=')?;
                (key.trim() == "pid")
                    .then(|| value.trim().parse::<u32>().ok())
                    .flatten()
            });
            output("/bin/launchctl", &["bootout", WORKER])?;
            pid
        } else {
            None
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let alive = if let Some(pid) = pid {
                Command::new("/bin/kill")
                    .args(["-0", &pid.to_string()])
                    .output()
                    .map_err(|_| "Cannot inspect worker process")?
                    .status
                    .success()
            } else {
                false
            };
            if !loaded()? && !alive {
                break;
            }
            if Instant::now() >= deadline {
                return Err("Worker did not stop within 30 seconds".into());
            }
            thread::sleep(Duration::from_millis(200));
        }
        Ok(())
    }
    pub fn start(root: &Path) -> Result<(), String> {
        validate_program(&root.join("program"))?;
        output("/bin/launchctl", &["enable", WORKER])?;
        if !loaded()? {
            output("/bin/launchctl", &["bootstrap", "system", PLIST])?;
        }
        output("/bin/launchctl", &["kickstart", WORKER])?;
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::*;
    use std::{
        ffi::OsString,
        os::windows::{ffi::OsStringExt, fs::MetadataExt},
        process::Command,
    };
    #[link(name = "kernel32")]
    extern "system" {
        fn GetSystemDirectoryW(buffer: *mut u16, size: u32) -> u32;
    }
    fn powershell() -> Result<PathBuf, String> {
        let mut buffer = [0u16; 32768];
        // The Windows API fills only this initialized, bounded UTF-16 buffer.
        let length =
            unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
        if length == 0 || length >= buffer.len() {
            return Err("Cannot resolve Windows system directory".into());
        }
        let path = PathBuf::from(OsString::from_wide(&buffer[..length]))
            .join("WindowsPowerShell/v1.0/powershell.exe");
        if !path.is_absolute() {
            return Err("Invalid Windows system directory".into());
        }
        for ancestor in path.ancestors() {
            let metadata = std::fs::symlink_metadata(ancestor)
                .map_err(|_| "Cannot inspect Windows system path")?;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("Reparse Windows system path".into());
            }
        }
        Ok(path)
    }
    fn run(script: &str) -> Result<String, String> {
        let out = Command::new(powershell()?)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .output()
            .map_err(|_| "Cannot run Windows installation service control".to_string())?;
        if !out.status.success() {
            return Err("Windows installation service validation or control failed".into());
        }
        String::from_utf8(out.stdout)
            .map(|s| s.trim().to_owned())
            .map_err(|_| "Invalid Windows service metadata".into())
    }
    pub fn root() -> Result<PathBuf, String> {
        run("$ErrorActionPreference='Stop'; [Console]::OutputEncoding=[Text.UTF8Encoding]::new($false); Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'Testflying Connector'").map(PathBuf::from)
    }
    pub fn control(operation: &str) -> Result<(), String> {
        // All script text is constant; remote commands and configuration cannot supply shell text.
        let action = match operation {
            "validate" => "",
            "stop" => {
                r#"Disable-ScheduledTask -TaskName 'Testflying Connector' -TaskPath '\' | Out-Null; Stop-ScheduledTask -TaskName 'Testflying Connector' -TaskPath '\'; $deadline=[DateTime]::UtcNow.AddSeconds(30); do { $t=Get-ScheduledTask -TaskName 'Testflying Connector' -TaskPath '\'; $processes=@(Get-CimInstance Win32_Process -Filter "Name = 'store-connector.exe'" | Where-Object { $_.ExecutablePath -ieq (Join-Path $program 'store-connector.exe') }); if($t.State -notin 'Running','Queued' -and $processes.Count -eq 0){break}; if([DateTime]::UtcNow -ge $deadline){throw 'Worker stop timed out'}; Start-Sleep -Milliseconds 200 } while($true)"#
            }
            "start" => {
                r"Assert-Program (Join-Path $root 'program'); Enable-ScheduledTask -TaskName 'Testflying Connector' -TaskPath '\' | Out-Null; Start-ScheduledTask -TaskName 'Testflying Connector' -TaskPath '\'"
            }
            "prepare-candidate" => {
                "Set-ProgramAccess (Join-Path $root 'updater\\private\\candidate\\program')"
            }
            "prepare-program" => "Set-ProgramAccess (Join-Path $root 'program')",
            _ => return Err("Unsupported installation operation".into()),
        };
        let script = format!(
            r#"
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$identity=[Security.Principal.WindowsIdentity]::GetCurrent()
$p=[Security.Principal.WindowsPrincipal]::new($identity)
if(-not $p.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) -and $identity.User.Value -ne 'S-1-5-18'){{throw 'Administrator identity required'}}
$root=Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'Testflying Connector'
function Assert-Protected($path,$directory) {{
 $item=Get-Item -LiteralPath $path -Force
 if([bool]$item.PSIsContainer -ne $directory){{throw 'Unexpected file type'}}
 $cursor=$item
 while($null -ne $cursor){{if($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint){{throw 'Reparse installation path'}}; $parent=Split-Path -Parent $cursor.FullName; if(-not $parent -or $parent -eq $cursor.FullName){{break}}; $cursor=Get-Item -LiteralPath $parent -Force}}
 $acl=Get-Acl -LiteralPath $path
 $owner=$acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
 if($owner -notin 'S-1-5-18','S-1-5-32-544'){{throw 'Unexpected installation owner'}}
 foreach($ace in $acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier])){{
  if($ace.AccessControlType -eq 'Allow' -and $ace.IdentityReference.Value -notin 'S-1-5-18','S-1-5-32-544' -and ([int]$ace.FileSystemRights -band 0x000d0156) -ne 0){{throw 'Writable privileged installation path'}}
 }}
}}
Assert-Protected $root $true
Assert-Protected (Join-Path $root 'updater') $true
Assert-Protected (Join-Path $root 'data') $true
foreach($file in 'updater\store-connector.exe','updater\trust.json','updater\testflying-prod-ca.crt'){{Assert-Protected (Join-Path $root $file) $false}}
function Assert-Task($name,$exe,$arguments,$work,$sid){{
 $t=Get-ScheduledTask -TaskName $name -TaskPath '\'
 $account=[Security.Principal.NTAccount]::new($t.Principal.UserId)
 try{{$actual=$account.Translate([Security.Principal.SecurityIdentifier]).Value}}catch{{$actual=$t.Principal.UserId}}
 if($actual -ne $sid -or $t.Principal.LogonType -ne 'ServiceAccount' -or $t.Actions.Count -ne 1 -or $t.Actions[0].Execute -ine $exe -or $t.Actions[0].Arguments -cne $arguments -or $t.Actions[0].WorkingDirectory -ine $work){{throw 'Task does not match installation'}}
}}
$cmd=Join-Path ([Environment]::GetFolderPath('System')) 'cmd.exe'
$program=Join-Path $root 'program'
$arguments='/d /s /c ""{{0}}" --background"' -f (Join-Path $program 'start.cmd')
Assert-Task 'Testflying Connector' $cmd $arguments (Join-Path $root 'data') 'S-1-5-19'
Assert-Task 'Testflying Connector Updater' (Join-Path $root 'updater\store-connector.exe') '--upgrade-manager' (Join-Path $root 'updater') 'S-1-5-18'
function Assert-Program($path){{
 Assert-Protected $path $true
 $required=@('store-connector.exe','start.cmd','testflying-prod-ca.crt')
 $allowed=$required+@('.store-update-owner.json')
 foreach($item in Get-ChildItem -LiteralPath $path -Force){{
  if($item.Name -cnotin $allowed){{throw 'Unexpected program file'}}
  Assert-Protected $item.FullName $false
 }}
 foreach($name in $required){{Assert-Protected (Join-Path $path $name) $false}}
}}
function Set-ProgramAccess($path){{
 Assert-Program $path
 foreach($entry in @($path)+(Get-ChildItem -LiteralPath $path -Force | ForEach-Object {{$_.FullName}})){{
  $directory=(Get-Item -LiteralPath $entry -Force).PSIsContainer
  if($directory){{$acl=[Security.AccessControl.DirectorySecurity]::new(); $inheritance='ContainerInherit,ObjectInherit'}}
  else{{$acl=[Security.AccessControl.FileSecurity]::new(); $inheritance='None'}}
  $acl.SetAccessRuleProtection($true,$false)
  $acl.SetOwner([Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'))
  foreach($sid in 'S-1-5-18','S-1-5-32-544','S-1-5-19'){{
   if($sid -eq 'S-1-5-19' -and (Split-Path -Leaf $entry) -eq '.store-update-owner.json'){{continue}}
   $rights=if($sid -eq 'S-1-5-19'){{'ReadAndExecute'}}else{{'FullControl'}}
   $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new([Security.Principal.SecurityIdentifier]::new($sid),$rights,$inheritance,'None','Allow'))
  }}
  Set-Acl -LiteralPath $entry -AclObject $acl
 }}
}}
{action}
"#
        );
        run(&script).map(|_| ())
    }
}
