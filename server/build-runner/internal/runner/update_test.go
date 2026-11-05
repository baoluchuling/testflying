package runner

import (
	"archive/zip"
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestIsNewerVersion(t *testing.T) {
	cases := []struct {
		name    string
		current string
		target  string
		want    bool
	}{
		{name: "major", current: "1.9.9", target: "2.0.0", want: true},
		{name: "minor", current: "1.2.9", target: "1.3.0", want: true},
		{name: "patch", current: "1.2.3", target: "1.2.4", want: true},
		{name: "dev", current: "dev", target: "0.2.0", want: true},
		{name: "equal", current: "1.2.3", target: "1.2.3", want: false},
		{name: "older", current: "2.0.0", target: "1.9.9", want: false},
		{name: "invalid target", current: "1.0.0", target: "latest", want: false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := IsNewerVersion(tc.current, tc.target); got != tc.want {
				t.Fatalf("IsNewerVersion(%q, %q) = %v, want %v", tc.current, tc.target, got, tc.want)
			}
		})
	}
}

func TestClientCheckUpdateUsesAuthenticatedRunnerContract(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/admin/api/build-runners/runner-1/updates/check" {
			t.Fatalf("path = %q", r.URL.Path)
		}
		if r.Header.Get("Authorization") != "Bearer runner-token" {
			t.Fatalf("authorization = %q", r.Header.Get("Authorization"))
		}
		var payload UpdateCheckRequest
		if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
			t.Fatal(err)
		}
		if payload.Platform != "darwin" || payload.Arch != "arm64" {
			t.Fatalf("scope = %s/%s", payload.Platform, payload.Arch)
		}
		if payload.RunnerVersion != "0.1.0" || payload.PackageAgentVersion != "0.1.0" {
			t.Fatalf("versions = %#v", payload)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{
            "updateAvailable":true,
            "version":"0.2.0",
            "runnerVersion":"0.2.0",
            "packageAgentVersion":"0.2.0",
            "bundleUrl":"/admin/api/build-runners/runner-1/updates/darwin/arm64/0.2.0/bundle",
            "sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        }`))
	}))
	defer server.Close()

	client := NewClient(server.URL, "runner-token", server.Client())
	manifest, err := client.CheckUpdate(context.Background(), "runner-1", UpdateCheckRequest{
		Platform:            "darwin",
		Arch:                "arm64",
		RunnerVersion:       "0.1.0",
		PackageAgentVersion: "0.1.0",
	})
	if err != nil {
		t.Fatal(err)
	}
	if manifest == nil || manifest.Version != "0.2.0" {
		t.Fatalf("manifest = %#v", manifest)
	}
}

func TestClientCheckUpdateReturnsNilWhenCurrent(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{"updateAvailable":false}`))
	}))
	defer server.Close()

	client := NewClient(server.URL, "runner-token", server.Client())
	manifest, err := client.CheckUpdate(
		context.Background(),
		"runner-1",
		UpdateCheckRequest{},
	)
	if err != nil {
		t.Fatal(err)
	}
	if manifest != nil {
		t.Fatalf("manifest = %#v, want nil", manifest)
	}
}

func TestInstallUpdateReplacesBothBinariesAfterChecksum(t *testing.T) {
	installDir := t.TempDir()
	writeExecutable(t, filepath.Join(installDir, "testflying-build-runner"), "old-runner")
	writeExecutable(t, filepath.Join(installDir, "package-agent"), "old-agent")
	bundle := updateBundle(t, map[string]string{
		"testflying-build-runner": "new-runner",
		"package-agent":           "new-agent",
	})
	digest := fmt.Sprintf("%x", sha256.Sum256(bundle))
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/bundle" {
			t.Fatalf("path = %q", r.URL.Path)
		}
		if r.Header.Get("Authorization") != "Bearer runner-token" {
			t.Fatalf("authorization = %q", r.Header.Get("Authorization"))
		}
		_, _ = w.Write(bundle)
	}))
	defer server.Close()

	client := NewClient(server.URL, "runner-token", server.Client())
	err := InstallUpdate(context.Background(), client, Config{
		InstallDir:          installDir,
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
	}, UpdateManifest{
		Version:             "0.2.0",
		RunnerVersion:       "0.2.0",
		PackageAgentVersion: "0.2.0",
		BundleURL:           "/bundle",
		SHA256:              digest,
	})

	if !errors.Is(err, ErrUpdateInstalled) {
		t.Fatalf("InstallUpdate() error = %v, want ErrUpdateInstalled", err)
	}
	assertFileContent(t, filepath.Join(installDir, "testflying-build-runner"), "new-runner")
	assertFileContent(t, filepath.Join(installDir, "package-agent"), "new-agent")
}

func TestInstallUpdateChecksumMismatchKeepsInstalledBinaries(t *testing.T) {
	installDir := t.TempDir()
	writeExecutable(t, filepath.Join(installDir, "testflying-build-runner"), "old-runner")
	writeExecutable(t, filepath.Join(installDir, "package-agent"), "old-agent")
	bundle := updateBundle(t, map[string]string{
		"testflying-build-runner": "new-runner",
		"package-agent":           "new-agent",
	})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		_, _ = w.Write(bundle)
	}))
	defer server.Close()

	client := NewClient(server.URL, "runner-token", server.Client())
	err := InstallUpdate(context.Background(), client, Config{
		InstallDir:          installDir,
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
	}, UpdateManifest{
		RunnerVersion:       "0.2.0",
		PackageAgentVersion: "0.2.0",
		BundleURL:           "/bundle",
		SHA256:              strings.Repeat("0", sha256.Size*2),
	})

	if err == nil || !strings.Contains(err.Error(), "mismatch") {
		t.Fatalf("InstallUpdate() error = %v, want checksum mismatch", err)
	}
	assertFileContent(t, filepath.Join(installDir, "testflying-build-runner"), "old-runner")
	assertFileContent(t, filepath.Join(installDir, "package-agent"), "old-agent")
}

func TestInstallUpdateRejectsMixedVersionDowngradeBeforeDownload(t *testing.T) {
	var downloadCalls int
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		downloadCalls++
		w.WriteHeader(http.StatusInternalServerError)
	}))
	defer server.Close()

	client := NewClient(server.URL, "runner-token", server.Client())
	err := InstallUpdate(context.Background(), client, Config{
		InstallDir:          t.TempDir(),
		Version:             "0.1.0",
		PackageAgentVersion: "0.2.0",
	}, UpdateManifest{
		RunnerVersion:       "0.2.0",
		PackageAgentVersion: "0.1.0",
		BundleURL:           "/bundle",
		SHA256:              strings.Repeat("0", sha256.Size*2),
	})

	if err == nil || !strings.Contains(err.Error(), "downgrade") {
		t.Fatalf("InstallUpdate() error = %v, want downgrade rejection", err)
	}
	if downloadCalls != 0 {
		t.Fatalf("download calls = %d, want 0", downloadCalls)
	}
}

func TestExtractUpdateBundleRejectsUnsafeEntries(t *testing.T) {
	tests := []struct {
		name  string
		files []zipTestFile
	}{
		{
			name: "parent traversal",
			files: []zipTestFile{
				{name: "../testflying-build-runner", content: "runner", mode: 0o755},
				{name: "package-agent", content: "agent", mode: 0o755},
			},
		},
		{
			name: "absolute path",
			files: []zipTestFile{
				{name: "/testflying-build-runner", content: "runner", mode: 0o755},
				{name: "package-agent", content: "agent", mode: 0o755},
			},
		},
		{
			name: "extra file",
			files: []zipTestFile{
				{name: "testflying-build-runner", content: "runner", mode: 0o755},
				{name: "package-agent", content: "agent", mode: 0o755},
				{name: "unexpected", content: "extra", mode: 0o755},
			},
		},
		{
			name: "symlink",
			files: []zipTestFile{
				{name: "testflying-build-runner", content: "target", mode: os.ModeSymlink | 0o755},
				{name: "package-agent", content: "agent", mode: 0o755},
			},
		},
		{
			name: "missing binary",
			files: []zipTestFile{
				{name: "testflying-build-runner", content: "runner", mode: 0o755},
			},
		},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			bundlePath := filepath.Join(t.TempDir(), "update.zip")
			if err := os.WriteFile(bundlePath, updateBundleFiles(t, tc.files), 0o600); err != nil {
				t.Fatal(err)
			}
			if err := extractUpdateBundle(bundlePath, t.TempDir()); err == nil {
				t.Fatal("extractUpdateBundle() error = nil, want rejection")
			}
		})
	}
}

func TestReplaceUpdateFilesRollsBackWhenSecondInstallFails(t *testing.T) {
	installDir := t.TempDir()
	extractedDir := t.TempDir()
	writeExecutable(t, filepath.Join(installDir, "testflying-build-runner"), "old-runner")
	writeExecutable(t, filepath.Join(installDir, "package-agent"), "old-agent")
	writeExecutable(t, filepath.Join(extractedDir, "testflying-build-runner"), "new-runner")
	writeExecutable(t, filepath.Join(extractedDir, "package-agent"), "new-agent")

	failed := false
	rename := func(oldPath string, newPath string) error {
		if !failed && oldPath == filepath.Join(extractedDir, "package-agent") {
			failed = true
			return errors.New("injected second install failure")
		}
		return os.Rename(oldPath, newPath)
	}
	if err := replaceUpdateFiles(installDir, extractedDir, rename); err == nil {
		t.Fatal("replaceUpdateFiles() error = nil, want injected failure")
	}

	assertFileContent(t, filepath.Join(installDir, "testflying-build-runner"), "old-runner")
	assertFileContent(t, filepath.Join(installDir, "package-agent"), "old-agent")
	for _, name := range updateBinaryNames {
		if _, err := os.Stat(filepath.Join(installDir, name) + ".previous"); !errors.Is(err, os.ErrNotExist) {
			t.Fatalf("backup %s still exists: %v", name, err)
		}
	}
}

type zipTestFile struct {
	name    string
	content string
	mode    os.FileMode
}

func updateBundle(t *testing.T, files map[string]string) []byte {
	t.Helper()
	entries := make([]zipTestFile, 0, len(files))
	for name, content := range files {
		entries = append(entries, zipTestFile{name: name, content: content, mode: 0o755})
	}
	return updateBundleFiles(t, entries)
}

func updateBundleFiles(t *testing.T, files []zipTestFile) []byte {
	t.Helper()
	var buffer bytes.Buffer
	writer := zip.NewWriter(&buffer)
	for _, file := range files {
		header := &zip.FileHeader{Name: file.name, Method: zip.Store}
		header.SetMode(file.mode)
		entry, err := writer.CreateHeader(header)
		if err != nil {
			t.Fatal(err)
		}
		if _, err := entry.Write([]byte(file.content)); err != nil {
			t.Fatal(err)
		}
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}
	return buffer.Bytes()
}

func writeExecutable(t *testing.T, path string, content string) {
	t.Helper()
	if err := os.WriteFile(path, []byte(content), 0o755); err != nil {
		t.Fatal(err)
	}
}

func assertFileContent(t *testing.T, path string, want string) {
	t.Helper()
	content, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(content) != want {
		t.Fatalf("%s content = %q, want %q", path, content, want)
	}
}
