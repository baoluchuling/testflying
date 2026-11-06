#!/bin/sh
set -eu
umask 077

fail() { printf '%s\n' "$*" >&2; exit 1; }
root=$(CDPATH= cd -- "$(/usr/bin/dirname -- "$0")" && pwd -P)
[ "$(/usr/bin/uname -s)" = Darwin ] || fail 'This installer requires macOS.'

if [ "$(/usr/bin/id -u)" -ne 0 ]; then
    [ "$#" -eq 0 ] || fail 'Usage: sh ./install.sh'
    python=${SCREENSHOT_CONNECTOR_PYTHON:-$(command -v python3)}
    tools=${SCREENSHOT_CONNECTOR_LIFECYCLE_BIN:-"$HOME/Library/Application Support/CodexArtifactLifecycle/bin"}
    "$python" -B -c 'import sys; raise SystemExit(sys.version_info < (3, 11))' ||
        fail 'Install Python 3.11 or newer.'
    "$tools/artifact-audit" --self-check >/dev/null || fail 'Activate the artifact lifecycle integration first.'
    exec /usr/bin/sudo -- /bin/sh "$root/install.sh" --register "$python" "$tools" "$PATH"
fi

PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH
[ "$#" -eq 4 ] && [ "$1" = --register ] || fail 'Run sh ./install.sh from the screenshot account.'
user=${SUDO_USER:-}
case "$user" in ''|*[!a-zA-Z0-9_.-]*) fail 'Cannot identify the screenshot account.' ;; esac
uid=$(/usr/bin/id -u "$user")
[ "$uid" -ne 0 ] && [ "$uid" = "${SUDO_UID:-}" ] || fail 'Use a non-root screenshot account.'
home=$(/usr/bin/dscl . -read "/Users/$user" NFSHomeDirectory | /usr/bin/sed -n 's/^NFSHomeDirectory: //p')
case "$home" in /*) ;; *) fail 'Cannot determine the local account home directory.' ;; esac
case "$root" in "$home/Library/Application Support/"*) ;; *)
    fail 'Move the extracted package under ~/Library/Application Support before installing.' ;;
esac
for path in "$2" "$3"; do
    case "$path" in /*) ;; *) fail 'Python and lifecycle tool paths must be absolute.' ;; esac
done
[ "$(/usr/bin/stat -f '%u %Lp' "$root")" = "$uid 700" ] ||
    fail 'Set package directory permissions to 700.'
config="$root/connector.json"
[ -f "$config" ] && [ ! -L "$config" ] &&
    [ "$(/usr/bin/stat -f '%u %Lp' "$config")" = "$uid 600" ] ||
    fail 'Set connector.json permissions to 600.'
[ -f "$root/run.sh" ] && [ ! -L "$root/run.sh" ] &&
    [ -x "$root/screenshot-connector" ] && [ ! -L "$root/screenshot-connector" ] ||
    fail 'The Screenshot Connector package is incomplete.'
node=$(/usr/bin/plutil -extract connectorId raw -expect string -o - -- "$config")
printf '%s\n' "$node" | /usr/bin/grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' ||
    fail 'Invalid Screenshot Connector ID.'
[ "$(/usr/bin/plutil -extract kind raw -expect string -o - -- "$config")" = screenshot ] ||
    fail 'The configuration is for a different Connector kind.'

label="com.testflying.screenshot-connector.$node"
job="system/$label"
plist="/Library/LaunchDaemons/$label.plist"
stage=$(/usr/bin/mktemp "/Library/LaunchDaemons/.$label.XXXXXXXX")
trap '/bin/rm -f -- "$stage"' EXIT
trap 'exit 1' HUP INT TERM

/usr/bin/plutil -create xml1 "$stage"
plist_insert() { /usr/bin/plutil -insert "$@" -- "$stage"; }
plist_insert Label -string "$label"
plist_insert UserName -string "$user"
plist_insert WorkingDirectory -string "$root"
plist_insert ProgramArguments -array
plist_insert ProgramArguments -append -string /bin/sh
plist_insert ProgramArguments -append -string "$root/run.sh"
plist_insert ProgramArguments -append -string --service
plist_insert EnvironmentVariables -dictionary
plist_insert EnvironmentVariables.HOME -string "$home"
plist_insert EnvironmentVariables.PATH -string "$4"
plist_insert EnvironmentVariables.SCREENSHOT_CONNECTOR_PYTHON -string "$2"
plist_insert EnvironmentVariables.SCREENSHOT_CONNECTOR_LIFECYCLE_BIN -string "$3"
plist_insert RunAtLoad -bool YES
plist_insert KeepAlive -bool YES
plist_insert ThrottleInterval -integer 5
plist_insert ExitTimeOut -integer 60
/usr/bin/plutil -lint "$stage" >/dev/null
/bin/chmod 644 "$stage"

created=false
if [ -e "$plist" ] || [ -L "$plist" ]; then
    [ -f "$plist" ] && [ ! -L "$plist" ] &&
        [ "$(/usr/bin/stat -f '%u %Lp' "$plist")" = '0 644' ] &&
        /usr/bin/cmp -s "$stage" "$plist" ||
        fail 'A different installation uses this node ID. The existing job was preserved.'
else
    if /bin/launchctl print "$job" >/dev/null 2>&1; then
        fail 'A different loaded job uses this node ID.'
    fi
    /bin/ln "$stage" "$plist" || fail 'A concurrent installation created this job.'
    created=true
fi

if ! /bin/launchctl enable "$job" ||
    { ! /bin/launchctl print "$job" >/dev/null 2>&1 && ! /bin/launchctl bootstrap system "$plist"; }; then
    if "$created" && ! /bin/launchctl print "$job" >/dev/null 2>&1; then
        /bin/rm -f -- "$plist"
    fi
    fail 'launchd could not start the Screenshot Connector.'
fi
printf '%s\n' "Installed and started $job" "Status: sudo launchctl print $job"
