"""Select immutable component outputs without coupling their versions or build snapshots."""
from __future__ import annotations

import json
import re
import subprocess
import hashlib
import os
import fcntl
from contextlib import contextmanager
from pathlib import Path

from component_release import (
    COMPONENT_PACKAGES, OUTPUT_COMPONENTS, component_metadata, guard_component,
    normalize_report, verify_output_binary,component_metadata_at,
)
from managed_workspace import PROJECT

REPORT_NAMES = ("deployment-build.json", "windows-release-build.json", "setup-api-build.json")
REQUIRED_OUTPUTS = frozenset(OUTPUT_COMPONENTS)

def report_paths(project: Path = PROJECT) -> list[Path]:
    main = project / "docs/deployment-build.json"
    windows = project / "docs/windows-release-build.json"
    api = project / "docs/setup-api-build.json"
    paths = [path for path in (main, windows) if path.is_file()]
    if api.is_file() and (not main.is_file() or
        json.loads(api.read_text()).get("finishedAt", "") > json.loads(main.read_text()).get("finishedAt", "")):
        paths.append(api)
    return paths

def git_source(commit:str,relative:str,project:Path=PROJECT)->bytes:
    if re.fullmatch(r"[0-9a-f]{40}",commit) is None or relative.startswith("/") or ".." in Path(relative).parts:
        raise RuntimeError("Exact committed project source required")
    prefix=subprocess.check_output(["git","rev-parse","--show-prefix"],cwd=project,text=True).strip()
    return subprocess.check_output(["git","show",commit+":"+prefix+relative],cwd=project)

def installer_asset_names(kind:str)->tuple[str,...]:
    return ("start.cmd","start.sh","install.cmd","install.ps1","install.sh","README.md") if kind=="store" else ("install.sh","run.sh","README.md")

def release_outputs(project: Path = PROJECT) -> dict:
    chosen = {}
    reports = {}
    for path in report_paths(project):
        report = json.loads(path.read_text())
        if "pendingPublication" in report:
            report=normalize_report(report,project)
            pending=normalize_report(report["pendingPublication"]["report"],project)
            for relative in pending["builtOutputs"]:
                candidate=pending["outputs"][relative]
                try:verify_output_binary(relative,candidate,project)
                except RuntimeError:continue
                report["outputs"][relative]=candidate
        reports[path] = report
        for relative, output in report.get("outputs", {}).items():
            stamp = output.get("builtAt", report.get("finishedAt", ""))
            if relative not in chosen or stamp > chosen[relative][0]:
                chosen[relative] = (stamp, path)
    outputs = {}
    for path, report in reports.items():
        selected = {relative: report["outputs"][relative] for relative, (_, owner) in chosen.items() if owner == path}
        if selected:
            normalized = normalize_report({**report, "outputs": selected}, project)
            for relative, output in normalized["outputs"].items():
                outputs[relative] = {**output, "builtAt": chosen[relative][0]}
    if set(outputs) != REQUIRED_OUTPUTS:
        raise RuntimeError("All five existing platform outputs must be recorded before selective publication")
    return outputs

def versions(project: Path = PROJECT) -> dict[str, str]:
    # Version declarations themselves are cheap and do not resolve the shared lock graph.
    from component_release import component_version
    return {kind: component_version(kind,project) for kind in COMPONENT_PACKAGES}

def select_components(mode: str, project: Path = PROJECT) -> tuple[set[str], dict, dict]:
    aliases = {"api":"center", "api-screenshots":"center-screenshots"}
    mode = aliases.get(mode,mode)
    explicit = {"center":{"center"},"screenshots":{"screenshot"},"store":{"store"},
                "windows":{"store"},"center-screenshots":{"center","screenshot"},
                "all":set(COMPONENT_PACKAGES)}
    if mode not in {"changed", *explicit}:
        raise RuntimeError("Use changed, center, screenshots, store, windows or all")
    previous = release_outputs(project)
    metadata = {kind:component_metadata(kind,project) for kind in COMPONENT_PACKAGES}
    selected = set(explicit.get(mode,()))
    if mode == "changed":
        for kind,current in metadata.items():
            # A workspace-wide lock can acquire dev/other-component feature edges. Reusing
            # an existing binary does not claim that it was rebuilt with that new lock.
            direct = {p:h for p,h in current["sourceHashes"].items() if p != "Cargo.lock"}
            for output in previous.values():
                if output["component"] != kind:
                    continue
                prior_direct = {p:h for p,h in output["sourceHashes"].items() if p != "Cargo.lock"}
                if output["version"] != current["version"] or direct != prior_direct:
                    selected.add(kind)
    for kind in selected:
        guard_component(kind,report_paths(project),project=project)
        from connector_release import version_tuple
        current=metadata[kind]
        for prior in previous.values():
            if prior["component"]!=kind:continue
            if version_tuple(current["version"])<version_tuple(prior["version"]):
                raise RuntimeError("Component version downgrade rejected: "+kind)
            if current["sourceFingerprint"]!=prior["sourceFingerprint"] and version_tuple(current["version"])<=version_tuple(prior["version"]):
                raise RuntimeError("Changed component requires a higher own version: "+kind)
    for relative,output in previous.items():
        verify_output_binary(relative,output,project)
    catalog = json.loads((project/"packaging/connector-versions.json").read_text())
    expected = {kind:metadata[kind]["version"] for kind in ("store","screenshot")}
    if catalog != expected:
        raise RuntimeError("Publication catalog must match each node's own Cargo version")
    return selected,metadata,previous

def assert_ready(relative: str, output: dict, project: Path = PROJECT) -> None:
    kind = OUTPUT_COMPONENTS[relative]
    current = component_metadata(kind,project)
    if output["component"] != kind or any(output[key] != current[key] for key in
                                          ("version","sourceFingerprint","sourceHashes")):
        raise RuntimeError("Build the changed component before publication: " + relative)
    verify_output_binary(relative,output,project)

def published_outputs(publication: dict, project: Path = PROJECT) -> dict:
    if "publishedOutputs" in publication:
        outputs=normalize_report({"schemaVersion":2,"outputs":publication["publishedOutputs"]},project)["outputs"]
    else:
        commit=publication["deploymentCommit"]
        if re.fullmatch(r"[0-9a-f]{40}",commit) is None:
            raise RuntimeError("Published history requires an exact source commit")
        prefix=subprocess.check_output(["git","rev-parse","--show-prefix"],cwd=project,text=True).strip()
        outputs={}
        for name in ("deployment-build.json","windows-release-build.json","setup-api-build.json"):
            data=git_source(commit,"docs/"+name,project)
            report=json.loads(data)
            expected_paths={
                "releases/linux-amd64/store-api":publication["apiSha256"],
                "releases/darwin-arm64/screenshot-connector":publication["screenshotConnectorSha256"],
                **{f"releases/{t}/"+("store-connector.exe" if t.startswith("windows") else "store-connector"):sha
                   for t,sha in publication["connectorBinarySha256"].items()},
            }
            selected={p:o for p,o in report["outputs"].items() if o["sha256"]==expected_paths.get(p)}
            if selected:outputs.update(normalize_report({**report,"outputs":selected},project)["outputs"])
    if set(outputs)!=REQUIRED_OUTPUTS:
        raise RuntimeError("Published component history is incomplete")
    expected={
        "releases/linux-amd64/store-api":publication["apiSha256"],
        "releases/darwin-arm64/screenshot-connector":publication["screenshotConnectorSha256"],
        **{f"releases/{target}/"+("store-connector.exe" if target.startswith("windows") else "store-connector"):digest
           for target,digest in publication["connectorBinarySha256"].items()},
    }
    if any(output["sha256"]!=expected.get(path) for path,output in outputs.items()):
        raise RuntimeError("Published binary hashes do not match their component provenance")
    return outputs

def deployment_outputs(mode:str, publication:dict, project:Path=PROJECT)->tuple[dict,set[str]]:
    previous=published_outputs(publication,project)
    available=release_outputs(project)
    aliases={"api":"center","api-docs":"center","integrations":"center",
             "screenshots":"screenshot","config-update":"changed"}
    mode=aliases.get(mode,mode)
    if mode not in ("changed","center","store","screenshot","all"):
        raise RuntimeError("Choose changed, center, store or screenshots for deployment")
    output=dict(previous);changed=set()
    from component_release import component_metadata
    requested_kinds=set(OUTPUT_COMPONENTS.values()) if mode in ("changed","all") else {mode}
    requested=set()
    for kind in requested_kinds:
        current=component_metadata(kind,project)
        direct={p:h for p,h in current["sourceHashes"].items() if p!="Cargo.lock"}
        for old in previous.values():
            if old["component"]!=kind:continue
            if old["version"]!=current["version"] or {p:h for p,h in old["sourceHashes"].items() if p!="Cargo.lock"}!=direct:
                requested.add(kind)
    for relative,kind in OUTPUT_COMPONENTS.items():
        if kind in requested:assert_ready(relative,available[relative],project)
    for relative,candidate in available.items():
        kind=candidate["component"]
        if mode not in ("changed","all",kind):continue
        if candidate["sha256"]==previous[relative]["sha256"]:continue
        assert_ready(relative,candidate,project)
        output[relative]=candidate;changed.add(kind)
    if "store" in changed:
        identities={(v["version"],v["sourceFingerprint"]) for v in output.values() if v["component"]=="store"}
        if len(identities)!=1:raise RuntimeError("Build all Store platforms for the selected Store release")
    center=output["releases/linux-amd64/store-api"]
    for kind in ({"store","screenshot"} if "center" in changed else changed-{"center"}):
        node=next(v for v in output.values() if v["component"]==kind)
        compatible=component_metadata_at(kind,center["sourceCommit"],project)
        packaging=lambda hashes:{p:h for p,h in hashes.items()
                                  if p.startswith("crates/store-api/") or p=="deploy/testflying-prod-ca.crt"}
        if packaging(node["sourceHashes"])!=packaging(compatible["sourceHashes"]):
            raise RuntimeError("The selected node also needs its changed center-side package generator; publish center together")
    return output,changed

def _atomic_json(root:Path,path:Path,data:dict)->None:
    staged=root/"tmp"/("publication-"+path.name)
    staged.parent.mkdir(exist_ok=True)
    with staged.open("w") as file:
        json.dump(data,file,indent=2);file.write("\n");file.flush();os.fsync(file.fileno())
    if staged.stat().st_dev!=path.parent.stat().st_dev:
        raise RuntimeError("Formal publication requires same-filesystem atomic rename")
    os.replace(staged,path)

@contextmanager
def _publication_lock(project:Path):
    directory=project/"docs"
    descriptor=os.open(directory,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW)
    try:
        fcntl.flock(descriptor,fcntl.LOCK_EX|fcntl.LOCK_NB)
        opened=os.fstat(descriptor);current=directory.stat()
        if (opened.st_dev,opened.st_ino)!=(current.st_dev,current.st_ino):
            raise RuntimeError("Formal publication directory identity changed")
        yield
    finally:
        os.close(descriptor)

def finish_pending_publication(root:Path,project:Path=PROJECT)->bool:
    with _publication_lock(project):
        return _finish_pending_publication(root,project)

def _finish_pending_publication(root:Path,project:Path)->bool:
    path=project/"docs/deployment-build.json"
    existing=json.loads(path.read_text())
    pending=existing.get("pendingPublication")
    if not pending:return True
    final=normalize_report(pending["report"],project)
    for relative in final["builtOutputs"]:
        try:verify_output_binary(relative,final["outputs"][relative],project)
        except RuntimeError:return False
    if pending.get("windowsReport"):
        _atomic_json(root,project/"docs/windows-release-build.json",pending["windowsReport"])
    _atomic_json(root,path,final)
    return True

def publish_outputs(root:Path,report:dict,windows:dict|None,binaries:dict,project:Path=PROJECT)->None:
    with _publication_lock(project):
        _publish_outputs(root,report,windows,binaries,project)

def _publish_outputs(root:Path,report:dict,windows:dict|None,binaries:dict,project:Path)->None:
    path=project/"docs/deployment-build.json"
    latest=release_outputs(project)
    from connector_release import version_tuple
    for relative,actual in latest.items():
        if relative not in binaries:
            report["outputs"][relative]=actual
        else:
            candidate=report["outputs"][relative]
            if version_tuple(candidate["version"])<version_tuple(actual["version"]) or (
                candidate["version"]==actual["version"] and candidate["sourceFingerprint"]!=actual["sourceFingerprint"]
            ):raise RuntimeError("Another publisher changed this component; refuse to replace its identity")
    store=report["outputs"]["releases/linux-amd64/store-connector"]
    report.update({"connectorVersion":store["version"],"connectorSourceFingerprint":store["sourceFingerprint"],
                   "connectorSourceHashes":store["sourceHashes"]})
    report["components"]={kind:next(v for v in report["outputs"].values() if v["component"]==kind)
                          for kind in COMPONENT_PACKAGES}
    # Validate every program and its exact committed provenance before touching formal files.
    normalize_report(report,project)
    for relative,binary in binaries.items():
        target=project/relative
        target.parent.mkdir(parents=True,exist_ok=True)
        if target.is_symlink() or binary.stat().st_dev!=target.parent.stat().st_dev:
            raise RuntimeError("Formal binary requires a regular target and atomic same-filesystem rename")
        expected=report["outputs"][relative]
        if binary.stat().st_size!=expected["bytes"] or hashlib.sha256(binary.read_bytes()).hexdigest()!=expected["sha256"]:
            raise RuntimeError("Staged binary identity changed before publication")
    previous=normalize_report(json.loads(path.read_text()),project)
    previous["outputs"]=release_outputs(project)
    previous["pendingPublication"]={"report":report,"windowsReport":windows}
    _atomic_json(root,path,previous)
    for relative,binary in binaries.items():
        binary.chmod(0o755)
        os.replace(binary,project/relative)
    if not _finish_pending_publication(root,project):
        raise RuntimeError("Publication is incomplete; the next build can recover the recorded transaction")
