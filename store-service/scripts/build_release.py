"""Build only requested or changed components; preserve untouched program identities."""
from __future__ import annotations
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys

from verify_store_refactor import PROJECT, managed_root, source_hashes
from component_release import component_metadata_at
from release_catalog import select_components, finish_pending_publication, publish_outputs, _atomic_json

def main() -> None:
    root=managed_root()
    if len(sys.argv)>2 or platform.system()!="Darwin" or platform.machine()!="arm64":
        raise RuntimeError("Release builds require an Apple Silicon macOS host")
    mode=sys.argv[1] if len(sys.argv)==2 else "changed"
    finish_pending_publication(root,PROJECT)
    baseline=source_hashes()
    subprocess.run(["git","ls-files","--error-unmatch","--",*baseline],cwd=PROJECT,stdout=subprocess.DEVNULL,check=True)
    subprocess.run(["git","diff","--quiet","HEAD","--",*baseline],cwd=PROJECT,check=True)
    commit=subprocess.check_output(["git","rev-parse","HEAD"],cwd=PROJECT,text=True).strip()
    selected,metadata,previous=select_components(mode)
    for kind in selected:
        if component_metadata_at(kind,commit)!=metadata[kind]:
            raise RuntimeError("Commit component production inputs before building: "+kind)
    print("Selected components: "+", ".join(sorted(selected)),flush=True)
    if not selected:
        print("All published components are unchanged",flush=True)
        return
    tmp=root/"tmp";tmp.mkdir(exist_ok=True)
    source=tmp/"source"
    for relative in baseline:
        if relative.startswith("scripts/"):continue
        destination=source/relative
        destination.parent.mkdir(parents=True,exist_ok=True)
        prefix=subprocess.check_output(["git","rev-parse","--show-prefix"],cwd=PROJECT,text=True).strip()
        destination.write_bytes(subprocess.check_output(["git","show",commit+":"+prefix+relative],cwd=PROJECT))
    packages=tmp/"python-packages"
    env={**os.environ,"RUSTUP_HOME":str(tmp/"rustup"),"CARGO_HOME":str(tmp/"cargo-home"),
         "RUSTUP_TOOLCHAIN":"1.94.0","CARGO_TARGET_DIR":str(tmp/"target"),
         "CARGO_INCREMENTAL":"0","CARGO_BUILD_JOBS":"3","TMPDIR":str(tmp),
         "XDG_CACHE_HOME":str(tmp/"cache"),"PIP_CACHE_DIR":str(tmp/"pip-cache"),
         "ZIG_GLOBAL_CACHE_DIR":str(tmp/"zig-global"),"ZIG_LOCAL_CACHE_DIR":str(tmp/"zig-local"),
         "CARGO_ZIGBUILD_CACHE_DIR":str(tmp/"zigbuild"),
         "CARGO_ZIGBUILD_ZIG_PATH":str(packages/"ziglang/zig"),"PYTHONPATH":str(packages),
         "PYTHONDONTWRITEBYTECODE":"1","PATH":str(packages/"bin")+os.pathsep+os.environ["PATH"]}
    outputs={}
    commands=[]
    triples=[]
    if "center" in selected or ("store" in selected and mode!="windows"):
        triples.append("x86_64-unknown-linux-musl")
    if "store" in selected:
        triples.append("x86_64-pc-windows-gnu")
        dlltool=Path("/opt/homebrew/opt/llvm/bin/llvm-dlltool")
        if not dlltool.is_file():raise RuntimeError("Trusted LLVM dlltool is required")
        env["CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS"]="-C dlltool="+str(dlltool)
    if triples:
        commands.append([sys.executable,"-m","pip","install","--disable-pip-version-check",
                         "--no-cache-dir","--only-binary=:all:","--target",str(packages),
                         "cargo-zigbuild==0.20.1","ziglang==0.14.1"])
    commands.append([shutil.which("rustup") or "rustup","toolchain","install","1.94.0",
                     "--profile","minimal","--no-self-update"])
    for triple in triples:
        commands.append(["rustup","target","add",triple,"--toolchain","1.94.0"])
    if "center" in selected:
        commands.append(["cargo","zigbuild","--locked","--release","--target","x86_64-unknown-linux-musl","-p","store-api"])
        outputs["releases/linux-amd64/store-api"]=(tmp/"target/x86_64-unknown-linux-musl/release/store-api","center")
    if "store" in selected:
        if mode!="windows":
            commands.extend([
                ["cargo","zigbuild","--locked","--release","--target","x86_64-unknown-linux-musl","-p","store-connector"],
                ["cargo","build","--locked","--release","-p","store-connector"]])
            outputs["releases/linux-amd64/store-connector"]=(tmp/"target/x86_64-unknown-linux-musl/release/store-connector","store")
            outputs["releases/darwin-arm64/store-connector"]=(tmp/"target/release/store-connector","store")
        commands.append(["cargo","zigbuild","--locked","--release","--target","x86_64-pc-windows-gnu","-p","store-connector"])
        outputs["releases/windows-amd64/store-connector.exe"]=(tmp/"target/x86_64-pc-windows-gnu/release/store-connector.exe","store")
    if "screenshot" in selected:
        commands.append(["cargo","build","--locked","--release","-p","screenshot-connector"])
        outputs["releases/darwin-arm64/screenshot-connector"]=(tmp/"target/release/screenshot-connector","screenshot")
    for command in commands:
        print("Running: "+" ".join(command),flush=True)
        attempts=3 if "rustup" in Path(command[0]).name else 1
        for attempt in range(attempts):
            try:subprocess.run(command,cwd=source,env=env,check=True);break
            except subprocess.CalledProcessError:
                if attempt+1==attempts:raise
    if source_hashes()!=baseline:raise RuntimeError("Source changed during release build")
    if subprocess.check_output(["git","rev-parse","HEAD"],cwd=PROJECT,text=True).strip()!=commit:
        raise RuntimeError("Source commit changed during build")
    finished=datetime.datetime.now(datetime.timezone.utc).isoformat()
    merged=dict(previous)
    for relative,(binary,kind) in outputs.items():
        description=subprocess.check_output(["file",str(binary)],text=True).strip()
        expected="PE32+ executable" if "windows" in relative else "ELF 64-bit" if "linux" in relative else "Mach-O 64-bit executable arm64"
        if expected not in description:raise RuntimeError("Unexpected release architecture: "+description)
        if kind=="screenshot" and subprocess.check_output(["lipo","-archs",str(binary)],text=True).split()!=["arm64"]:
            raise RuntimeError("Screenshot Connector must contain only arm64")
        imported=[]
        if "windows" in relative:
            details=subprocess.check_output(["/opt/homebrew/opt/llvm/bin/llvm-objdump","-p",str(binary)],text=True)
            imported=sorted(set(re.findall(r"DLL Name:\s*(\S+)",details)))
            allowed={"advapi32.dll","bcrypt.dll","bcryptprimitives.dll","crypt32.dll","kernel32.dll",
                     "msvcrt.dll","ntdll.dll","ole32.dll","secur32.dll","shell32.dll","user32.dll",
                     "userenv.dll","ws2_32.dll","iphlpapi.dll","synchronization.dll","kernelbase.dll"}
            if not imported or any(n.lower() not in allowed and not n.lower().startswith("api-ms-win-") for n in imported):
                raise RuntimeError("Unexpected non-system Windows DLL")
        merged[relative]={**metadata[kind],"sourceCommit":commit,"builtAt":finished,
                          "sha256":hashlib.sha256(binary.read_bytes()).hexdigest(),"bytes":binary.stat().st_size,
                          "format":description.split(": ",1)[-1]}
        if imported:merged[relative]["importedDlls"]=imported
    store=merged["releases/linux-amd64/store-connector"]
    report={"schemaVersion":2,"runId":root.name,"finishedAt":finished,"sourceCommit":commit,
            "sourceHashes":baseline,"builtComponents":sorted(selected),"builtOutputs":sorted(outputs),
            "retainedOutputs":sorted(set(previous)-set(outputs)),"outputs":merged,
            "connectorVersion":store["version"],"connectorSourceFingerprint":store["sourceFingerprint"],
            "connectorSourceHashes":store["sourceHashes"],
            "components":{kind:next(v for v in merged.values() if v["component"]==kind) for kind in metadata},
            "cleanup":"See the supervisor cleanup report for this runId"}
    windows=None
    if "releases/windows-amd64/store-connector.exe" in outputs:
        windows={**report,"outputs":{"releases/windows-amd64/store-connector.exe":merged["releases/windows-amd64/store-connector.exe"]}}
        win=merged["releases/windows-amd64/store-connector.exe"]
        windows.update({"connectorVersion":win["version"],"connectorSourceFingerprint":win["sourceFingerprint"],
                        "connectorSourceHashes":win["sourceHashes"],"components":{"store":win}})
    publish_outputs(root,report,windows,{relative:binary for relative,(binary,_) in outputs.items()},PROJECT)
    if selected=={"center"}:
        _atomic_json(root,PROJECT/"docs/setup-api-build.json",report)
    print("Published selected formal programs; retained all other binary bytes and provenance",flush=True)

if __name__=="__main__":main()
