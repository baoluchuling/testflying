"""Synthetic release regression fixtures; executed only inside artifact-run."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import connector_release as release
from managed_workspace import managed_root


class ConnectorReleaseTests(unittest.TestCase):
    def setUp(self):
        temporary = managed_root() / "tmp"
        temporary.mkdir(exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(prefix="release-fixture-", dir=temporary)
        self.addCleanup(self.directory.cleanup)
        self.project = Path(self.directory.name)
        self.write("Cargo.toml", '[workspace.package]\nversion = "0.1.0"\n')
        self.set_version("0.2.0")
        self.write("crates/store-protocol/src/lib.rs", "pub fn production() {}\n")
        self.write("crates/store-provider/src/lib.rs", "pub fn provider() {}\n")
        self.write("crates/store-connector/src/lib.rs", "pub fn connector() {}\n")
        self.write("crates/store-connector/examples/screenshot-connector.rs", "fn main() {}\n")
        self.write("crates/store-api/src/lib.rs", 'fn config() {}\nasync fn installer() {}\nfn business() {}\n')
        self.write("crates/store-api/src/screenshots.rs", 'fn screenshot_installer() {}\nfn installer_archive() {}\n')
        self.write("packaging/connector/install.sh", "#!/bin/sh\nexit 0\n")
        self.write("packaging/screenshot-connector/run.sh", "#!/bin/sh\nexit 0\n")
        self.report = self.project / "previous.json"
        self.record()

    def write(self, path: str, content: str):
        destination = self.project / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content)

    def set_version(self, version: str):
        self.write(release.MANIFEST, '[package]\nname = "store-connector"\nversion = "' + version + '"\n')
        self.write("Cargo.lock", '[[package]]\nname = "store-connector"\nversion = "' + version + '"\n')
        self.write(release.RELEASE, 'pub const CONNECTOR_VERSION: &str = "' + version + '";\n')

    def record(self, path: Path | None = None):
        hashes = release.connector_source_hashes(self.project)
        (path or self.report).write_text(json.dumps({
            "connectorVersion": release.connector_version(self.project),
            "connectorSourceFingerprint": release.connector_source_fingerprint(self.project),
            "connectorSourceHashes": hashes,
        }))

    def guard(self, *, api_only=False, paths=None):
        return release.guard_connector_release(paths or [self.report], api_only=api_only, project=self.project)

    def test_repeat_same_version_same_inputs_and_api_only(self):
        self.assertEqual(self.guard()["connectorVersion"], "0.2.0")
        self.assertEqual(self.guard(api_only=True)["connectorVersion"], "0.2.0")

    def test_changed_production_requires_higher_version(self):
        self.write("crates/store-provider/src/lib.rs", "pub fn changed_provider() {}\n")
        with self.assertRaisesRegex(RuntimeError, "increase Connector version"):
            self.guard()
        self.set_version("0.2.1")
        self.assertEqual(self.guard()["connectorVersion"], "0.2.1")
        with self.assertRaisesRegex(RuntimeError, "rebuild all"):
            self.guard(api_only=True)

    def test_downgrade_always_rejected_even_with_same_fingerprint(self):
        report = json.loads(self.report.read_text())
        report["connectorVersion"] = "0.3.0"
        self.report.write_text(json.dumps(report))
        with self.assertRaisesRegex(RuntimeError, "downgrade"):
            self.guard()

    def test_numeric_semver_and_invalid_versions(self):
        self.assertGreater(release.version_tuple("0.10.0"), release.version_tuple("0.9.99"))
        for value in (None, 2, "01.2.0", "0.02.0", "0.2.00", "v0.2.0", "0.2.0-rc1",
                      "0.2.0\n", "０.2.0", "9" * 30 + ".0.0", "4294967296.0.0"):
            with self.subTest(version=value), self.assertRaises(RuntimeError):
                release.version_tuple(value)

    def test_cargo_protocol_and_lock_must_agree(self):
        self.write(release.RELEASE, 'pub const CONNECTOR_VERSION: &str = "0.1.0";')
        with self.assertRaisesRegex(RuntimeError, "Cargo and protocol"):
            self.guard()
        self.set_version("0.2.0")
        self.write("Cargo.lock", '[[package]]\nname = "store-connector"\nversion = "0.1.0"\n')
        with self.assertRaisesRegex(RuntimeError, "Cargo.lock"):
            self.guard()

    def test_center_ui_docs_and_tests_do_not_change_node_inputs(self):
        self.write("crates/store-api/src/lib.rs", 'fn config() {}\nasync fn installer() {}\nfn business_changed() {}\n')
        for path in ("crates/store-api/src/setup/app.mjs", "docs/store-api.md",
                     "crates/store-connector/tests/new_test.rs"):
            self.write(path, "fixture update\n")
        self.assertEqual(self.guard(api_only=True)["connectorVersion"], "0.2.0")

    def test_inline_tests_are_excluded_without_omitting_following_production(self):
        path = "crates/store-protocol/src/lib.rs"
        self.write(path, 'pub fn before() {}\n#[cfg(test)]\nmod tests { fn example() { let x = "}"; } }\npub fn after() {}\n')
        self.record()
        self.write(path, 'pub fn before() {}\n#[cfg(test)]\nmod tests { fn updated() { /* { /* } */ } */ } }\npub fn after() {}\n')
        self.guard(api_only=True)
        self.write(path, 'pub fn before() {}\n#[cfg(test)]\nmod tests {}\npub fn changed_after() {}\n')
        with self.assertRaisesRegex(RuntimeError, "increase Connector version"):
            self.guard()

    def test_screenshot_packaging_and_api_packaging_are_inputs(self):
        for path, value in (
            ("packaging/connector/install.sh", "changed"),
            ("packaging/screenshot-connector/run.sh", "changed"),
            ("crates/store-connector/examples/screenshot-connector.rs", "fn main() { panic!(); }"),
            ("crates/store-api/src/lib.rs", "fn config() { changed(); }\nasync fn installer() {}"),
        ):
            with self.subTest(path=path):
                old = (self.project / path).read_text()
                self.write(path, value)
                with self.assertRaisesRegex(RuntimeError, "increase Connector version"):
                    self.guard()
                self.write(path, old)

    def test_new_and_deleted_production_files_change_fingerprint(self):
        self.write("crates/store-provider/src/new.rs", "pub fn added() {}")
        with self.assertRaisesRegex(RuntimeError, "increase Connector version"):
            self.guard()
        self.record()
        (self.project / "crates/store-provider/src/new.rs").unlink()
        with self.assertRaisesRegex(RuntimeError, "increase Connector version"):
            self.guard()

    def test_each_platform_uses_its_own_previous_release(self):
        windows = self.project / "windows.json"
        self.record(windows)
        report = json.loads(windows.read_text())
        report["connectorVersion"] = "0.3.0"
        windows.write_text(json.dumps(report))
        self.guard(paths=[self.report])
        with self.assertRaisesRegex(RuntimeError, "downgrade"):
            self.guard(paths=[windows])
        with self.assertRaisesRegex(RuntimeError, "downgrade"):
            self.guard(paths=[self.report, windows], api_only=True)

    def test_missing_or_inconsistent_previous_metadata_fails_closed(self):
        report = json.loads(self.report.read_text())
        report["connectorSourceFingerprint"] = "0" * 64
        self.report.write_text(json.dumps(report))
        with self.assertRaisesRegex(RuntimeError, "fingerprint"):
            self.guard()
        with self.assertRaises(RuntimeError):
            release.guard_connector_release([], project=self.project)

    def legacy(self, *, inherited: bool):
        files = {path.relative_to(self.project).as_posix(): path.read_bytes()
                 for path in self.project.rglob("*") if path.is_file() and path != self.report}
        files[release.MANIFEST] = ('[package]\nname = "store-connector"\n' + (
            'version.workspace = true\n' if inherited else 'version = "0.1.0"\n')).encode()
        report = {"sourceCommit": "a" * 40, "sourceHashes": {
            path: hashlib.sha256(data).hexdigest() for path, data in files.items()}}
        self.report.write_text(json.dumps(report))

        def historical(project, commit, path):
            self.assertEqual(project, self.project)
            self.assertEqual(commit, "a" * 40)
            return files[path]

        return patch.object(release, "_git_source", side_effect=historical)

    def test_legacy_version_comes_from_report_commit_with_workspace_inheritance(self):
        for inherited in (False, True):
            with self.subTest(inherited=inherited), self.legacy(inherited=inherited):
                self.assertEqual(self.guard()["connectorVersion"], "0.2.0")
                with self.assertRaisesRegex(RuntimeError, "rebuild all"):
                    self.guard(api_only=True)

    def test_legacy_manifest_hash_mismatch_is_rejected(self):
        with self.legacy(inherited=True):
            report = json.loads(self.report.read_text())
            report["sourceHashes"]["Cargo.toml"] = "0" * 64
            self.report.write_text(json.dumps(report))
            with self.assertRaisesRegex(RuntimeError, "Historical node manifest"):
                self.guard()

    def test_historical_git_reads_exact_commit_and_project_prefix(self):
        with patch.object(release.subprocess, "check_output", side_effect=["store-service/\n", b"old source"]) as command:
            self.assertEqual(release._git_source(self.project, "a" * 40, release.MANIFEST), b"old source")
            self.assertEqual(command.call_args.args[0], ["git", "show", "a" * 40 + ":store-service/" + release.MANIFEST])
        with self.assertRaisesRegex(RuntimeError, "exact sourceCommit"):
            release._git_source(self.project, "HEAD", release.MANIFEST)


if __name__ == "__main__":
    managed_root()
    unittest.main()
