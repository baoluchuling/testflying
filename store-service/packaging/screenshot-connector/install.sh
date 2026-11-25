#!/bin/sh
set -eu
umask 077

fail() { printf '%s\n' "$*" >&2; exit 1; }
root=$(CDPATH= cd -- "$(/usr/bin/dirname -- "$0")" && pwd -P)
[ "$(/usr/bin/uname -s)" = Darwin ] || fail 'This installer requires macOS.'
[ "$(/usr/sbin/sysctl -n hw.optional.arm64 2>/dev/null || true)" = 1 ] ||
    fail 'This installer requires an Apple Silicon Mac.'

validate_package() {
account_home=$(/usr/bin/dscl . -read "/Users/$user" NFSHomeDirectory | /usr/bin/sed -n 's/^NFSHomeDirectory: //p')
case "$account_home" in /*) ;; *) fail 'Cannot determine the local account home directory.' ;; esac
case "$root" in "$account_home/Library/Application Support/"*) ;; *)
    fail 'Move the extracted package under ~/Library/Application Support before installing.' ;;
esac
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
}

permissions_guide() {
    printf '%s\n' \
        '截图节点需要访问本机项目文件。请在 macOS 系统设置中为以下程序开启文件访问权限：' \
        "$root/screenshot-connector" \
        '路径：系统设置 → 隐私与安全性 → 完全磁盘访问。点“+”，按 Command+Shift+G 粘贴上述完整路径，添加并开启。' \
        '若系统提供按文件夹授权，可选择项目所需目录；需要访问多个目录时可手动开启完全磁盘访问。' \
        '升级后若旧开关已开启但仍提示无权限，请移除旧条目，重新添加当前文件并开启。' \
        '此操作由你在系统界面批准；安装器不会读取隐私目录来检测或代替你授权。'
    if [ -n "${SSH_CONNECTION:-}${SSH_TTY:-}${SSH_CLIENT:-}" ] || \
        [ ! -t 0 ] || ! /bin/launchctl print "gui/$uid" >/dev/null 2>&1; then
        printf '%s\n' \
            '当前不是本机交互式桌面终端，未打开系统设置，也未确认授权。' \
            '请在截图账号的本机终端进入原安装目录，运行：sh ./install.sh --permissions' \
            '已手动完成授权时，也可运行：sh ./install.sh --restart'
        return 1
    fi
    /usr/bin/open -R "$root/screenshot-connector" || \
        printf '%s\n' '无法打开 Finder，请使用上方完整路径手动添加程序。'
    /usr/bin/open 'x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles' || \
        printf '%s\n' '无法直接打开授权页面，请按上方路径手动进入系统设置。'
    printf '%s\n' \
        '请先在系统设置中完成授权，再回到这里按回车。Ctrl+C 可退出引导。' \
        '完成后将安装或优雅重启该节点；后台服务会由 launchd 自动拉起，无需重启电脑。'
    if ! IFS= read -r acknowledged; then
        printf '%s\n' '未收到完成确认，未安装或重启服务。'
        return 2
    fi
    printf '%s\n' '已收到你的完成确认，继续安装或重启；授权结果仍以 macOS 系统设置为准。'
}

if [ "$(/usr/bin/id -u)" -ne 0 ]; then
    case "$#:${1-}" in
        0:) mode=--register ;;
        1:--permissions) mode=--permissions ;;
        1:--restart) mode=--restart ;;
        *) fail 'Usage: sh ./install.sh [--permissions|--restart]' ;;
    esac
    user=$(/usr/bin/id -un)
    uid=$(/usr/bin/id -u)
    validate_package
    if [ "$mode" = --permissions ]; then
        if permissions_guide; then
            exec /usr/bin/sudo -- /bin/sh "$root/install.sh" --restart "$PATH"
        else
            outcome=$?
            [ "$outcome" -eq 1 ] || exit 1
        fi
        exit 0
    fi
    if [ "$mode" = --register ]; then
        if permissions_guide; then :; else
            outcome=$?
            [ "$outcome" -eq 1 ] || exit 1
        fi
    fi
    exec /usr/bin/sudo -- /bin/sh "$root/install.sh" "$mode" "$PATH"
fi

PATH=/usr/bin:/bin:/usr/sbin:/sbin
export PATH
[ "$#" -eq 2 ] && { [ "$1" = --register ] || [ "$1" = --restart ]; } ||
    fail 'Run sh ./install.sh from the screenshot account.'
mode=$1
user=${SUDO_USER:-}
case "$user" in ''|*[!a-zA-Z0-9_.-]*) fail 'Cannot identify the screenshot account.' ;; esac
uid=$(/usr/bin/id -u "$user")
[ "$uid" -ne 0 ] && [ "$uid" = "${SUDO_UID:-}" ] || fail 'Use a non-root screenshot account.'
validate_package

label="com.testflying.screenshot-connector.$node"
job="system/$label"
plist="/Library/LaunchDaemons/$label.plist"
if [ "$mode" = --restart ] && [ ! -e "$plist" ] && [ ! -L "$plist" ]; then
    fail 'This node is not installed. Run sh ./install.sh first.'
fi
stage=$(/usr/bin/mktemp "/Library/LaunchDaemons/.$label.XXXXXXXX")
trap '/bin/rm -f -- "$stage"' EXIT
trap 'exit 1' HUP INT TERM

/usr/bin/plutil -create xml1 "$stage"
plist_insert() { /usr/bin/plutil -insert "$@" -- "$stage"; }
plist_insert Label -string "$label"
plist_insert UserName -string "$user"
plist_insert WorkingDirectory -string "$root"
plist_insert ProgramArguments -array
plist_insert ProgramArguments.0 -string /bin/sh
plist_insert ProgramArguments.1 -string "$root/run.sh"
plist_insert ProgramArguments.2 -string --service
plist_insert EnvironmentVariables -dictionary
plist_insert EnvironmentVariables.HOME -string "$account_home"
plist_insert EnvironmentVariables.PATH -string "$2"
plist_insert RunAtLoad -bool YES
plist_insert KeepAlive -bool YES
plist_insert ThrottleInterval -integer 5
plist_insert ExitTimeOut -integer 60
/usr/bin/plutil -lint "$stage" >/dev/null
/bin/chmod 644 "$stage"

created=false
if [ -e "$plist" ] || [ -L "$plist" ]; then
    [ -f "$plist" ] && [ ! -L "$plist" ] &&
        [ "$(/usr/bin/stat -f '%u %Lp' "$plist")" = '0 644' ] ||
        fail 'A different installation uses this node ID. The existing job was preserved.'
    for key in Label UserName WorkingDirectory ProgramArguments RunAtLoad KeepAlive; do
        current=$(/usr/bin/plutil -extract "$key" xml1 -o - -- "$plist")
        wanted=$(/usr/bin/plutil -extract "$key" xml1 -o - -- "$stage")
        [ "$current" = "$wanted" ] ||
            fail 'A different installation uses this node ID. The existing job was preserved.'
    done
else
    if /bin/launchctl print "$job" >/dev/null 2>&1; then
        fail 'A different loaded job uses this node ID.'
    fi
    /bin/ln "$stage" "$plist" || fail 'A concurrent installation created this job.'
    created=true
fi

start_job() {
    /bin/launchctl enable "$job" || return 1
    if /bin/launchctl print "$job" >/dev/null 2>&1; then
        /bin/launchctl kill SIGTERM "$job" 2>/dev/null || /bin/launchctl kickstart "$job"
    else
        /bin/launchctl bootstrap system "$plist"
    fi
}
if ! start_job; then
    if "$created" && ! /bin/launchctl print "$job" >/dev/null 2>&1; then
        /bin/rm -f -- "$plist"
    fi
    fail 'launchd could not start the Screenshot Connector.'
fi
if [ "$mode" = --restart ]; then
    printf '%s\n' "Restart requested for $job; launchd relaunches the service after graceful shutdown."
else
    printf '%s\n' "Installed and started $job"
fi
printf '%s\n' "Status: sudo launchctl print $job"
