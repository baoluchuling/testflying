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

[ ! -L "$base" ] || fail 'Unexpected installation directory.'
if [ ! -e "$base" ]; then
    install -d -m 755 -o root -g wheel "$base"
fi
[ -d "$base" ] && [ "$(stat -f '%u %Lp' "$base")" = '0 755' ] ||
    fail 'Unexpected ownership or permissions on the installation directory.'
mkdir "$lock" || fail 'An installation is in progress or needs recovery.'

old_loaded=false
new_loaded=false
new_plist=false
new_program=false
new_data=false
new_credentials=false
cleanup() {
    result=$?
    trap - EXIT HUP INT TERM
    recovered=true
    if [ "$result" -ne 0 ]; then
        if $new_loaded && launchctl print "$job" >/dev/null 2>&1; then
            launchctl bootout "$job" || recovered=false
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
        if $recovered && $old_loaded; then
            launchctl bootstrap system "$plist" || recovered=false
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
[ ! -L "$data" ] && [ ! -L "$program" ] && [ ! -L "$plist" ] || fail 'Unexpected installation paths.'

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

for name in store-connector testflying-prod-ca.crt; do
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

if [ -e "$plist" ]; then
    regular "$plist" && [ "$(stat -f '%u' "$plist")" = 0 ] &&
        [ "$(plutil -extract ProgramArguments.0 raw -o - -- "$plist")" = "$program/store-connector" ] &&
        [ "$(plutil -extract UserName raw -o - -- "$plist")" = "$run_user" ] ||
        fail 'An existing launchd job does not match this installation.'
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

if launchctl print "$job" >/dev/null 2>&1; then
    [ -f "$plist" ] || fail 'A launchd job already uses the Connector label.'
    launchctl bootout "$job"
    old_loaded=true
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
if [ -f "$lock/connector.plist" ]; then
    install -m 644 -o root -g wheel "$lock/connector.plist" "$plist"
    new_plist=true
fi
launchctl enable "$job"
new_loaded=true
launchctl bootstrap system "$plist"
printf '%s\n' 'Connector installed. It starts now and at system boot.'
