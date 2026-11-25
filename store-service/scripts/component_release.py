"""Version and verify the production inputs of each formal release component."""
from __future__ import annotations

import ast
import hashlib
import json
import posixpath
import re
import subprocess
import tomllib
from pathlib import Path

from connector_release import _fingerprint, _item_end, _production_rust, _rust_mask, version_tuple
from managed_workspace import PROJECT


COMPONENT_PACKAGES = {
    "center": "store-api",
    "store": "store-connector",
    "screenshot": "screenshot-connector",
}
BUILD_HOST = "aarch64-apple-darwin"
COMPONENT_TARGETS = {
    "center": ("x86_64-unknown-linux-musl",),
    "store": ("x86_64-unknown-linux-musl", BUILD_HOST, "x86_64-pc-windows-gnu"),
    "screenshot": (BUILD_HOST,),
}
TARGET_FACTS = {
    "x86_64-unknown-linux-musl": ("linux", "unix", "musl"),
    BUILD_HOST: ("macos", "unix", ""),
    "x86_64-pc-windows-gnu": ("windows", "windows", "gnu"),
}
OUTPUT_COMPONENTS = {
    "releases/linux-amd64/store-api": "center",
    "releases/linux-amd64/store-connector": "store",
    "releases/darwin-arm64/store-connector": "store",
    "releases/windows-amd64/store-connector.exe": "store",
    "releases/darwin-arm64/screenshot-connector": "screenshot",
}
STORE_MANIFEST = "crates/store-connector/Cargo.toml"
LEGACY_EXAMPLE = "crates/store-connector/examples/screenshot-connector.rs"
LEGACY_MODULES = "crates/store-connector/examples/screenshot"
LEGACY_SCREENSHOT_DEPS = frozenset({
    "futures-util", "hex", "libc", "md5", "reqwest", "serde", "serde_json",
    "sha2", "store-protocol", "tempfile", "tokio", "tokio-tungstenite", "uuid",
})
API_LIB = "crates/store-api/src/lib.rs"
API_SCREENSHOTS = "crates/store-api/src/screenshots.rs"
API_SETUP_APP = "crates/store-api/src/setup/app.mjs"
STORE_PACKAGER = "crates/store-api/src/setup/packager.mjs"
STORE_ZIP_LIBRARY = "crates/store-api/src/setup/vendor/jszip.min.js"
CA_CERT = "deploy/testflying-prod-ca.crt"
PRODUCTION_DEPENDENCIES = ("dependencies", "build-dependencies")
METADATA_KEYS = ("component", "version", "sourceFingerprint", "sourceHashes", "sourceCommit")
REPORT_SCHEMA_VERSION = 2
COMMIT = re.compile(r"[a-f0-9]{40}")
SHA256 = re.compile(r"[a-f0-9]{64}")
LOCK_REFERENCE = re.compile(r"([^ ]+)(?: ([^ ]+)(?: \((.+)\))?)?")
INCLUDE = re.compile(r"\binclude(?:_bytes|_str)?\s*!\s*\(")
LITERAL_INCLUDE = re.compile(r'\s*"([^"\\]+)"\s*,?\s*\)')
PATH_ATTRIBUTE = re.compile(r"#\s*\[[^\]]*?\bpath\s*=")
LITERAL_PATH_MODULE = re.compile(r'\s*"([^"\\]+)"\s*\]')
BUILD_PRINT = re.compile(r"\bprintln\s*!\s*\(\s*\)\s*;")
BUILD_INSTRUCTION = re.compile(r'println\s*!\s*\(\s*"cargo::?([a-z-]+)=([^"\\]*)"\s*\)\s*;')
CONFIG_BRANCH = re.compile(r"\bif\s+node\s*\.\s*kind\s*==\s*ConnectorKind\s*::\s*Screenshot\s*\{")


def _digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _encoded(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def _relative(path: str) -> str:
    relative = posixpath.normpath(path)
    if relative in (".", "..") or relative.startswith(("/", "../")):
        raise RuntimeError("Release source must stay inside its project: " + path)
    return relative


def _manifest_path(kind: str) -> str:
    if kind not in COMPONENT_PACKAGES:
        raise RuntimeError("Unknown release component: " + repr(kind))
    return f"crates/{COMPONENT_PACKAGES[kind]}/Cargo.toml"


def _exact_commit(value: object) -> str:
    if not isinstance(value, str) or COMMIT.fullmatch(value) is None:
        raise RuntimeError("Release output requires an exact sourceCommit")
    return value


def _valid_sha(value: object) -> bool:
    return isinstance(value, str) and SHA256.fullmatch(value) is not None


def _checked_hashes(value: object) -> dict[str, str]:
    if not isinstance(value, dict) or not value or any(
        not isinstance(path, str) or not path or _relative(path) != path or not _valid_sha(digest)
        for path, digest in value.items()
    ):
        raise RuntimeError("Invalid release source hashes")
    return value


def _rust_item(source: str, mask: str, item: str, name: str) -> str:
    matches = list(re.finditer(r"\b" + item + r"\s+" + name + r"\b", mask))
    if len(matches) != 1:
        raise RuntimeError("Cannot identify node packaging item: " + name)
    start = matches[0].start()
    return source[start:_item_end(mask, start)]


def _config_branch(source: str, mask: str, kind: str) -> str:
    config = _rust_item(source, mask, "fn", "config")
    config_mask = _rust_mask(config)
    matches = list(CONFIG_BRANCH.finditer(config_mask))
    if len(matches) != 1:
        raise RuntimeError("Cannot identify Store and Screenshot installation config branches")
    branch = matches[0]
    end = _item_end(config_mask, branch.end() - 1)
    if not re.match(r"\s*return\s+json\s*!", config_mask[branch.end():end]):
        raise RuntimeError("Cannot identify Screenshot installation config result")
    if not re.fullmatch(r"\s*json\s*!\s*\(.*\)\s*\}\s*", config_mask[end:], re.DOTALL):
        raise RuntimeError("Cannot identify Store installation config result")
    shared = config[:branch.end()]
    if kind == "screenshot":
        return shared + config[branch.end():end]
    return shared + "}" + config[end:]


def _js_block(source: str, start: str, end: str) -> str:
    expression = re.compile(r"^" + re.escape(start) + r".*?^" + re.escape(end) + r"[ \t]*$",
                            re.MULTILINE | re.DOTALL)
    matches = list(expression.finditer(source))
    if len(matches) != 1:
        raise RuntimeError("Cannot identify node installer code: " + start)
    return matches[0][0]


def _matches_target(selector: str, target: str) -> bool:
    if not selector.startswith("cfg("):
        return selector == target
    arch, vendor, *_ = target.split("-")
    system, family, environment = TARGET_FACTS[target]
    facts = {"target_arch": arch, "target_vendor": vendor, "target_os": system,
             "target_family": family, "target_env": environment, "target_abi": "",
             "target_endian": "little", "target_pointer_width": "64"}

    def evaluate(node: ast.AST) -> bool:
        if isinstance(node, ast.Name) and node.id in {"unix", "windows"}:
            return node.id == family
        if (isinstance(node, ast.Compare) and isinstance(node.left, ast.Name)
                and node.left.id in facts and len(node.ops) == 1 and isinstance(node.ops[0], ast.Eq)
                and isinstance(node.comparators[0], ast.Constant)):
            return facts[node.left.id] == node.comparators[0].value
        if (isinstance(node, ast.Call) and isinstance(node.func, ast.Name)
                and node.func.id in {"all", "any"} and not node.keywords):
            values = [evaluate(arg) for arg in node.args]
            return all(values) if node.func.id == "all" else any(values)
        if isinstance(node, ast.UnaryOp) and isinstance(node.op, ast.Not):
            return not evaluate(node.operand)
        raise RuntimeError("Cannot scope unsupported Cargo platform selector: " + selector)

    expression = re.sub(r'\bnot\s*\(', 'not (', selector[4:-1]).replace("=", "==")
    try:
        return evaluate(ast.parse(expression.strip(), mode="eval").body)
    except SyntaxError as error:
        raise RuntimeError("Cannot scope invalid Cargo platform selector: " + selector) from error


def _expand_features(definitions: dict[str, list[str]], requested: set[str]) -> set[str]:
    active = set(requested)
    pending = list(active)
    while pending:
        additions = set(definitions.get(pending.pop(), ())) - active
        active.update(additions)
        pending.extend(additions)
    return active


def _active_dependency(alias: str, options: dict, features: set[str], definitions: dict[str, list[str]]) -> bool:
    if not options.get("optional") or f"dep:{alias}" in features:
        return True
    if any(feature.startswith(alias + "/") for feature in features):
        return True
    has_implicit_feature = not any(f"dep:{alias}" in members for members in definitions.values())
    return alias in features and has_implicit_feature


def _forwarded_features(alias: str, features: set[str]) -> set[str]:
    return {feature for reference in features if "/" in reference
            for name, feature in [reference.split("/", 1)] if name.rstrip("?") == alias}


class _Sources:
    def __init__(self, project: Path, commit: str | None = None):
        self.project = project
        self.commit = commit
        self.blobs: dict[str, str] = {}
        self.contents: dict[str, bytes] = {}
        if commit is not None:
            if self._git("cat-file", "-t", commit) != b"commit\n":
                raise RuntimeError("Release sourceCommit must identify a Git commit")
            prefix = self._git("rev-parse", "--show-prefix").decode().strip()
            pathspec = (prefix,) if prefix else ()
            tree = self._git("ls-tree", "-r", "--full-tree", "-z", commit, "--", *pathspec)
            for record in tree.split(b"\0"):
                if not record:
                    continue
                header, path = record.decode().split("\t", 1)
                mode, kind, oid = header.split()
                if kind == "blob" and mode in {"100644", "100755"}:
                    self.blobs[path.removeprefix(prefix)] = oid

    def _git(self, *args: str, data: bytes | None = None) -> bytes:
        try:
            return subprocess.check_output(["git", "--no-replace-objects", *args], cwd=self.project, input=data,
                                           stderr=subprocess.DEVNULL)
        except subprocess.CalledProcessError as error:
            raise RuntimeError(f"Cannot recover release sourceCommit {self.commit}") from error

    def files(self, directory: str) -> list[str]:
        if self.commit is not None:
            return sorted(path for path in self.blobs if path.startswith(directory + "/"))
        return sorted(path.relative_to(self.project).as_posix()
                      for path in (self.project / directory).rglob("*") if path.is_file())

    def has(self, path: str) -> bool:
        if self.commit is not None:
            return path in self.blobs
        return (self.project / path).is_file()

    def load(self, paths) -> None:
        if self.commit is None:
            return
        missing = sorted(set(paths) - self.contents.keys())
        if not missing:
            return
        try:
            request = "".join(self.blobs[path] + "\n" for path in missing).encode()
        except KeyError as error:
            raise RuntimeError(f"Missing release source {error.args[0]} at {self.commit}") from error
        response = self._git("cat-file", "--batch", data=request)
        cursor = 0
        for path in missing:
            header = response.find(b"\n", cursor)
            fields = response[cursor:header].decode().split()
            if header < 0 or len(fields) != 3 or fields[:2] != [self.blobs[path], "blob"]:
                raise RuntimeError(f"Invalid Git blob for {path} at {self.commit}")
            end = header + 1 + int(fields[2])
            if response[end:end + 1] != b"\n":
                raise RuntimeError(f"Incomplete Git blob for {path} at {self.commit}")
            self.contents[path] = response[header + 1:end]
            cursor = end + 1

    def read(self, path: str) -> bytes:
        if self.commit is not None:
            self.load((path,))
        elif path not in self.contents:
            source = self.project / path
            if not source.is_file() or source.is_symlink():
                raise RuntimeError("Missing regular release source: " + path)
            self.contents[path] = source.read_bytes()
        return self.contents[path]


class _Inputs:
    def __init__(self, sources: _Sources, kind: str, recorded: dict[str, str] | None = None):
        self.sources = sources
        self.recorded = recorded
        self.kind = kind
        self.root = _manifest_path(kind)
        self.legacy_example = (kind == "screenshot" and sources.commit is not None
                               and not self.has(self.root))
        if self.legacy_example:
            if not sources.has(LEGACY_EXAMPLE):
                raise RuntimeError("Historical release has no Screenshot Connector source")
            self.root = STORE_MANIFEST
        self.hashes: dict[str, str] = {}
        sources.load(("Cargo.toml", "Cargo.lock", self.root))
        self.workspace = self.toml("Cargo.toml")
        self.lock = _CargoLock(self.toml("Cargo.lock"))
        self.manifests: dict[str, dict] = {}
        self.local: dict[tuple[str, str, str], str] = {}
        self.edges: dict[tuple[str, str, str], set[tuple[str, str, str]]] = {}
        self.local_requests: dict[tuple[str, str], set[str]] = {}
        self.local_projections: dict[str, dict[str, dict]] = {}

    def read(self, path: str) -> bytes:
        data = self.sources.read(path)
        if self.recorded is not None and self.recorded.get(path) != _digest(data):
            raise RuntimeError("Historical release source hash differs from sourceCommit: " + path)
        return data

    def has(self, path: str) -> bool:
        return self.sources.has(path) or (self.recorded is not None and path in self.recorded)

    def files(self, directory: str) -> set[str]:
        paths = set(self.sources.files(directory))
        if self.recorded is not None:
            paths.update(path for path in self.recorded if path.startswith(directory + "/"))
        return paths

    def toml(self, path: str) -> dict:
        return tomllib.loads(self.read(path).decode())

    def manifest(self, path: str) -> dict:
        if path not in self.manifests:
            self.manifests[path] = self.toml(path)
        return self.manifests[path]

    def package(self, path: str) -> dict:
        inherited = self.workspace.get("workspace", {}).get("package", {})
        return {key: inherited[key] if value == {"workspace": True} else value
                for key, value in self.manifest(path)["package"].items() if key != "metadata"}

    def locked_package(self, path: str) -> tuple[str, str, str]:
        package = self.package(path)
        key = (package["name"], package["version"], "")
        if key not in self.lock.packages:
            raise RuntimeError(f"{path} package version must agree with Cargo.lock")
        return key

    def version(self) -> str:
        key = self.locked_package(self.root)
        expected = "store-connector" if self.legacy_example else COMPONENT_PACKAGES[self.kind]
        if key[0] != expected:
            raise RuntimeError(f"Expected {expected} in {self.root}")
        version_tuple(key[1])
        if [candidate for candidate in self.lock.by_name[expected] if not candidate[2]] != [key]:
            raise RuntimeError(f"Ambiguous {expected} package version in Cargo.lock")
        return key[1]

    def dependency(self, alias: str, options: dict | str, parent: str) -> dict:
        options = {"version": options} if isinstance(options, str) else dict(options)
        if options.pop("workspace", False):
            base = self.workspace["workspace"]["dependencies"][alias]
            base = {"version": base} if isinstance(base, str) else base
            features = sorted(set(base.get("features", ())) | set(options.get("features", ())))
            options = {**base, **options}
            if features:
                options["features"] = features
            parent = ""
        if "path" in options:
            options["path"] = _relative(posixpath.join(parent, options["path"]))
        if "features" in options:
            options["features"] = sorted(set(options["features"]))
        return options

    def dependency_tables(self, path: str, target: str) -> dict[str, dict]:
        manifest = self.manifest(path)
        parent = posixpath.dirname(path)
        tables = {key: manifest[key] for key in PRODUCTION_DEPENDENCIES if key in manifest}
        if self.legacy_example and path == self.root:
            # Cargo builds the historical example against its package's normal and dev deps.
            # Capture the imports used by that executable, including example-only dependencies.
            direct = dict(tables.get("dependencies", {}))
            for name, options in manifest.get("dev-dependencies", {}).items():
                if name in LEGACY_SCREENSHOT_DEPS and name not in direct:
                    direct[name] = options
            tables["dependencies"] = {name: value for name, value in direct.items()
                                      if name in LEGACY_SCREENSHOT_DEPS}
        result = {key: {name: self.dependency(name, value, parent) for name, value in table.items()}
                  for key, table in tables.items()}
        for selector, config in manifest.get("target", {}).items():
            if not any(config.get(key) for key in PRODUCTION_DEPENDENCIES) or not _matches_target(selector, target):
                continue
            for key in PRODUCTION_DEPENDENCIES:
                if config.get(key):
                    result[f"target.{selector}.{key}"] = {
                        name: self.dependency(name, value, parent) for name, value in config[key].items()}
        return result

    def add_rust(self, path: str, production: bytes | None = None) -> None:
        if path in self.hashes:
            return
        if production is None:
            production = _production_rust(self.read(path))
        if not production:
            return
        self.hashes[path] = _digest(production)
        source = production.decode()
        mask = _rust_mask(source)
        for macro in INCLUDE.finditer(mask):
            literal = LITERAL_INCLUDE.match(source, macro.end())
            if literal is None:
                raise RuntimeError("Cannot scope a non-literal Rust include (including OUT_DIR): " + path)
            included = _relative(posixpath.join(posixpath.dirname(path), literal[1]))
            if macro[0].startswith("include_"):
                self.hashes[included] = _digest(self.read(included))
            else:
                self.add_rust(included)
        for attribute in PATH_ATTRIBUTE.finditer(mask):
            literal = LITERAL_PATH_MODULE.match(source, attribute.end())
            prefix = mask[:attribute.start()]
            if literal is None or prefix.count("{") != prefix.count("}"):
                raise RuntimeError("Cannot scope a nested or non-literal Rust path module: " + path)
            self.add_rust(_relative(posixpath.join(posixpath.dirname(path), literal[1])))

    def add_build_script(self, path: str, package_directory: str) -> None:
        production = _production_rust(self.read(path))
        source = production.decode()
        mask = _rust_mask(source)
        if not re.fullmatch(r"\s*fn\s+main\s*\(\s*\)\s*\{\s*\}\s*", BUILD_PRINT.sub("", mask)):
            raise RuntimeError("Cannot scope build script inputs without static Cargo directives: " + path)
        for call in BUILD_PRINT.finditer(mask):
            instruction = BUILD_INSTRUCTION.fullmatch(source[call.start():call.end()])
            if instruction is None:
                raise RuntimeError("Cannot scope dynamic build script input: " + path)
            key, value = instruction.groups()
            if key == "rerun-if-changed":
                input_path = _relative(posixpath.join(package_directory, value))
                files = [input_path] if self.has(input_path) else sorted(self.files(input_path))
                if not files:
                    raise RuntimeError("Missing build script input: " + input_path)
                for file in files:
                    self.hashes[file] = _digest(self.read(file))
            elif key not in {"rustc-cfg", "rustc-check-cfg", "rustc-env", "warning"}:
                raise RuntimeError(f"Cannot scope build script directive {key}: {path}")
        self.hashes[path] = _digest(production)

    def add_sources(self, path: str) -> None:
        manifest = self.manifest(path)
        parent = posixpath.dirname(path)
        package = self.package(path)
        if self.legacy_example and path == self.root:
            files = {LEGACY_EXAMPLE, *self.files(LEGACY_MODULES)}
        else:
            files = self.files(f"{parent}/src")
        rust = {file for file in files if file.endswith(".rs")}
        if not (self.legacy_example and path == self.root):
            for target in [manifest.get("lib", {}), *manifest.get("bin", [])]:
                if "path" in target:
                    rust.add(_relative(posixpath.join(parent, target["path"])))
        build_script = package.get("build", "build.rs")
        if build_script is not False:
            build_path = _relative(posixpath.join(parent, build_script))
            if "build" in package or self.has(build_path):
                rust.discard(build_path)
                self.add_build_script(build_path, parent)
        self.sources.load(rust)
        for file in sorted(rust):
            self.add_rust(file)

    def add_local(self, path: str, target: str, requested: set[str]) -> tuple[str, str, str]:
        key = self.locked_package(path)
        previous_path = self.local.get(key)
        if previous_path is not None and previous_path != path:
            raise RuntimeError(f"Ambiguous Cargo package paths {previous_path} and {path}")
        request_key = (path, target)
        already_requested = self.local_requests.get(request_key)
        if already_requested is not None and requested <= already_requested:
            return key
        self.local[key] = path
        requested = requested | (already_requested or set())
        self.local_requests[request_key] = requested
        manifest = self.manifest(path)
        definitions = manifest.get("features", {})
        active = _expand_features(definitions, requested)
        dependencies = self.dependency_tables(path, target)
        projected = {}
        edges = self.edges.setdefault(key, set())
        for table_name, table in dependencies.items():
            package_target = BUILD_HOST if table_name.endswith("build-dependencies") else target
            selected = {}
            for name, options in table.items():
                if not _active_dependency(name, options, active, definitions):
                    continue
                package = options.get("package", name)
                features = set(options.get("features", ())) | _forwarded_features(name, active)
                if options.get("default-features", True):
                    features.add("default")
                selected[name] = {**options, "features": sorted(features)}
                if "path" in options:
                    child = self.add_local(options["path"] + "/Cargo.toml", package_target, features)
                    if child[0] != package or child not in self.lock.dependencies(key):
                        raise RuntimeError(f"{path} dependency {name} does not agree with Cargo.lock")
                else:
                    child = self.lock.resolve(key, package, options)
                    self.add_external(child)
                edges.add(child)
            if selected:
                projected[table_name] = selected
        self.local_projections.setdefault(path, {})[target] = {
            "features": {feature: definitions[feature] for feature in definitions.keys() & active},
            **projected,
        }
        return key

    def add_external(self, key: tuple[str, str, str]) -> None:
        if key in self.edges:
            return
        if not key[2]:
            raise RuntimeError("Cargo.lock local dependency needs a production path: " + key[0])
        self.edges[key] = self.lock.dependencies(key)
        for child in self.edges[key]:
            self.add_external(child)

    def add_packaging(self) -> None:
        if self.kind == "center":
            return
        files = [CA_CERT]
        items = {API_LIB: [("fn", "installation_config")]}
        if self.kind == "store":
            files.extend(("examples/connector.json", "examples/connector-google.json",
                          STORE_PACKAGER, STORE_ZIP_LIBRARY))
            files.extend(self.files("packaging/connector"))
            items[API_LIB].append(("fn", "installer"))
        else:
            files.extend(self.files("packaging/screenshot-connector"))
            items[API_SCREENSHOTS] = [("fn", name) for name in
                                      ("screenshot_installer", "installer_archive", "release_unavailable")]
            items[API_SCREENSHOTS].append(("const", "ARM64_MACHO_HEADER"))
        self.sources.load([*items, *(path for path in files if self.has(path))])
        for path in files:
            if self.has(path):
                self.hashes[path] = _digest(self.read(path))
        for path, names in items.items():
            source = self.read(path).decode()
            mask = _rust_mask(source)
            scoped = [_rust_item(source, mask, item, name) for item, name in names]
            if path == API_LIB:
                scoped.append(_config_branch(source, mask, self.kind))
            self.add_rust(path, "\n".join(scoped).encode())
        if self.has(API_SETUP_APP):
            app = self.read(API_SETUP_APP).decode()
            blocks = [_js_block(app, "function downloadFile(", "}")]
            if self.kind == "store":
                blocks.append(_js_block(app, '$("setupForm").addEventListener("submit",', "});"))
            else:
                blocks.extend(_js_block(app, expression, "});") for expression in (
                    '$("screenshotProvision").addEventListener("submit",',
                    '$("screenshotDownload").addEventListener("click",',
                ))
                if self.has(STORE_PACKAGER):
                    packager = self.read(STORE_PACKAGER).decode()
                    self.hashes[STORE_PACKAGER] = _digest(
                        _js_block(packager, "export function archiveNodeName(", "}").encode())
            self.hashes[API_SETUP_APP] = _digest("\n".join(blocks).encode())

    def metadata(self) -> dict:
        version = self.version()
        for target in COMPONENT_TARGETS[self.kind]:
            self.add_local(self.root, target, {"default"})
        for path, per_target in self.local_projections.items():
            manifest = self.manifest(path)
            compiled = {key: manifest[key] for key in ("lib", "bin", "lints") if key in manifest}
            if compiled.get("lints") == {"workspace": True}:
                compiled["lints"] = self.workspace["workspace"]["lints"]
            self.hashes[path] = _digest(_encoded({**compiled, "package": self.package(path), "targets": per_target}))
            self.add_sources(path)
        self.add_packaging()
        self.hashes["Cargo.lock"] = _digest(_encoded([
            {**self.lock.packages[key], "dependencies": sorted(self.edges[key])}
            for key in sorted(self.edges)
        ]))
        release = dict(self.workspace.get("profile", {}).get("release", {}))
        if "package" in release:
            names = {name for name, _, _ in self.edges}
            local_overrides = {name: config for name, config in release.pop("package").items()
                               if name == "*" or name.split(":", 1)[0] in names}
            if local_overrides:
                release["package"] = local_overrides
        resolver = self.workspace.get("workspace", {}).get("resolver")
        self.hashes["Cargo.toml"] = _digest(_encoded({"resolver": resolver, "release": release}))
        for path in (".cargo/config", ".cargo/config.toml", "rust-toolchain", "rust-toolchain.toml"):
            if self.has(path):
                self.hashes[path] = _digest(self.read(path))
        return {"component": self.kind, "version": version,
                "sourceFingerprint": _fingerprint(self.hashes), "sourceHashes": dict(sorted(self.hashes.items()))}


class _CargoLock:
    def __init__(self, lock: dict):
        self.packages = {}
        self.by_name = {}
        self.references = {}
        self.resolved = {}
        for package in lock["package"]:
            key = (package["name"], package["version"], package.get("source", ""))
            if key in self.packages:
                raise RuntimeError("Duplicate package in Cargo.lock: " + repr(key))
            self.packages[key] = {name: value for name, value in package.items() if name != "dependencies"}
            self.by_name.setdefault(key[0], []).append(key)
            self.references[key] = package.get("dependencies", ())

    def dependencies(self, key: tuple[str, str, str]) -> set[tuple[str, str, str]]:
        if key in self.resolved:
            return self.resolved[key]
        result = set()
        for reference in self.references[key]:
            match = LOCK_REFERENCE.fullmatch(reference)
            if match is None:
                raise RuntimeError("Invalid Cargo.lock dependency: " + reference)
            name, version, source = match.groups()
            matches = [candidate for candidate in self.by_name.get(name, ())
                       if (version is None or candidate[1] == version)
                       and (source is None or candidate[2] == source)]
            if len(matches) != 1:
                raise RuntimeError("Cannot resolve Cargo.lock dependency: " + reference)
            result.add(matches[0])
        self.resolved[key] = result
        return result

    def resolve(self, parent: tuple[str, str, str], name: str, options: dict) -> tuple[str, str, str]:
        matches = [key for key in self.dependencies(parent) if key[0] == name and key[2]]
        if "git" in options:
            origin = "git+" + options["git"]
            matches = [key for key in matches if key[2].split("#", 1)[0].split("?", 1)[0] == origin]
        if len(matches) > 1 and "version" in options:
            matches = [key for key in matches if _version_matches(options["version"], key[1])]
        if len(matches) != 1:
            raise RuntimeError(f"Cannot resolve {name} in Cargo.lock production inputs for {parent[0]}")
        return matches[0]


def _version_matches(requirement: str, version: str) -> bool:
    """Disambiguate same-name lock entries using Cargo's numeric version requirements."""
    actual = tuple(map(int, version.split("-", 1)[0].split("+", 1)[0].split(".")))
    for condition in requirement.split(","):
        match = re.fullmatch(r"(\^|~|<=|>=|=|<|>)?\s*([0-9.*]+)", condition.strip())
        if match is None:
            raise RuntimeError("Cannot disambiguate Cargo.lock version requirement: " + requirement)
        operator, value = match.groups()
        parts = tuple(map(int, value.rstrip(".*").split("."))) if value != "*" else ()
        if "*" in value or operator == "=":
            valid = actual[:len(parts)] == parts
        elif operator in ("<", "<=", ">", ">="):
            current = actual[:len(parts)]
            valid = {"<": current < parts, "<=": current <= parts,
                     ">": current > parts, ">=": current >= parts}[operator]
        else:
            floor = parts + (0,) * (3 - len(parts))
            if operator == "~":
                width = min(len(parts), 2)
            else:
                width = next((index + 1 for index, part in enumerate(parts) if part), len(parts))
            valid = actual >= floor and actual[:width] == parts[:width]
        if not valid:
            return False
    return True


def component_version(kind: str, project: Path = PROJECT) -> str:
    return _Inputs(_Sources(project), kind).version()


def component_source_hashes(kind: str, project: Path = PROJECT) -> dict[str, str]:
    return component_metadata(kind, project)["sourceHashes"]

def component_metadata_at(kind: str, commit: str, project: Path = PROJECT) -> dict:
    return _Inputs(_Sources(project,_exact_commit(commit)),kind).metadata()


def component_source_fingerprint(kind: str, project: Path = PROJECT) -> str:
    return component_metadata(kind, project)["sourceFingerprint"]


def component_metadata(kind: str, project: Path = PROJECT) -> dict:
    return _Inputs(_Sources(project), kind).metadata()


def _only_lock_changed(current: dict, previous: dict) -> bool:
    new_hashes = current["sourceHashes"]
    old_hashes = previous["sourceHashes"]
    return (new_hashes["Cargo.lock"] != old_hashes["Cargo.lock"]
            and {path: value for path, value in new_hashes.items() if path != "Cargo.lock"}
            == {path: value for path, value in old_hashes.items() if path != "Cargo.lock"})


def _graph_subset(left: _Inputs, right: _Inputs) -> bool:
    return left.edges.keys() <= right.edges.keys() and all(
        left.lock.packages[key] == right.lock.packages[key] and children <= right.edges[key]
        for key, children in left.edges.items()
    )


def _lock_change_is_ambiguous(current: _Inputs, previous: dict, sources: _Sources) -> bool:
    historical = _Inputs(sources, current.kind)
    if historical.metadata()["sourceFingerprint"] != previous["sourceFingerprint"]:
        raise RuntimeError("Cannot establish historical Cargo.lock production projection")
    # The lock stores feature-unified registry edges without recording why they exist.
    # A pure graph addition or removal may be attributable to an unrelated workspace feature.
    return _graph_subset(historical, current) or _graph_subset(current, historical)


def lock_change_is_ambiguous(kind: str, previous: dict, project: Path = PROJECT) -> bool:
    """Classify lock-only graph additions/removals for retaining an existing binary."""
    current = _Inputs(_Sources(project), kind)
    metadata = current.metadata()
    if not _only_lock_changed(metadata, previous):
        return False
    sources = _Sources(project, _exact_commit(previous.get("sourceCommit")))
    return _lock_change_is_ambiguous(current, previous, sources)


def _report_outputs(report: dict) -> dict:
    outputs = report.get("outputs") if isinstance(report, dict) else None
    if not isinstance(outputs, dict) or not outputs or outputs.keys() - OUTPUT_COMPONENTS.keys():
        raise RuntimeError("Invalid or missing formal release outputs")
    return outputs


def _normalize_report(report: dict, project: Path, histories: dict, selected: str | None = None) -> dict:
    recorded_outputs = _report_outputs(report)
    schema = report.get("schemaVersion")
    if "schemaVersion" in report and (type(schema) is not int or schema != REPORT_SCHEMA_VERSION):
        raise RuntimeError("Unsupported release report schemaVersion")
    modern = schema == REPORT_SCHEMA_VERSION or any(
        isinstance(output, dict) and any(key in output for key in METADATA_KEYS)
        for output in recorded_outputs.values()
    )
    if modern and any(not isinstance(output, dict) or not all(key in output for key in METADATA_KEYS)
                      for output in recorded_outputs.values()):
        raise RuntimeError("Every output in a component release report needs its own metadata")

    outputs = {}
    for path, output in recorded_outputs.items():
        kind = OUTPUT_COMPONENTS[path]
        if selected is not None and kind != selected:
            continue
        if (not isinstance(output, dict) or not _valid_sha(output.get("sha256"))
                or type(output.get("bytes")) is not int or output["bytes"] <= 0):
            raise RuntimeError("Invalid release binary identity: " + path)
        if modern:
            if output.get("component") != kind:
                raise RuntimeError("Invalid component for release output: " + path)
            version_tuple(output.get("version"))
            hashes = _checked_hashes(output.get("sourceHashes"))
            if output.get("sourceFingerprint") != _fingerprint(hashes):
                raise RuntimeError("Invalid release source fingerprint: " + path)
            commit = _exact_commit(output.get("sourceCommit"))
            recorded = None
        else:
            commit = _exact_commit(report.get("sourceCommit"))
            recorded = _checked_hashes(report.get("sourceHashes"))

        if commit not in histories:
            histories[commit] = _Sources(project, commit)
        metadata = _Inputs(histories[commit], kind, recorded).metadata()
        if modern:
            if any(output[key] != value for key, value in metadata.items()):
                raise RuntimeError("Release component metadata differs from sourceCommit: " + path)
        else:
            _check_legacy_metadata(report, output, kind, metadata["version"])
        outputs[path] = {**output, **metadata, "sourceCommit": commit}
    return {**report, "schemaVersion": REPORT_SCHEMA_VERSION, "outputs": outputs}


def _check_legacy_metadata(report: dict, output: dict, kind: str, version: str) -> None:
    fields = ("connectorVersion", "connectorSourceHashes", "connectorSourceFingerprint")
    if not any(key in report or key in output for key in fields):
        return
    old_version = report.get("connectorVersion")
    version_tuple(old_version)
    if kind != "center" and old_version != version:
        raise RuntimeError("Historical Connector version differs from sourceCommit")
    hashes = _checked_hashes(report.get("connectorSourceHashes"))
    fingerprint = report.get("connectorSourceFingerprint")
    if fingerprint != _fingerprint(hashes):
        raise RuntimeError("Invalid historical Connector source fingerprint")
    if any(key in output and output[key] != report.get(key) for key in fields):
        raise RuntimeError("Historical Connector output differs from its report")


def normalize_report(report: dict, project: Path = PROJECT) -> dict:
    """Read legacy and component releases without modifying any historical output records."""
    return _normalize_report(report, project, {})


def guard_component(kind: str, report_paths: list[Path], *, project: Path = PROJECT,
                    require_unchanged: bool = False) -> dict:
    inputs = _Inputs(_Sources(project), kind)
    current = inputs.metadata()
    version = version_tuple(current["version"])
    histories = {}
    found = False
    for path in report_paths:
        report = _normalize_report(json.loads(path.read_text()), project, histories, kind)
        for relative, previous in report["outputs"].items():
            found = True
            previous_version = version_tuple(previous["version"])
            changed = previous["sourceFingerprint"] != current["sourceFingerprint"]
            if version < previous_version:
                raise RuntimeError(f"{kind} version downgrade rejected: {relative}")
            if changed and version == previous_version:
                if (_only_lock_changed(current, previous)
                        and _lock_change_is_ambiguous(inputs, previous, histories[previous["sourceCommit"]])):
                    raise RuntimeError(f"{kind} Cargo.lock has an ambiguous feature-unified edge change: {relative}")
                raise RuntimeError(f"{kind} production inputs changed; increase {kind} version: {relative}")
            if require_unchanged and (changed or version != previous_version):
                raise RuntimeError(f"{kind} production inputs or version changed: {relative}")
    if not found:
        raise RuntimeError(f"Previous {kind} release output is required")
    return current


def verify_output_binary(relative: str, output: dict, project: Path = PROJECT) -> None:
    if relative not in OUTPUT_COMPONENTS:
        raise RuntimeError("Unknown formal release output: " + relative)
    binary = project / relative
    if (binary.is_symlink() or not binary.is_file() or binary.stat().st_size != output.get("bytes")
            or _digest(binary.read_bytes()) != output.get("sha256")):
        raise RuntimeError("Release binary differs from its report: " + relative)
