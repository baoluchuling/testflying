#!/bin/sh
set -eu
PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH
umask 077

fail() { printf '%s\n' "$*" >&2; exit 1; }
source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
[ "$(uname -s)" = Darwin ] || fail 'This installer requires macOS.'
if [ "$(id -u)" -ne 0 ]; then
    exec sudo -- /bin/sh "$source_dir/install.sh"
fi

base='/Library/Application Support/Testflying Connector'
label=com.testflying.connector
job="system/$label"
plist="/Library/LaunchDaemons/$label.plist"
program="$base/program"
data="$base/data"
lock="$base/.install"
updater="$base/updater"
updater_label=com.testflying.connector.updater
updater_job="system/$updater_label"
updater_plist="/Library/LaunchDaemons/$updater_label.plist"

# Our installation has no extended ACL. System parents may retain deny-only ACLs;
# never rewrite ACLs on shared operating-system directories.
no_acl() {
    acl_listing=$(LC_ALL=C /bin/ls -lde "$1") || return 1
    acl_mode=$(printf '%s\n' "$acl_listing" | sed -n '1s/ .*//p')
    case "$acl_mode" in *+*) return 1 ;; esac
    [ "$(printf '%s\n' "$acl_listing" | wc -l | tr -d ' ')" = 1 ]
}
system_acl_safe() {
    acl_listing=$(LC_ALL=C /bin/ls -lde "$1") || return 1
    printf '%s\n' "$acl_listing" | awk '
        NR > 1 && ($0 ~ / allow / || $0 !~ /^[[:space:]]*[0-9]+: .* deny [a-z_,]+$/) { bad=1 }
        END { exit bad }
    '
}
for parent in / /Library '/Library/Application Support' /Library/LaunchDaemons; do
    [ -d "$parent" ] && [ ! -L "$parent" ] && [ "$(stat -f '%u' "$parent")" = 0 ] ||
        fail 'Unexpected installation parent directory.'
    parent_mode=$(stat -f '%Lp' "$parent")
    [ "$((0$parent_mode & 022))" -eq 0 ] || fail 'Writable installation parent directory.'
    system_acl_safe "$parent" || fail 'Unexpected allow ACL on an installation parent directory.'
done

[ ! -L "$base" ] || fail 'Unexpected installation directory.'
if [ ! -e "$base" ]; then
    install -d -m 755 -o root -g wheel "$base"
fi
[ -d "$base" ] && [ "$(stat -f '%u %Lp' "$base")" = '0 755' ] ||
    fail 'Unexpected ownership or permissions on the installation directory.'
no_acl "$base" || fail 'Unexpected extended ACL on the installation directory.'
mkdir "$lock" || fail 'An installation is in progress or needs recovery.'

old_loaded=false
new_loaded=false
new_plist=false
new_program=false
new_data=false
new_credentials=false
old_updater_loaded=false
new_updater_loaded=false
new_updater_plist=false
new_updater=false
updater_changed=false
worker_override_changed=false
updater_override_changed=false
old_worker_disabled=false
old_updater_disabled=false
stop_job() {
    stopped_job=$1
    snapshot=$(launchctl print "$stopped_job" 2>/dev/null) || return 0
    stopped_pid=$(printf '%s\n' "$snapshot" | sed -n 's/^[[:space:]]*pid = \([0-9][0-9]*\)$/\1/p')
    case "$stopped_pid" in ''|*[!0-9]*) stopped_pid='' ;; esac
    launchctl bootout "$stopped_job" || return 1
    attempts=0
    while launchctl print "$stopped_job" >/dev/null 2>&1 ||
        { [ -n "$stopped_pid" ] && /bin/kill -0 "$stopped_pid" 2>/dev/null; }; do
        attempts=$((attempts + 1))
        [ "$attempts" -lt 150 ] || return 1
        sleep 0.2
    done
}
cleanup() {
    result=$?
    trap - EXIT HUP INT TERM
    recovered=true
    if [ "$result" -ne 0 ]; then
        if $new_updater_loaded && launchctl print "$updater_job" >/dev/null 2>&1; then
            stop_job "$updater_job" || recovered=false
        fi
        if $new_loaded && launchctl print "$job" >/dev/null 2>&1; then
            stop_job "$job" || recovered=false
        fi
        if $recovered && $new_data; then
            mv "$data" "$lock/failed-data" || recovered=false
        elif $recovered; then
            for relative in connector.json credentials/apple-private-key.p8 credentials/google-service-account.json; do
                name=$(basename "$relative")
                if [ -f "$lock/applied-$name" ]; then
                    if [ -f "$lock/old-data/$relative" ]; then
                        mv "$lock/old-data/$relative" "$data/$relative" || recovered=false
                    else
                        rm -f -- "$data/$relative" || recovered=false
                    fi
                fi
            done
            if $new_credentials; then rmdir "$data/credentials" || recovered=false; fi
        fi
        if $recovered && $new_program; then
            mv "$program" "$lock/failed-program" || recovered=false
        fi
        if $recovered && [ -d "$lock/old-program" ]; then
            mv "$lock/old-program" "$program" || recovered=false
        fi
        if $recovered && $new_plist; then
            rm -f "$plist" || recovered=false
        fi
        if $recovered && $new_updater; then
            mv "$updater" "$lock/failed-updater" || recovered=false
        elif $recovered && $updater_changed; then
            for name in store-connector trust.json testflying-prod-ca.crt; do
                if [ -f "$lock/old-updater/$name" ]; then
                    mv "$lock/old-updater/$name" "$updater/$name" || recovered=false
                else
                    rm -f -- "$updater/$name" || recovered=false
                fi
            done
        fi
        if $recovered && $new_updater_plist; then
            rm -f "$updater_plist" || recovered=false
        fi
        if $recovered && $old_loaded; then
            worker_override_changed=true
            launchctl enable "$job" || recovered=false
            launchctl bootstrap system "$plist" || recovered=false
        fi
        if $recovered && $old_updater_loaded; then
            updater_override_changed=true
            launchctl enable "$updater_job" || recovered=false
            launchctl bootstrap system "$updater_plist" || recovered=false
        fi
        if $worker_override_changed; then
            if $old_worker_disabled; then launchctl disable "$job" || recovered=false
            else launchctl enable "$job" || recovered=false; fi
        fi
        if $updater_override_changed; then
            if $old_updater_disabled; then launchctl disable "$updater_job" || recovered=false
            else launchctl enable "$updater_job" || recovered=false; fi
        fi
    fi
    if $recovered; then
        rm -rf -- "$lock"
    else
        printf '%s\n' "Installer rollback needs attention: $lock" >&2
    fi
    exit "$result"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

# Disabled overrides are independent of whether either job is currently loaded.
disabled_snapshot=$(launchctl print-disabled system) || fail 'Cannot read launchd disabled overrides.'
if printf '%s\n' "$disabled_snapshot" | awk -v key="\"$label\"" '$1 == key && $2 == "=>" && $3 == "true" { found=1 } END { exit !found }'; then old_worker_disabled=true; fi
if printf '%s\n' "$disabled_snapshot" | awk -v key="\"$updater_label\"" '$1 == key && $2 == "=>" && $3 == "true" { found=1 } END { exit !found }'; then old_updater_disabled=true; fi

regular() { [ -f "$1" ] && [ ! -L "$1" ]; }
node_id() { regular "$1" && plutil -extract connectorId raw -o - -- "$1" 2>/dev/null; }
valid_id() {
    case "$1" in ''|*[!a-zA-Z0-9_-]*) return 1 ;; esac
    [ "${#1}" -le 128 ]
}

if [ -e "$source_dir/node-id.txt" ]; then
    regular "$source_dir/node-id.txt" || fail 'Invalid upgrade package.'
    expected=$(cat "$source_dir/node-id.txt")
else
    expected=$(node_id "$source_dir/connector.json") || fail 'The package is missing connector.json.'
fi
valid_id "$expected" || fail 'Invalid node ID.'
[ ! -L "$data" ] && [ ! -L "$program" ] && [ ! -L "$plist" ] &&
    [ ! -L "$updater" ] && [ ! -L "$updater_plist" ] || fail 'Unexpected installation paths.'
if [ -e "$program" ]; then
    no_acl "$program" || fail 'Unexpected extended ACL on the installed program directory.'
    for name in store-connector testflying-prod-ca.crt; do
        no_acl "$program/$name" || fail 'Unexpected extended ACL on an installed program file.'
    done
fi

has_data=false
if [ -e "$data" ]; then
    has_data=true
    current=$(node_id "$data/connector.json") || fail 'Cannot read the installed configuration.'
    [ "$current" = "$expected" ] || fail 'This package is for a different node. The installed configuration was preserved.'
    run_user=$(stat -f '%Su' "$data")
else
    current=$(node_id "$source_dir/connector.json") ||
        fail 'No local configuration. For a manual installation, unpack this upgrade over the old directory.'
    [ "$current" = "$expected" ] || fail 'The configuration is for a different node.'
    run_user=${SUDO_USER:-}
fi
case "$run_user" in ''|*[!a-zA-Z0-9_.-]*) fail 'Cannot identify the local installation user.' ;; esac
[ "$(id -u "$run_user")" -ne 0 ] || fail 'Run this installer from a normal macOS user account.'

for name in store-connector testflying-prod-ca.crt update-trust.json; do
    regular "$source_dir/$name" || fail "The package is missing $name."
done
install -d -m 755 "$lock/program"
install -m 755 "$source_dir/store-connector" "$lock/program/store-connector"
install -m 644 "$source_dir/testflying-prod-ca.crt" "$lock/program/testflying-prod-ca.crt"

update_configuration=false
if [ -e "$source_dir/connector.json" ] || [ -L "$source_dir/connector.json" ]; then
    regular "$source_dir/connector.json" || fail 'The package configuration is not a regular file.'
    printf '%s\n' 'Preparing selected platform settings; other installed settings will be kept.'
    "$lock/program/store-connector" --prepare-install "$source_dir" "$data" "$lock/prepared" ||
        fail 'Cannot prepare platform settings. The installed configuration was preserved.'
    update_configuration=true
    chown "$run_user" "$lock/prepared" "$lock/prepared/credentials"
    install -d -m 700 "$lock/old-data/credentials"
    for relative in connector.json credentials/apple-private-key.p8 credentials/google-service-account.json; do
        if [ -e "$lock/prepared/$relative" ]; then
            regular "$lock/prepared/$relative" || fail 'Invalid prepared configuration file.'
            chown "$run_user" "$lock/prepared/$relative"
            chmod 600 "$lock/prepared/$relative"
            if $has_data && { [ -e "$data/$relative" ] || [ -L "$data/$relative" ]; }; then
                if [ "$relative" != connector.json ]; then
                    [ -d "$data/credentials" ] && [ ! -L "$data/credentials" ] || fail 'Invalid installed credentials directory.'
                fi
                regular "$data/$relative" || fail 'Invalid installed configuration file.'
                cp -p "$data/$relative" "$lock/old-data/$relative"
            fi
        fi
    done
elif ! $has_data; then
    fail 'No full configuration is available for the initial installation.'
else
    printf '%s\n' 'Keeping the installed node configuration and shop credentials unchanged.'
fi

trust_config="$data/connector.json"
if $update_configuration; then trust_config="$lock/prepared/connector.json"; fi
"$lock/program/store-connector" --validate-update-trust "$trust_config" "$source_dir/update-trust.json" ||
    fail 'The upgrade trust does not match this node, center, or platform.'
if [ -e "$updater" ]; then
    [ -d "$updater" ] && [ "$(stat -f '%u %Lp' "$updater")" = '0 755' ] ||
        fail 'Unexpected updater ownership or permissions.'
    no_acl "$updater" || fail 'Unexpected extended ACL on the updater directory.'
    install -d -m 700 "$lock/old-updater"
    for name in store-connector trust.json testflying-prod-ca.crt; do
        if [ -e "$updater/$name" ] || [ -L "$updater/$name" ]; then
            regular "$updater/$name" && [ "$(stat -f '%u %l' "$updater/$name")" = '0 1' ] ||
                fail 'Unexpected updater file.'
            updater_mode=$(stat -f '%Lp' "$updater/$name")
            [ "$((0$updater_mode & 022))" -eq 0 ] || fail 'Writable updater file.'
            no_acl "$updater/$name" || fail 'Unexpected extended ACL on an updater file.'
            cp -p "$updater/$name" "$lock/old-updater/$name"
        fi
    done
fi
if [ -e "$updater_plist" ]; then
    no_acl "$updater_plist" || fail 'Unexpected extended ACL on the updater launchd plist.'
    regular "$updater_plist" && [ "$(stat -f '%u %Lp' "$updater_plist")" = '0 644' ] &&
        [ "$(plutil -extract Label raw -o - -- "$updater_plist")" = "$updater_label" ] &&
        [ "$(plutil -extract ProgramArguments.0 raw -o - -- "$updater_plist")" = "$updater/store-connector" ] &&
        [ "$(plutil -extract ProgramArguments.1 raw -o - -- "$updater_plist")" = '--upgrade-manager' ] &&
        [ "$(plutil -extract WorkingDirectory raw -o - -- "$updater_plist")" = "$updater" ] &&
        [ "$(plutil -extract UserName raw -o - -- "$updater_plist")" = root ] ||
        fail 'An existing updater launchd job does not match this installation.'
    if plutil -extract ProgramArguments.2 raw -o - -- "$updater_plist" >/dev/null 2>&1 ||
        plutil -extract Program raw -o - -- "$updater_plist" >/dev/null 2>&1; then
        fail 'Unexpected updater launchd executable or arguments.'
    fi
else
    cat >"$lock/updater.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>$updater_label</string>
  <key>UserName</key><string>root</string>
  <key>ProgramArguments</key><array><string>$updater/store-connector</string><string>--upgrade-manager</string></array>
  <key>WorkingDirectory</key><string>$updater</string>
  <key>EnvironmentVariables</key><dict><key>SSL_CERT_FILE</key><string>$updater/testflying-prod-ca.crt</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
</dict></plist>
PLIST
    plutil -lint "$lock/updater.plist" >/dev/null
fi

if [ -e "$plist" ]; then
    no_acl "$plist" || fail 'Unexpected extended ACL on the worker launchd plist.'
    regular "$plist" && [ "$(stat -f '%u %Lp' "$plist")" = '0 644' ] &&
        [ "$(plutil -extract Label raw -o - -- "$plist")" = "$label" ] &&
        [ "$(plutil -extract ProgramArguments.0 raw -o - -- "$plist")" = "$program/store-connector" ] &&
        [ "$(plutil -extract ProgramArguments.1 raw -o - -- "$plist")" = connector.json ] &&
        [ "$(plutil -extract WorkingDirectory raw -o - -- "$plist")" = "$data" ] &&
        [ "$(plutil -extract UserName raw -o - -- "$plist")" = "$run_user" ] ||
        fail 'An existing launchd job does not match this installation.'
    if plutil -extract ProgramArguments.2 raw -o - -- "$plist" >/dev/null 2>&1 ||
        plutil -extract Program raw -o - -- "$plist" >/dev/null 2>&1; then
        fail 'Unexpected worker launchd executable or arguments.'
    fi
else
    cat >"$lock/connector.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>$label</string>
  <key>UserName</key><string>$run_user</string>
  <key>ProgramArguments</key><array><string>$program/store-connector</string><string>connector.json</string></array>
  <key>WorkingDirectory</key><string>$data</string>
  <key>EnvironmentVariables</key><dict><key>SSL_CERT_FILE</key><string>$program/testflying-prod-ca.crt</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
</dict></plist>
PLIST
    plutil -lint "$lock/connector.plist" >/dev/null
fi

if launchctl print "$updater_job" >/dev/null 2>&1; then
    [ -f "$updater_plist" ] || fail 'A launchd job already uses the updater label.'
    old_updater_loaded=true
    stop_job "$updater_job" || fail 'The upgrade manager did not stop within 30 seconds.'
fi
if launchctl print "$job" >/dev/null 2>&1; then
    [ -f "$plist" ] || fail 'A launchd job already uses the Connector label.'
    old_loaded=true
    stop_job "$job" || fail 'The Connector did not stop within 30 seconds.'
fi
if $update_configuration; then
    if ! $has_data; then
        mv "$lock/prepared" "$data"
        new_data=true
    else
        if [ -e "$data/credentials" ] || [ -L "$data/credentials" ]; then
            [ -d "$data/credentials" ] && [ ! -L "$data/credentials" ] || fail 'Invalid installed credentials directory.'
        else
            install -d -m 700 -o "$run_user" "$data/credentials"
            new_credentials=true
        fi
        for relative in connector.json credentials/apple-private-key.p8 credentials/google-service-account.json; do
            if [ -f "$lock/prepared/$relative" ]; then
                name=$(basename "$relative")
                : > "$lock/applied-$name"
                mv "$lock/prepared/$relative" "$data/$relative"
            fi
        done
    fi
fi
if [ -e "$program" ]; then
    [ -d "$program" ] || fail 'The installed program path is not a directory.'
    mv "$program" "$lock/old-program"
fi
mv "$lock/program" "$program"
new_program=true
if [ ! -d "$updater" ]; then
    install -d -m 755 -o root -g wheel "$updater"
    new_updater=true
fi
updater_changed=true
install -m 755 -o root -g wheel "$source_dir/store-connector" "$updater/store-connector"
install -m 644 -o root -g wheel "$source_dir/update-trust.json" "$updater/trust.json"
install -m 644 -o root -g wheel "$source_dir/testflying-prod-ca.crt" "$updater/testflying-prod-ca.crt"
for protected_path in "$program" "$program/store-connector" "$program/testflying-prod-ca.crt" "$updater" "$updater/store-connector" "$updater/trust.json" "$updater/testflying-prod-ca.crt"; do
    no_acl "$protected_path" || fail 'Unexpected extended ACL on the installed program or updater.'
done
if [ -f "$lock/connector.plist" ]; then
    install -m 644 -o root -g wheel "$lock/connector.plist" "$plist"
    new_plist=true
fi
if [ -f "$lock/updater.plist" ]; then
    install -m 644 -o root -g wheel "$lock/updater.plist" "$updater_plist"
    new_updater_plist=true
fi
no_acl "$plist" && no_acl "$updater_plist" || fail 'Unexpected extended ACL on an installed launchd plist.'
worker_override_changed=true
launchctl enable "$job"
new_loaded=true
launchctl bootstrap system "$plist"
updater_override_changed=true
launchctl enable "$updater_job"
new_updater_loaded=true
launchctl bootstrap system "$updater_plist"
printf '%s\n' 'Connector and upgrade manager installed. They start now and at system boot.'
