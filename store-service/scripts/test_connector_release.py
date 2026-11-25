"""Synthetic component release fixtures. Run inside artifact-run."""
from __future__ import annotations

import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

import component_release as release
import connector_release as compat
from managed_workspace import managed_root


API = "releases/linux-amd64/store-api"
STORE = "releases/linux-amd64/store-connector"
WINDOWS = "releases/windows-amd64/store-connector.exe"
SCREENSHOT = "releases/darwin-arm64/screenshot-connector"
REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


class ComponentReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = managed_root() / "tmp"
        temporary.mkdir(exist_ok=True)
        fixture = tempfile.TemporaryDirectory(prefix="release-fixture-", dir=temporary)
        self.addCleanup(fixture.cleanup)
        self.project = Path(fixture.name) / "store-service"
        self.project.mkdir()
        self.reports = self.project.parent / "reports"
        self.reports.mkdir()
        self.base_report = self.reports / "deployment-build.json"
        self.windows_report = self.reports / "windows-release-build.json"
        self.write("Cargo.toml", '''
[workspace]
members = ["crates/store-api", "crates/store-connector", "crates/screenshot-connector"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"

[workspace.dependencies]
store-protocol = { path = "crates/store-protocol" }
store-provider = { path = "crates/store-provider" }
''')
        packages = {
            "store-api": ("0.1.1", '''
[dependencies]
store-provider.workspace = true
center-runtime = "1"
[dev-dependencies]
store-connector = { path = "../store-connector" }
'''),
            "store-connector": ("0.2.8", '''
[dependencies]
store-provider.workspace = true
[dev-dependencies]
dev-helper = "1"
'''),
            "screenshot-connector": ("0.2.9", '''
[dependencies]
store-protocol.workspace = true
'''),
            "store-provider": (None, '''
[dependencies]
store-protocol.workspace = true
store-runtime = "1"
'''),
            "store-protocol": (None, ""),
        }
        for name, (version, dependencies) in packages.items():
            version_line = f'version = "{version}"' if version else "version.workspace = true"
            self.write(f"crates/{name}/Cargo.toml",
                       f'[package]\nname = "{name}"\n{version_line}\nedition.workspace = true\n' + dependencies)
        self.write("Cargo.lock", "version = 4\n" + "".join([
            self.lock_item("store-api", "0.1.1", ["center-runtime", "store-connector", "store-provider"]),
            self.lock_item("store-connector", "0.2.8", ["dev-helper", "store-provider"]),
            self.lock_item("screenshot-connector", "0.2.9", ["store-protocol"]),
            self.lock_item("store-provider", "0.1.0", ["store-protocol", "store-runtime"]),
            self.lock_item("store-protocol", "0.1.0"),
            self.lock_item("store-runtime", "1.0.0", ["transitive"], source=REGISTRY),
            self.lock_item("transitive", "1.0.0", source=REGISTRY),
            self.lock_item("center-runtime", "1.0.0", source=REGISTRY),
            self.lock_item("dev-helper", "1.0.0", source=REGISTRY),
        ]))
        self.write("crates/store-protocol/src/lib.rs", "pub fn production() {}\n")
        self.write(compat.RELEASE, 'pub const CONNECTOR_VERSION: &str = "0.2.8";\n')
        self.write("crates/store-provider/src/lib.rs", "pub fn provider() {}\n")
        self.write("crates/store-connector/src/lib.rs", "pub fn connector() {}\n")
        self.write("crates/screenshot-connector/src/main.rs", "fn main() {}\n")
        self.write("crates/screenshot-connector/src/worker.rs", "fn worker() {}\n")
        self.write(release.API_LIB, '''
const DOCS: &str = include_str!("../../../docs/store-api.md");
const SETUP: &[u8] = include_bytes!("setup/app.mjs");
fn config(state: &ApiState, node: &Connector) -> Value {
    if node.kind == ConnectorKind::Screenshot {
        return json!({"screenshotSetting":10});
    }
    json!({"requestTimeoutSeconds":60})
}
async fn installation_config() {}
async fn installer() {}
fn business() {}
''')
        self.write(release.API_SCREENSHOTS, '''
const ARM64_MACHO_HEADER: &[u8] = b"header";
fn screenshot_installer() {}
fn installer_archive() {}
fn release_unavailable() {}
''')
        self.write(release.API_SETUP_APP, '''
function downloadFile(bytes) {
  save(bytes);
}
$("setupForm").addEventListener("submit", () => {
  buildInstaller();
});
$("screenshotProvision").addEventListener("submit", () => {
  createScreenshotNode();
});
$("screenshotDownload").addEventListener("click", () => {
  downloadScreenshotInstaller();
});
export const ready = true;
''')
        self.write("docs/store-api.md", "# API docs\n")
        self.write("packaging/connector/install.sh", "#!/bin/sh\nexit 0\n")
        self.write("packaging/screenshot-connector/run.sh", "#!/bin/sh\nexit 0\n")
        self.git("init", "-q", "--template=", str(self.project.parent))
        self.commit()
        self.record()

    @staticmethod
    def lock_item(name, version, dependencies=(), *, source=None):
        result = f'\n[[package]]\nname = "{name}"\nversion = "{version}"\n'
        if source:
            result += f'source = "{source}"\nchecksum = "{"a" * 64}"\n'
        if dependencies:
            result += "dependencies = " + json.dumps(dependencies) + "\n"
        return result

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-c", "user.name=Release Tests", "-c", "user.email=tests@example.invalid",
             "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", *args],
            cwd=self.project, text=True,
        ).strip()

    def commit(self):
        self.git("add", "--", ".")
        self.git("commit", "-qm", "Fixture release sources")
        return self.git("rev-parse", "HEAD")

    def write(self, path: str, content: str):
        destination = self.project / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content)

    def replace(self, path: str, old: str, new: str):
        data = (self.project / path).read_text()
        self.assertIn(old, data)
        self.write(path, data.replace(old, new, 1))

    def set_version(self, kind, previous, current):
        name = release.COMPONENT_PACKAGES[kind]
        self.replace(f"crates/{name}/Cargo.toml", f'version = "{previous}"', f'version = "{current}"')
        self.replace("Cargo.lock", f'name = "{name}"\nversion = "{previous}"',
                     f'name = "{name}"\nversion = "{current}"')

    def record(self):
        commit = self.git("rev-parse", "HEAD")
        for path, outputs in ((self.base_report, (API, STORE, SCREENSHOT)), (self.windows_report, (WINDOWS,))):
            path.write_text(json.dumps({"schemaVersion": release.REPORT_SCHEMA_VERSION, "sourceCommit": commit, "outputs": {
                relative: {**release.component_metadata(release.OUTPUT_COMPONENTS[relative], self.project),
                           "sourceCommit": commit, "sha256": "a" * 64, "bytes": 12}
                for relative in outputs
            }}))

    def guard(self, kind, *, unchanged=False, paths=None):
        return release.guard_component(kind, [self.base_report, self.windows_report] if paths is None else paths,
                                       project=self.project, require_unchanged=unchanged)

    def catalog_fixture(self):
        from release_catalog import REQUIRED_OUTPUTS
        commit=self.git("rev-parse","HEAD")
        outputs={}
        for relative in REQUIRED_OUTPUTS:
            kind=release.OUTPUT_COMPONENTS[relative]
            data=("old-"+kind).encode()
            binary=self.project/relative
            binary.parent.mkdir(parents=True,exist_ok=True);binary.write_bytes(data)
            outputs[relative]={**release.component_metadata(kind,self.project),"sourceCommit":commit,
                               "sha256":hashlib.sha256(data).hexdigest(),"bytes":len(data),"builtAt":"2020"}
        self.catalog_write(outputs)
        return {"publishedOutputs":outputs,"apiSha256":outputs[API]["sha256"],
                "screenshotConnectorSha256":outputs[SCREENSHOT]["sha256"],
                "connectorBinarySha256":{target:outputs[f"releases/{target}/"+("store-connector.exe" if target.startswith("windows") else "store-connector")]["sha256"]
                                        for target in ("linux-amd64","darwin-arm64","windows-amd64")}}

    def catalog_write(self,outputs,built=()):
        report={"schemaVersion":2,"outputs":outputs,"builtOutputs":list(built),"finishedAt":"2026"}
        self.write("docs/deployment-build.json",json.dumps(report))
        self.write("docs/windows-release-build.json",json.dumps({**report,"outputs":{WINDOWS:outputs[WINDOWS]}}))
        return report

    def test_default_deployment_rejects_an_unbuilt_version_even_when_binary_sha_matches(self):
        from release_catalog import deployment_outputs
        publication=self.catalog_fixture()
        self.set_version("screenshot","0.2.9","0.2.10");self.commit()
        with self.assertRaisesRegex(RuntimeError,"Build the changed component"):
            deployment_outputs("changed",publication,self.project)

    def test_node_only_publication_retains_center_with_unpublished_center_document_changes(self):
        from release_catalog import deployment_outputs
        publication=self.catalog_fixture();outputs=copy.deepcopy(publication["publishedOutputs"])
        self.set_version("screenshot","0.2.9","0.2.10")
        self.set_version("center","0.1.1","0.1.2")
        self.write("docs/store-api.md","# Unpublished center document")
        commit=self.commit();data=b"new screenshot"
        (self.project/SCREENSHOT).write_bytes(data)
        outputs[SCREENSHOT]={**release.component_metadata("screenshot",self.project),"sourceCommit":commit,
                             "sha256":hashlib.sha256(data).hexdigest(),"bytes":len(data),"builtAt":"2027"}
        self.catalog_write(outputs,[SCREENSHOT])
        selected,changed=deployment_outputs("screenshots",publication,self.project)
        self.assertEqual(changed,{"screenshot"})
        self.assertEqual(selected[API],publication["publishedOutputs"][API])
        with self.assertRaisesRegex(RuntimeError,"Build the changed component"):
            deployment_outputs("changed",publication,self.project)

    def test_formal_publication_journal_recovers_after_interrupted_multi_output_rename(self):
        from release_catalog import publish_outputs,finish_pending_publication,release_outputs
        from unittest.mock import patch
        import os
        publication=self.catalog_fixture();outputs=copy.deepcopy(publication["publishedOutputs"])
        self.set_version("screenshot","0.2.9","0.2.10");self.set_version("center","0.1.1","0.1.2")
        commit=self.commit()
        staged={}
        for relative,kind in [(API,"center"),(SCREENSHOT,"screenshot")]:
            data=("new-"+kind).encode()
            binary=managed_root()/"tmp"/("candidate-"+kind)
            binary.write_bytes(data);staged[relative]=binary
            outputs[relative]={**release.component_metadata(kind,self.project),"sourceCommit":commit,
                               "sha256":hashlib.sha256(data).hexdigest(),"bytes":len(data),"builtAt":"2027"}
        report={"schemaVersion":2,"outputs":outputs,"builtOutputs":[API,SCREENSHOT],"finishedAt":"2027"}
        original=os.replace;count=0
        def interrupt(source,target):
            nonlocal count
            count+=1
            if count==3:raise OSError("Fixture interruption")
            original(source,target)
        with patch("release_catalog.os.replace",side_effect=interrupt):
            with self.assertRaises(OSError):
                publish_outputs(managed_root(),report,None,staged,self.project)
        self.assertIn("pendingPublication",json.loads((self.project/"docs/deployment-build.json").read_text()))
        self.assertEqual(release_outputs(self.project)[API]["sha256"],outputs[API]["sha256"])
        original(staged[SCREENSHOT],self.project/SCREENSHOT)
        self.assertTrue(finish_pending_publication(managed_root(),self.project))
        final=json.loads((self.project/"docs/deployment-build.json").read_text())
        self.assertNotIn("pendingPublication",final)
        self.assertEqual(final["outputs"][STORE],publication["publishedOutputs"][STORE])

    def assert_changed(self, affected):
        for kind in release.COMPONENT_PACKAGES:
            if kind in affected:
                with self.assertRaisesRegex(RuntimeError, "increase " + kind + " version"):
                    self.guard(kind)
            else:
                self.guard(kind, unchanged=True)

    def test_store_compatibility_uses_store_cargo_identity(self):
        metadata = self.guard("store", unchanged=True)
        self.assertEqual(compat.connector_version(self.project), "0.2.8")
        self.assertEqual(compat.connector_source_fingerprint(self.project), metadata["sourceFingerprint"])
        self.assertEqual(compat.connector_source_hashes(self.project), metadata["sourceHashes"])
        self.set_version("store", "0.2.8", "0.2.9")
        self.assertEqual(compat.guard_connector_release([self.base_report], project=self.project)["connectorVersion"],
                         "0.2.9")
        self.assertEqual(release.component_version("screenshot", self.project), "0.2.9")

    def test_center_assets_and_embedded_api_docs_only_affect_center(self):
        for path in ("crates/store-api/src/setup/app.mjs", "docs/store-api.md"):
            with self.subTest(path=path):
                baseline = (self.project / path).read_text()
                self.write(path, baseline + "changed\n")
                self.assert_changed({"center"})
                self.write(path, baseline)
        self.set_version("center", "0.1.1", "0.1.2")
        self.assertEqual(self.guard("center")["version"], "0.1.2")
        self.guard("store", unchanged=True)
        self.guard("screenshot", unchanged=True)

    def test_screenshot_source_and_its_scripts_do_not_affect_store(self):
        self.write("crates/screenshot-connector/src/worker.rs", "fn new_worker() {}\n")
        self.write("packaging/screenshot-connector/run.sh", "#!/bin/sh\nexit 1\n")
        self.assert_changed({"screenshot"})
        self.set_version("screenshot", "0.2.9", "0.2.10")
        self.assertEqual(self.guard("screenshot")["version"], "0.2.10")
        with self.assertRaisesRegex(RuntimeError, "inputs or version changed"):
            self.guard("screenshot", unchanged=True)

    def test_store_source_and_its_scripts_do_not_affect_screenshot(self):
        self.write("crates/store-connector/src/lib.rs", "pub fn changed_connector() {}\n")
        self.write("packaging/connector/install.sh", "#!/bin/sh\nexit 1\n")
        self.assert_changed({"store"})

    def test_packaging_functions_follow_their_consumers(self):
        self.replace(release.API_LIB, "async fn installer() {}", "async fn installer() { changed(); }")
        self.assert_changed({"center", "store"})
        self.replace(release.API_LIB, "async fn installer() { changed(); }", "async fn installer() {}")
        self.replace(release.API_SCREENSHOTS, "fn installer_archive() {}", "fn installer_archive() { changed(); }")
        self.assert_changed({"center", "screenshot"})

    def test_packaging_config_branches_and_browser_orchestration_are_scoped(self):
        for path, old, new, kind in (
            (release.API_LIB, '"requestTimeoutSeconds":60', '"requestTimeoutSeconds":61', "store"),
            (release.API_LIB, '"screenshotSetting":10', '"screenshotSetting":11', "screenshot"),
            (release.API_SETUP_APP, "buildInstaller();", "buildInstaller(target);", "store"),
            (release.API_SETUP_APP, "downloadScreenshotInstaller();", "downloadScreenshotInstaller(id);", "screenshot"),
            (release.API_SCREENSHOTS, 'b"header"', 'b"other"', "screenshot"),
        ):
            with self.subTest(path=path, change=new):
                self.replace(path, old, new)
                self.assert_changed({"center", kind})
                self.replace(path, new, old)

    def test_shared_production_dependencies_affect_actual_consumers(self):
        self.write("crates/store-provider/src/lib.rs", "pub fn changed_provider() {}\n")
        self.assert_changed({"center", "store"})
        self.write("crates/store-protocol/src/lib.rs", "pub fn changed_protocol() {}\n")
        self.assert_changed({"center", "store", "screenshot"})

    def test_lock_noise_and_publication_catalog_do_not_affect_inputs(self):
        self.replace("Cargo.toml", 'members = [', 'members = ["crates/unrelated", ')
        self.write("crates/unrelated/Cargo.toml", '[package]\nname = "unrelated"\nversion = "2.0.0"\n')
        self.replace("Cargo.lock", 'name = "dev-helper"\nversion = "1.0.0"',
                     'name = "dev-helper"\nversion = "1.0.1"')
        self.replace("Cargo.lock", '"transitive"', '"transitive 1.0.0"')
        self.write("Cargo.lock", (self.project / "Cargo.lock").read_text()
                   + self.lock_item("transitive", "2.0.0", source=REGISTRY)
                   + self.lock_item("unrelated", "2.0.0", ["transitive 2.0.0"]))
        self.write("Cargo.toml", (self.project / "Cargo.toml").read_text()
                   + '\n[profile.release.package.unrelated]\nopt-level = 1\n')
        self.replace(compat.MANIFEST, 'dev-helper = "1"', 'dev-helper = "1.0.1"')
        self.write("packaging/connector-versions.json", '{"store":"22.0.0","screenshot":"33.0.0"}\n')
        self.assert_changed(set())

    def test_reachable_transitive_lock_change_affects_consumers(self):
        self.replace("Cargo.lock", 'name = "transitive"\nversion = "1.0.0"',
                     'name = "transitive"\nversion = "1.0.1"')
        previous = self.recorded_output(STORE)
        self.assertFalse(release.lock_change_is_ambiguous("store", previous, self.project))
        self.assert_changed({"center", "store"})

    def test_feature_unified_registry_edge_requires_a_selected_release_to_fail_closed(self):
        old = self.lock_item("store-runtime", "1.0.0", ["transitive"], source=REGISTRY)
        new = self.lock_item("store-runtime", "1.0.0", ["transitive", "feature-union"], source=REGISTRY)
        self.replace("Cargo.lock", old, new + self.lock_item("feature-union", "1.0.0", source=REGISTRY))
        self.assertTrue(release.lock_change_is_ambiguous("store", self.recorded_output(STORE), self.project))
        with self.assertRaisesRegex(RuntimeError, "ambiguous feature-unified edge"):
            self.guard("store")
        self.guard("screenshot", unchanged=True)

    def test_local_dependencies_use_only_release_platforms_and_active_optional_features(self):
        manifest = "crates/screenshot-connector/Cargo.toml"
        self.write(manifest, (self.project / manifest).read_text() + '''
[dependencies.feature-helper]
path = "../feature-helper"
optional = true

[target.'cfg(all(unix, target_os = "linux"))'.dependencies]
linux-only = { path = "../linux-only" }
''')
        self.replace("Cargo.lock", self.lock_item("screenshot-connector", "0.2.9", ["store-protocol"]),
                     self.lock_item("screenshot-connector", "0.2.9", ["store-protocol", "feature-helper", "linux-only"])
                     + self.lock_item("feature-helper", "1.0.0") + self.lock_item("linux-only", "1.0.0"))
        for name in ("feature-helper", "linux-only"):
            self.write(f"crates/{name}/Cargo.toml", f'[package]\nname = "{name}"\nversion = "1.0.0"\n')
            self.write(f"crates/{name}/src/lib.rs", "pub fn example() {}\n")
        self.assert_changed(set())
        self.write(manifest, (self.project / manifest).read_text() + '\n[features]\ndefault = ["dep:feature-helper"]\n')
        hashes = release.component_source_hashes("screenshot", self.project)
        self.assertIn("crates/feature-helper/src/lib.rs", hashes)
        self.assertNotIn("crates/linux-only/src/lib.rs", hashes)
        self.assert_changed({"screenshot"})

    def test_new_build_script_and_removed_production_source_change_inputs(self):
        self.write("crates/store-provider/build.rs", "fn main() {}\n")
        self.assert_changed({"center", "store"})
        (self.project / "crates/store-provider/build.rs").unlink()
        (self.project / "crates/store-provider/src/lib.rs").unlink()
        self.assert_changed({"center", "store"})

    def test_cfg_test_stripping_keeps_following_production_items(self):
        path = "crates/store-protocol/src/lib.rs"
        self.write(path, 'pub fn before() {}\npub fn after() {}\n')
        self.commit()
        self.record()
        self.write(path, '''pub fn before() {}
#[allow(dead_code)]
#[cfg(test)]
mod tests { fn check() { let s = "}"; /* { /* } */ } */ } }
#[test]
fn isolated_test() {}
pub fn after() {}
''')
        self.write("crates/store-provider/src/test_helpers.rs", "#[cfg(test)]\nstruct Test([u8; 2]);\n")
        self.write("crates/store-provider/src/test_only.rs", "#![cfg(test)]\npub fn test_helper() {}\n")
        self.assert_changed(set())
        self.replace(path, "pub fn after() {}", "pub fn changed_after() {}")
        self.assert_changed({"center", "store", "screenshot"})

    def test_path_modules_follow_project_files_outside_src(self):
        source = "crates/store-provider/src/lib.rs"
        self.write(source, (self.project / source).read_text() + '#[path = "../../../shared.rs"]\nmod shared;\n')
        self.write("shared.rs", "pub fn shared() {}\n")
        self.commit()
        self.record()
        self.write("shared.rs", "pub fn changed() {}\n")
        self.assert_changed({"center", "store"})

    def test_static_build_script_inputs_are_tracked_and_dynamic_inputs_are_rejected(self):
        script = "crates/store-provider/build.rs"
        asset = "crates/store-provider/build-input.txt"
        self.write(script, 'fn main() { println!("cargo::rerun-if-changed=build-input.txt"); }\n')
        self.write(asset, "first value\n")
        self.commit()
        self.record()
        self.write(asset, "another value\n")
        self.assert_changed({"center", "store"})
        self.write(script, 'fn main() { std::fs::read("dynamic.txt").unwrap(); }\n')
        with self.assertRaisesRegex(RuntimeError, "Cannot scope build script inputs"):
            self.guard("store")
        (self.project / script).unlink()
        self.write("crates/store-provider/src/embedded.rs", 'include!(concat!(env!("OUT_DIR"), "/extra.rs"));\n')
        with self.assertRaisesRegex(RuntimeError, "OUT_DIR"):
            self.guard("store")

    def test_downgrade_and_missing_baseline_are_rejected(self):
        self.set_version("store", "0.2.8", "0.2.7")
        with self.assertRaisesRegex(RuntimeError, "downgrade"):
            self.guard("store")
        with self.assertRaisesRegex(RuntimeError, "Previous screenshot release output"):
            self.guard("screenshot", paths=[self.windows_report])

    def test_invalid_versions_and_mismatched_cargo_lock(self):
        self.assertGreater(compat.version_tuple("0.10.0"), compat.version_tuple("0.9.99"))
        for value in (None, 2, "01.2.0", "0.2.00", "v0.2.0", "0.2.0-rc1", "０.2.0", "4294967296.0.0"):
            with self.subTest(value=value), self.assertRaises(RuntimeError):
                compat.version_tuple(value)
        self.replace(compat.MANIFEST, 'version = "0.2.8"', 'version = "0.2.9"')
        with self.assertRaisesRegex(RuntimeError, "must agree with Cargo.lock"):
            release.component_version("store", self.project)

    def test_retained_outputs_keep_their_own_commit_and_binary_identity(self):
        previous = json.loads(self.base_report.read_text())
        store = previous["outputs"][STORE]
        self.set_version("center", "0.1.1", "0.1.2")
        commit = self.commit()
        report = {"sourceCommit": commit, "outputs": {**previous["outputs"], API: {
            **release.component_metadata("center", self.project), "sourceCommit": commit,
            "sha256": "b" * 64, "bytes": 24,
        }}}
        normalized = release.normalize_report(report, self.project)
        self.assertEqual(normalized["outputs"][STORE], store)
        self.assertEqual(normalized["outputs"][API]["sourceCommit"], commit)
        self.assertNotEqual(store["sourceCommit"], commit)

    def test_modern_reports_cannot_relabel_retained_metadata_as_legacy(self):
        report = json.loads(self.base_report.read_text())
        for key in release.METADATA_KEYS:
            report["outputs"][STORE].pop(key)
        with self.assertRaisesRegex(RuntimeError, "Every output.*own metadata"):
            release.normalize_report(report, self.project)
        report.pop("schemaVersion")
        with self.assertRaisesRegex(RuntimeError, "Every output.*own metadata"):
            release.normalize_report(report, self.project)
        single = json.loads(self.windows_report.read_text())
        for key in release.METADATA_KEYS:
            single["outputs"][WINDOWS].pop(key)
        with self.assertRaisesRegex(RuntimeError, "Every output.*own metadata"):
            release.normalize_report(single, self.project)

    def test_forged_modern_metadata_and_sha_are_rejected(self):
        report = json.loads(self.base_report.read_text())
        output = report["outputs"][STORE]
        for field, value in (("component", "screenshot"), ("version", "0.2.0"),
                             ("sourceFingerprint", "0" * 64), ("sourceCommit", "HEAD"),
                             ("sourceCommit", "0" * 40), ("sha256", "bad")):
            with self.subTest(field=field, value=value):
                forged = copy.deepcopy(report)
                forged["outputs"][STORE][field] = value
                with self.assertRaises(RuntimeError):
                    release.normalize_report(forged, self.project)
        output["sourceHashes"]["Cargo.lock"] = "0" * 64
        output["sourceFingerprint"] = compat._fingerprint(output["sourceHashes"])
        with self.assertRaisesRegex(RuntimeError, "metadata differs from sourceCommit"):
            release.normalize_report(report, self.project)
        self.write(STORE, "build output")
        output["sha256"] = hashlib.sha256(b"build output").hexdigest()
        release.verify_output_binary(STORE, output, self.project)
        self.write(STORE, "other output")
        with self.assertRaisesRegex(RuntimeError, "binary differs"):
            release.verify_output_binary(STORE, output, self.project)

    def legacy_record(self):
        self.replace("Cargo.toml", ', "crates/screenshot-connector"', "")
        self.replace("Cargo.lock", self.lock_item("screenshot-connector", "0.2.9", ["store-protocol"]), "")
        self.replace("crates/store-api/Cargo.toml", 'version = "0.1.1"', "version.workspace = true")
        self.replace("Cargo.lock", 'name = "store-api"\nversion = "0.1.1"',
                     'name = "store-api"\nversion = "0.1.0"')
        (self.project / "crates/screenshot-connector/Cargo.toml").unlink()
        for name, destination in (("main.rs", release.LEGACY_EXAMPLE), ("worker.rs", release.LEGACY_MODULES + "/worker.rs")):
            source = self.project / "crates/screenshot-connector/src" / name
            self.write(destination, source.read_text())
            source.unlink()
        commit = self.commit()
        raw = {path.relative_to(self.project).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
               for path in self.project.rglob("*") if path.is_file()}
        legacy_node_hashes = {"Cargo.lock": raw["Cargo.lock"]}
        report = {"sourceCommit": commit, "sourceHashes": raw,
                  "connectorVersion": "0.2.8", "connectorSourceHashes": legacy_node_hashes,
                  "connectorSourceFingerprint": compat._fingerprint(legacy_node_hashes),
                  "outputs": {path: {"sha256": "b" * 64, "bytes": 12} for path in (API, STORE, SCREENSHOT)}}
        self.base_report.write_text(json.dumps(report))
        return report

    def test_legacy_reports_project_historical_versions_and_keep_store_fingerprint(self):
        current_commit = self.git("rev-parse", "HEAD")
        report = self.legacy_record()
        self.git("checkout", "--detach", "-q", current_commit)
        converted = release.normalize_report(report, self.project)
        self.assertEqual(converted["schemaVersion"], release.REPORT_SCHEMA_VERSION)
        normalized = converted["outputs"]
        self.assertEqual([normalized[path]["version"] for path in (API, STORE, SCREENSHOT)],
                         ["0.1.0", "0.2.8", "0.2.8"])
        self.assertIn(release.LEGACY_EXAMPLE, normalized[SCREENSHOT]["sourceHashes"])
        self.assertNotIn(release.LEGACY_EXAMPLE, normalized[STORE]["sourceHashes"])
        self.assertEqual(normalized[STORE]["sourceCommit"], report["sourceCommit"])
        self.assertEqual(normalized[STORE]["sha256"], report["outputs"][STORE]["sha256"])
        self.guard("store", unchanged=True)
        self.assertEqual(self.guard("screenshot")["version"], "0.2.9")

    def test_legacy_unknown_and_mismatched_production_bytes_are_rejected(self):
        report = self.legacy_record()
        baseline = report["sourceHashes"]
        for hashes in ({**baseline, compat.MANIFEST: "0" * 64},
                       {path: digest for path, digest in baseline.items() if path != compat.MANIFEST},
                       {**baseline, "crates/store-connector/src/uncommitted.rs": "0" * 64}):
            with self.subTest(hashes=hashes), self.assertRaisesRegex(RuntimeError, "source hash|Missing release source"):
                release.normalize_report({**report, "sourceHashes": hashes}, self.project)

    def recorded_output(self, relative):
        return json.loads(self.base_report.read_text())["outputs"][relative]


class PlatformScreenshotRolloutTests(unittest.TestCase):
    def test_rollout_retains_both_initial_choices_and_existing_splits(self):
        from deploy_setup import expected_platform_screenshots

        base = {"migrationMarkers": [], "platformScreenshotBindings": [],
                "accountScreenshotBindings": [], "connectors": [["store", "auto", 1]],
                "accounts": [["ios-id", "name", "ios", "store", "mac", 1, None]]}
        self.assertEqual(expected_platform_screenshots(base),
                         [["store", "android", "mac", 0], ["store", "ios", "mac", 0]])
        base["accountScreenshotBindings"] = [["store", None]]
        self.assertEqual(expected_platform_screenshots(base),
                         [["store", "android", None, 1], ["store", "ios", None, 1]])
        base["migrationMarkers"] = [["platform_screenshots_v1"]]
        base["platformScreenshotBindings"] = [["store", "android", None, 1], ["store", "ios", "other", 1]]
        self.assertEqual(expected_platform_screenshots(base), base["platformScreenshotBindings"])


if __name__ == "__main__":
    managed_root()
    unittest.main()
