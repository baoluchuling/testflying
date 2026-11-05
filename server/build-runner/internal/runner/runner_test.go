package runner

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestWorkspacePathUsesBuildID(t *testing.T) {
	root := t.TempDir()
	path := WorkspacePath(root, "build-123")

	want := filepath.Join(root, "workspaces", "build-123")
	if path != want {
		t.Fatalf("WorkspacePath() = %q, want %q", path, want)
	}
}

func TestWriteBuildInputCreatesJSON(t *testing.T) {
	root := t.TempDir()
	input := BuildInput{
		BuildID:        "build-1",
		ProjectDir:     root,
		Platform:       "ios",
		Environment:    "development",
		ArtifactType:   "ipa",
		CredentialRefs: map[string]string{"git": "git-main"},
		MaxAttempts:    BuildRetryLimit,
	}

	path, err := WriteBuildInput(root, input)
	if err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}

	var got BuildInput
	if err := json.Unmarshal(data, &got); err != nil {
		t.Fatal(err)
	}

	if got.BuildID != input.BuildID {
		t.Fatalf("buildId = %q, want %q", got.BuildID, input.BuildID)
	}
	if got.ProjectDir != input.ProjectDir {
		t.Fatalf("projectDir = %q, want %q", got.ProjectDir, input.ProjectDir)
	}
	if got.Platform != input.Platform {
		t.Fatalf("platform = %q, want %q", got.Platform, input.Platform)
	}
	if got.Environment != input.Environment {
		t.Fatalf("environment = %q, want %q", got.Environment, input.Environment)
	}
	if got.ArtifactType != input.ArtifactType {
		t.Fatalf("artifactType = %q, want %q", got.ArtifactType, input.ArtifactType)
	}
	if got.CredentialRefs["git"] != input.CredentialRefs["git"] {
		t.Fatalf("credentialRefs[git] = %q, want %q", got.CredentialRefs["git"], input.CredentialRefs["git"])
	}
	if got.MaxAttempts != BuildRetryLimit {
		t.Fatalf("maxAttempts = %d, want %d", got.MaxAttempts, BuildRetryLimit)
	}
	var raw map[string]interface{}
	if err := json.Unmarshal(data, &raw); err != nil {
		t.Fatal(err)
	}
	if _, exists := raw["repoSubpath"]; exists {
		t.Fatal("build input must not contain repoSubpath")
	}
}

func TestRedactTextCoversSecrets(t *testing.T) {
	input := "token=abc123456789\npassword: hunter2\nAuthorization: Bearer abcdefghijklmnop\nghp_abcdefghijklmnop\n-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----"
	redacted := RedactText(input)
	for _, secret := range []string{"abc123456789", "hunter2", "abcdefghijklmnop", "ghp_"} {
		if strings.Contains(redacted, secret) {
			t.Fatalf("redacted text still contains %q: %q", secret, redacted)
		}
	}
	if strings.Count(redacted, "[REDACTED]") < 4 {
		t.Fatalf("redacted text = %q", redacted)
	}
}

func TestRunOnceNoBuildOnlyHeartbeatsAndPolls(t *testing.T) {
	var heartbeatCalls int
	var pollCalls int
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/admin/api/build-runners/heartbeat":
			heartbeatCalls++
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/poll":
			pollCalls++
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"build":null}`))
		default:
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
	}))
	defer server.Close()

	cfg := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           server.URL,
		RootDir:             t.TempDir(),
		PackageAgentBin:     "package-agent",
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		Platform:            "darwin",
		Arch:                "arm64",
		Labels:              []string{"ios-release"},
		LLMAdapters:         []string{"codex"},
		Capacity:            1,
	}

	if err := RunOnce(context.Background(), cfg, server.Client()); err != nil {
		t.Fatal(err)
	}

	if heartbeatCalls != 1 {
		t.Fatalf("heartbeat calls = %d, want 1", heartbeatCalls)
	}
	if pollCalls != 1 {
		t.Fatalf("poll calls = %d, want 1", pollCalls)
	}
}

func TestRunChecksForUpdatesAndPollsUntilCanceled(t *testing.T) {
	var updateCalls int
	var heartbeatCalls int
	var pollCalls int
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/admin/api/build-runners/runner-1/updates/check":
			updateCalls++
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"updateAvailable":false}`))
		case "/admin/api/build-runners/heartbeat":
			heartbeatCalls++
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/poll":
			pollCalls++
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"build":null}`))
			if pollCalls == 2 {
				cancel()
			}
		default:
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
	}))
	defer server.Close()

	cfg := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           server.URL,
		RootDir:             t.TempDir(),
		PackageAgentBin:     "package-agent",
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		InstallDir:          t.TempDir(),
		Platform:            "darwin",
		Arch:                "arm64",
		PollInterval:        time.Millisecond,
		UpdateInterval:      time.Hour,
		Capacity:            1,
	}

	if err := Run(ctx, cfg); err != nil {
		t.Fatal(err)
	}
	if updateCalls != 1 {
		t.Fatalf("update calls = %d, want 1", updateCalls)
	}
	if heartbeatCalls != 2 || pollCalls != 2 {
		t.Fatalf("heartbeat/poll calls = %d/%d, want 2/2", heartbeatCalls, pollCalls)
	}
}

func TestConfigValidateRequiresFieldsAndCapacityOne(t *testing.T) {
	valid := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           "https://example.test",
		RootDir:             t.TempDir(),
		PackageAgentBin:     "/tmp/package-agent",
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		InstallDir:          t.TempDir(),
		Platform:            "darwin",
		Arch:                "arm64",
		PollInterval:        5 * time.Second,
		UpdateInterval:      30 * time.Minute,
		Capacity:            1,
	}
	if err := valid.Validate(); err != nil {
		t.Fatalf("Validate() unexpected error: %v", err)
	}

	cases := []struct {
		name string
		cfg  Config
		want string
	}{
		{
			name: "missing runner id",
			cfg: func() Config {
				cfg := valid
				cfg.RunnerID = ""
				return cfg
			}(),
			want: "RunnerID is required",
		},
		{
			name: "missing name",
			cfg: func() Config {
				cfg := valid
				cfg.Name = " "
				return cfg
			}(),
			want: "Name is required",
		},
		{
			name: "missing token",
			cfg: func() Config {
				cfg := valid
				cfg.Token = ""
				return cfg
			}(),
			want: "Token is required",
		},
		{
			name: "missing server url",
			cfg: func() Config {
				cfg := valid
				cfg.ServerURL = ""
				return cfg
			}(),
			want: "ServerURL is required",
		},
		{
			name: "missing root dir",
			cfg: func() Config {
				cfg := valid
				cfg.RootDir = ""
				return cfg
			}(),
			want: "RootDir is required",
		},
		{
			name: "missing package agent bin",
			cfg: func() Config {
				cfg := valid
				cfg.PackageAgentBin = ""
				return cfg
			}(),
			want: "PackageAgentBin is required",
		},
		{
			name: "missing capacity",
			cfg: func() Config {
				cfg := valid
				cfg.Capacity = 0
				return cfg
			}(),
			want: "Capacity is required",
		},
		{
			name: "capacity must stay one",
			cfg: func() Config {
				cfg := valid
				cfg.Capacity = 2
				return cfg
			}(),
			want: "Capacity must be 1 in the current implementation",
		},
		{
			name: "poll interval must be positive",
			cfg: func() Config {
				cfg := valid
				cfg.PollInterval = 0
				return cfg
			}(),
			want: "PollInterval must be positive",
		},
		{
			name: "update interval must be positive",
			cfg: func() Config {
				cfg := valid
				cfg.UpdateInterval = -time.Second
				return cfg
			}(),
			want: "UpdateInterval must be positive",
		},
		{
			name: "unsupported platform",
			cfg: func() Config {
				cfg := valid
				cfg.Platform = "linux"
				return cfg
			}(),
			want: "Platform must be darwin",
		},
		{
			name: "unsupported architecture",
			cfg: func() Config {
				cfg := valid
				cfg.Arch = "x86_64"
				return cfg
			}(),
			want: "Arch must be arm64 or amd64",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := tc.cfg.Validate()
			if err == nil {
				t.Fatal("Validate() error = nil, want error")
			}
			if err.Error() != tc.want {
				t.Fatalf("Validate() error = %q, want %q", err.Error(), tc.want)
			}
		})
	}
}

func TestRunOnceAssignedBuildRunsPackageAgentPostsEventsAndCompletesNeedsHuman(t *testing.T) {
	root := t.TempDir()
	gitURL := createSourceRepo(t, root, []string{"ios/app/.keep"})
	packageAgentBin := writeFakePackageAgentWithFiles(t, root, 0, `{
  "status": "success",
  "classification": "build_succeeded",
  "summary": "fake package-agent completed",
  "version": "1.2.3",
  "buildNumber": "42",
  "commitSha": "abc123def456",
  "packagePaths": ["app.ipa"],
  "symbolsPaths": ["app.dSYM.zip", "symbols/additional.dSYM.zip"],
  "logPaths": ["logs/a-b/c.log", "logs/a/b-c.log"],
  "maxAttempts": 5
}`, map[string]string{
		"app.ipa":                     "ipa-bytes",
		"app.dSYM.zip":                "symbols-bytes",
		"symbols/additional.dSYM.zip": "additional-symbols",
		"logs/a-b/c.log":              "left-log",
		"logs/a/b-c.log":              "right-log",
	})

	type capturedEvent struct {
		Type            string                 `json:"type"`
		LifecycleStatus string                 `json:"lifecycleStatus"`
		Payload         map[string]interface{} `json:"payload"`
	}
	type capturedComplete struct {
		RunnerID              string `json:"runnerId"`
		Status                string `json:"status"`
		Version               string `json:"version"`
		BuildNumber           string `json:"buildNumber"`
		CommitSHA             string `json:"commitSha"`
		Note                  string `json:"note"`
		FailureClassification string `json:"failureClassification"`
		FailureSummary        string `json:"failureSummary"`
		HumanAction           string `json:"humanAction"`
	}
	type uploadedArtifact struct {
		ArtifactType string
		FileName     string
		Body         string
	}

	var heartbeatBody heartbeatRequest
	var pollBody pollRequest
	events := make([]capturedEvent, 0, 2)
	completes := make([]capturedComplete, 0, 1)
	uploads := make([]uploadedArtifact, 0, 4)

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/admin/api/build-runners/heartbeat":
			if err := json.NewDecoder(r.Body).Decode(&heartbeatBody); err != nil {
				t.Fatalf("decode heartbeat: %v", err)
			}
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/poll":
			if err := json.NewDecoder(r.Body).Decode(&pollBody); err != nil {
				t.Fatalf("decode poll: %v", err)
			}
			w.Header().Set("Content-Type", "application/json")
			writeBuildAssignmentResponse(t, w, gitURL)
		case "/admin/api/build-runners/builds/build-123/events":
			var event capturedEvent
			if err := json.NewDecoder(r.Body).Decode(&event); err != nil {
				t.Fatalf("decode event: %v", err)
			}
			events = append(events, event)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/artifacts":
			if err := r.ParseMultipartForm(1 << 20); err != nil {
				t.Fatalf("parse multipart form: %v", err)
			}
			file, header, err := r.FormFile("file")
			if err != nil {
				t.Fatalf("read artifact file: %v", err)
			}
			defer file.Close()
			data, err := io.ReadAll(file)
			if err != nil {
				t.Fatalf("read artifact body: %v", err)
			}
			uploads = append(uploads, uploadedArtifact{
				ArtifactType: r.FormValue("artifactType"),
				FileName:     header.Filename,
				Body:         string(data),
			})
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/complete":
			var complete capturedComplete
			if err := json.NewDecoder(r.Body).Decode(&complete); err != nil {
				t.Fatalf("decode complete: %v", err)
			}
			completes = append(completes, complete)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		default:
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
	}))
	defer server.Close()

	cfg := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           server.URL,
		RootDir:             root,
		PackageAgentBin:     packageAgentBin,
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		Platform:            "darwin",
		Arch:                "arm64",
		Labels:              []string{"ios-release"},
		LLMAdapters:         []string{"codex"},
		Capacity:            1,
	}

	if err := RunOnce(context.Background(), cfg, server.Client()); err != nil {
		t.Fatal(err)
	}

	if heartbeatBody.RunnerID != cfg.RunnerID {
		t.Fatalf("heartbeat runnerId = %q, want %q", heartbeatBody.RunnerID, cfg.RunnerID)
	}
	if heartbeatBody.Capabilities["hostPlatform"] != cfg.Platform {
		t.Fatalf(
			"heartbeat hostPlatform = %q, want %q",
			heartbeatBody.Capabilities["hostPlatform"],
			cfg.Platform,
		)
	}
	if heartbeatBody.Capabilities["arch"] != cfg.Arch {
		t.Fatalf("heartbeat arch = %q, want %q", heartbeatBody.Capabilities["arch"], cfg.Arch)
	}
	platforms, ok := heartbeatBody.Capabilities["platforms"].([]interface{})
	if !ok || len(platforms) != 2 || platforms[0] != "ios" || platforms[1] != "android" {
		t.Fatalf("heartbeat platforms = %#v, want ios and android", platforms)
	}
	if pollBody.RunnerID != cfg.RunnerID {
		t.Fatalf("poll runnerId = %q, want %q", pollBody.RunnerID, cfg.RunnerID)
	}

	workspace := WorkspacePath(root, "build-123")
	inputPath := filepath.Join(workspace, "build-input.json")
	outputDir := OutputPath(workspace)

	if _, err := os.Stat(workspace); err != nil {
		t.Fatalf("workspace stat: %v", err)
	}
	if _, err := os.Stat(inputPath); err != nil {
		t.Fatalf("build-input stat: %v", err)
	}

	inputData, err := os.ReadFile(inputPath)
	if err != nil {
		t.Fatal(err)
	}
	var gotInput BuildInput
	if err := json.Unmarshal(inputData, &gotInput); err != nil {
		t.Fatal(err)
	}
	expectedProjectDir := CheckoutPath(workspace)
	if gotInput.ProjectDir != expectedProjectDir {
		t.Fatalf("projectDir = %q, want %q", gotInput.ProjectDir, expectedProjectDir)
	}
	if gotInput.MaxAttempts != BuildRetryLimit {
		t.Fatalf("maxAttempts = %d, want %d", gotInput.MaxAttempts, BuildRetryLimit)
	}

	invocationData, err := os.ReadFile(filepath.Join(outputDir, "invocation.txt"))
	if err != nil {
		t.Fatal(err)
	}
	invocation := string(invocationData)
	if !strings.Contains(invocation, "input="+inputPath) {
		t.Fatalf("invocation missing input path: %q", invocation)
	}
	if !strings.Contains(invocation, "output="+outputDir) {
		t.Fatalf("invocation missing output path: %q", invocation)
	}

	if len(events) != 2 {
		t.Fatalf("event count = %d, want 2", len(events))
	}
	if events[0].Type != "runner.build.started" {
		t.Fatalf("first event type = %q", events[0].Type)
	}
	if events[0].LifecycleStatus != "building" {
		t.Fatalf("first event lifecycleStatus = %q", events[0].LifecycleStatus)
	}
	if events[0].Payload["workspace"] != workspace {
		t.Fatalf("first event workspace = %#v, want %q", events[0].Payload["workspace"], workspace)
	}
	if events[1].Type != "runner.build.finished" {
		t.Fatalf("second event type = %q", events[1].Type)
	}
	if events[1].LifecycleStatus != "building" {
		t.Fatalf("second event lifecycleStatus = %q", events[1].LifecycleStatus)
	}
	if events[1].Payload["workspace"] != workspace {
		t.Fatalf("second event workspace = %#v, want %q", events[1].Payload["workspace"], workspace)
	}
	if events[1].Payload["outputDir"] != outputDir {
		t.Fatalf("second event outputDir = %#v, want %q", events[1].Payload["outputDir"], outputDir)
	}

	reportPayload, ok := events[1].Payload["report"].(map[string]interface{})
	if !ok {
		t.Fatalf("second event report payload type = %T", events[1].Payload["report"])
	}
	if reportPayload["summary"] != "fake package-agent completed" {
		t.Fatalf("report summary = %#v", reportPayload["summary"])
	}
	if reportPayload["maxAttempts"] != float64(BuildRetryLimit) {
		t.Fatalf("report maxAttempts = %#v", reportPayload["maxAttempts"])
	}

	if len(completes) != 1 {
		t.Fatalf("complete count = %d, want 1", len(completes))
	}
	if completes[0].RunnerID != cfg.RunnerID {
		t.Fatalf("complete runnerId = %q, want %q", completes[0].RunnerID, cfg.RunnerID)
	}
	if completes[0].Status != "succeeded" {
		t.Fatalf("complete status = %q, want %q", completes[0].Status, "succeeded")
	}
	if completes[0].Version != "1.2.3" {
		t.Fatalf("complete version = %q", completes[0].Version)
	}
	if completes[0].BuildNumber != "42" {
		t.Fatalf("complete buildNumber = %q", completes[0].BuildNumber)
	}
	if completes[0].CommitSHA != "abc123def456" {
		t.Fatalf("complete commitSha = %q", completes[0].CommitSHA)
	}
	if completes[0].FailureClassification != "" {
		t.Fatalf("complete failureClassification = %q, want empty", completes[0].FailureClassification)
	}
	if completes[0].FailureSummary != "" {
		t.Fatalf("complete failureSummary = %q, want empty", completes[0].FailureSummary)
	}
	if completes[0].Note != "fake package-agent completed" {
		t.Fatalf("complete note = %q", completes[0].Note)
	}
	if completes[0].HumanAction != "" {
		t.Fatalf("complete humanAction = %q, want empty", completes[0].HumanAction)
	}

	if len(uploads) != 6 {
		t.Fatalf("upload count = %d, want 6", len(uploads))
	}
	if uploads[0].ArtifactType != "report" || uploads[0].FileName != "report.json" {
		t.Fatalf("report upload = %#v, want report.json", uploads[0])
	}
	if uploads[1].ArtifactType != "package" || !strings.HasPrefix(uploads[1].FileName, "app-") || !strings.HasSuffix(uploads[1].FileName, ".ipa") {
		t.Fatalf("package upload filename = %q, want hashed ipa name", uploads[1].FileName)
	}
	if uploads[2].ArtifactType != "symbols" || !strings.HasPrefix(uploads[2].FileName, "app.dSYM-") || !strings.HasSuffix(uploads[2].FileName, ".zip") {
		t.Fatalf("symbols upload filename = %q, want hashed zip name", uploads[2].FileName)
	}
	if uploads[3].ArtifactType != "symbols" || !strings.HasPrefix(uploads[3].FileName, "additional.dSYM-") || !strings.HasSuffix(uploads[3].FileName, ".zip") {
		t.Fatalf("nested symbols upload filename = %q, want hashed zip name", uploads[3].FileName)
	}
	if uploads[4].ArtifactType != "log" || !strings.HasPrefix(uploads[4].FileName, "c-") || !strings.HasSuffix(uploads[4].FileName, ".log") {
		t.Fatalf("left log upload filename = %q, want hashed log name", uploads[4].FileName)
	}
	if uploads[5].ArtifactType != "log" || !strings.HasPrefix(uploads[5].FileName, "b-c-") || !strings.HasSuffix(uploads[5].FileName, ".log") {
		t.Fatalf("right log upload filename = %q, want hashed log name", uploads[5].FileName)
	}
	if uploads[4].FileName == uploads[5].FileName {
		t.Fatalf("log upload filenames collided: %#v", uploads[4].FileName)
	}
}

func TestRunOnceAssignedBuildCompletesFailedWhenPackageAgentExitsNonZero(t *testing.T) {
	root := t.TempDir()
	gitURL := createSourceRepo(t, root, []string{"ios/app/.keep"})
	packageAgentBin := writeFakePackageAgent(t, root, 7, `{
  "status": "failed",
  "classification": "codesign_failed",
  "summary": "codesign step failed",
  "packagePaths": [],
  "symbolsPaths": [],
  "logPaths": ["runner.log"],
  "maxAttempts": 5
}`)

	type capturedEvent struct {
		Type            string                 `json:"type"`
		LifecycleStatus string                 `json:"lifecycleStatus"`
		Message         string                 `json:"message"`
		Payload         map[string]interface{} `json:"payload"`
	}
	type capturedComplete struct {
		RunnerID              string `json:"runnerId"`
		Status                string `json:"status"`
		Note                  string `json:"note"`
		FailureClassification string `json:"failureClassification"`
		FailureSummary        string `json:"failureSummary"`
	}

	events := make([]capturedEvent, 0, 2)
	completes := make([]capturedComplete, 0, 1)

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/admin/api/build-runners/heartbeat":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/poll":
			w.Header().Set("Content-Type", "application/json")
			writeBuildAssignmentResponse(t, w, gitURL)
		case "/admin/api/build-runners/builds/build-123/events":
			var event capturedEvent
			if err := json.NewDecoder(r.Body).Decode(&event); err != nil {
				t.Fatalf("decode event: %v", err)
			}
			events = append(events, event)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/complete":
			var complete capturedComplete
			if err := json.NewDecoder(r.Body).Decode(&complete); err != nil {
				t.Fatalf("decode complete: %v", err)
			}
			completes = append(completes, complete)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		default:
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
	}))
	defer server.Close()

	cfg := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           server.URL,
		RootDir:             root,
		PackageAgentBin:     packageAgentBin,
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		Labels:              []string{"ios-release"},
		LLMAdapters:         []string{"codex"},
		Capacity:            1,
	}

	err := RunOnce(context.Background(), cfg, server.Client())
	if err == nil {
		t.Fatal("RunOnce() error = nil, want error")
	}
	var exitErr *exec.ExitError
	if !errors.As(err, &exitErr) {
		t.Fatalf("RunOnce() error type = %T, want *os.ExitError", err)
	}
	if exitErr.ExitCode() != 7 {
		t.Fatalf("RunOnce() exit code = %d, want 7", exitErr.ExitCode())
	}

	if len(events) != 2 {
		t.Fatalf("event count = %d, want 2", len(events))
	}
	if events[0].Type != "runner.build.started" {
		t.Fatalf("first event type = %q", events[0].Type)
	}
	if events[1].Type != "runner.build.failed" {
		t.Fatalf("second event type = %q, want %q", events[1].Type, "runner.build.failed")
	}
	if events[1].Message != "package-agent failed: exit status 7" {
		t.Fatalf("second event message = %q", events[1].Message)
	}
	if events[1].Payload["error"] != "exit status 7" {
		t.Fatalf("second event error payload = %#v", events[1].Payload["error"])
	}
	reportPayload, ok := events[1].Payload["report"].(map[string]interface{})
	if !ok {
		t.Fatalf("second event report payload type = %T", events[1].Payload["report"])
	}
	if reportPayload["classification"] != "codesign_failed" {
		t.Fatalf("report classification = %#v", reportPayload["classification"])
	}
	if reportPayload["summary"] != "codesign step failed" {
		t.Fatalf("report summary = %#v", reportPayload["summary"])
	}

	if len(completes) != 1 {
		t.Fatalf("complete count = %d, want 1", len(completes))
	}
	if completes[0].Status != "failed" {
		t.Fatalf("complete status = %q, want %q", completes[0].Status, "failed")
	}
	if completes[0].FailureClassification != "codesign_failed" {
		t.Fatalf("complete failureClassification = %q", completes[0].FailureClassification)
	}
	if completes[0].FailureSummary != "codesign step failed" {
		t.Fatalf("complete failureSummary = %q", completes[0].FailureSummary)
	}
	if completes[0].Note != "codesign step failed" {
		t.Fatalf("complete note = %q", completes[0].Note)
	}
}

func TestRunOnceAssignedBuildCompletesNeedsHumanWhenPackageAgentExitsTwo(t *testing.T) {
	root := t.TempDir()
	gitURL := createSourceRepo(t, root, []string{"ios/app/.keep"})
	packageAgentBin := writeFakePackageAgent(t, root, 2, `{
  "status": "needs_human",
  "classification": "missing_artifacts",
  "summary": "Automatic success requires package, symbols, and logs.",
  "humanAction": "Upload the missing package, symbols, and logs, then resolve the build.",
  "packagePaths": [],
  "symbolsPaths": [],
  "logPaths": ["runner.log"],
  "maxAttempts": 5
}`)

	type capturedEvent struct {
		Type            string                 `json:"type"`
		LifecycleStatus string                 `json:"lifecycleStatus"`
		Message         string                 `json:"message"`
		Payload         map[string]interface{} `json:"payload"`
	}
	type capturedComplete struct {
		RunnerID              string `json:"runnerId"`
		Status                string `json:"status"`
		Note                  string `json:"note"`
		FailureClassification string `json:"failureClassification"`
		FailureSummary        string `json:"failureSummary"`
		HumanAction           string `json:"humanAction"`
	}

	events := make([]capturedEvent, 0, 2)
	completes := make([]capturedComplete, 0, 1)

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/admin/api/build-runners/heartbeat":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/poll":
			w.Header().Set("Content-Type", "application/json")
			writeBuildAssignmentResponse(t, w, gitURL)
		case "/admin/api/build-runners/builds/build-123/events":
			var event capturedEvent
			if err := json.NewDecoder(r.Body).Decode(&event); err != nil {
				t.Fatalf("decode event: %v", err)
			}
			events = append(events, event)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/complete":
			var complete capturedComplete
			if err := json.NewDecoder(r.Body).Decode(&complete); err != nil {
				t.Fatalf("decode complete: %v", err)
			}
			completes = append(completes, complete)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		default:
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
	}))
	defer server.Close()

	cfg := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           server.URL,
		RootDir:             root,
		PackageAgentBin:     packageAgentBin,
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		Labels:              []string{"ios-release"},
		LLMAdapters:         []string{"codex"},
		Capacity:            1,
	}

	if err := RunOnce(context.Background(), cfg, server.Client()); err != nil {
		t.Fatalf("RunOnce() error = %v, want nil", err)
	}

	if len(events) != 2 {
		t.Fatalf("event count = %d, want 2", len(events))
	}
	if events[0].Type != "runner.build.started" {
		t.Fatalf("first event type = %q", events[0].Type)
	}
	if events[1].Type != "runner.build.needs_human" {
		t.Fatalf("second event type = %q, want %q", events[1].Type, "runner.build.needs_human")
	}
	if events[1].Message != "Upload the missing package, symbols, and logs, then resolve the build." {
		t.Fatalf("second event message = %q", events[1].Message)
	}
	reportPayload, ok := events[1].Payload["report"].(map[string]interface{})
	if !ok {
		t.Fatalf("second event report payload type = %T", events[1].Payload["report"])
	}
	if reportPayload["classification"] != "missing_artifacts" {
		t.Fatalf("report classification = %#v", reportPayload["classification"])
	}
	if reportPayload["summary"] != "Automatic success requires package, symbols, and logs." {
		t.Fatalf("report summary = %#v", reportPayload["summary"])
	}
	if reportPayload["humanAction"] != "Upload the missing package, symbols, and logs, then resolve the build." {
		t.Fatalf("report humanAction = %#v", reportPayload["humanAction"])
	}

	if len(completes) != 1 {
		t.Fatalf("complete count = %d, want 1", len(completes))
	}
	if completes[0].Status != "needs_human" {
		t.Fatalf("complete status = %q, want needs_human", completes[0].Status)
	}
	if completes[0].FailureClassification != "missing_artifacts" {
		t.Fatalf("complete failureClassification = %q", completes[0].FailureClassification)
	}
	if completes[0].FailureSummary != "Automatic success requires package, symbols, and logs." {
		t.Fatalf("complete failureSummary = %q", completes[0].FailureSummary)
	}
	if completes[0].Note != "Automatic success requires package, symbols, and logs." {
		t.Fatalf("complete note = %q", completes[0].Note)
	}
	if completes[0].HumanAction != "Upload the missing package, symbols, and logs, then resolve the build." {
		t.Fatalf("complete humanAction = %q", completes[0].HumanAction)
	}
}

func TestRunOnceAssignedBuildCompletesFailedWhenWorkspaceSetupFails(t *testing.T) {
	root := filepath.Join(t.TempDir(), "root-file")
	gitURL := "file:///workspace-setup-fails-before-checkout"
	if err := os.WriteFile(root, []byte("not a directory"), 0o600); err != nil {
		t.Fatal(err)
	}

	type capturedComplete struct {
		RunnerID              string `json:"runnerId"`
		Status                string `json:"status"`
		Note                  string `json:"note"`
		FailureClassification string `json:"failureClassification"`
		FailureSummary        string `json:"failureSummary"`
	}

	completes := make([]capturedComplete, 0, 1)

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/admin/api/build-runners/heartbeat":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/poll":
			w.Header().Set("Content-Type", "application/json")
			writeBuildAssignmentResponse(t, w, gitURL)
		case "/admin/api/build-runners/builds/build-123/complete":
			var complete capturedComplete
			if err := json.NewDecoder(r.Body).Decode(&complete); err != nil {
				t.Fatalf("decode complete: %v", err)
			}
			completes = append(completes, complete)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/events":
			t.Fatal("events should not be posted when workspace setup fails")
		default:
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
	}))
	defer server.Close()

	cfg := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           server.URL,
		RootDir:             root,
		PackageAgentBin:     "/path/to/package-agent",
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		Labels:              []string{"ios-release"},
		LLMAdapters:         []string{"codex"},
		Capacity:            1,
	}

	err := RunOnce(context.Background(), cfg, server.Client())
	if err == nil {
		t.Fatal("RunOnce() error = nil, want error")
	}
	if len(completes) != 1 {
		t.Fatalf("complete count = %d, want 1", len(completes))
	}
	if completes[0].Status != "failed" {
		t.Fatalf("complete status = %q, want failed", completes[0].Status)
	}
	if completes[0].FailureClassification != preAgentWorkspaceFailureClassification {
		t.Fatalf("complete failureClassification = %q", completes[0].FailureClassification)
	}
	if !strings.Contains(completes[0].Note, "failed to create build workspace") {
		t.Fatalf("complete note = %q", completes[0].Note)
	}
	if completes[0].FailureSummary != completes[0].Note {
		t.Fatalf("complete failureSummary = %q, want note match", completes[0].FailureSummary)
	}
}

func TestRunOnceAssignedBuildCompletesFailedWhenStartEventPostFails(t *testing.T) {
	root := t.TempDir()
	gitURL := createSourceRepo(t, root, []string{"ios/app/.keep"})

	type capturedComplete struct {
		RunnerID              string `json:"runnerId"`
		Status                string `json:"status"`
		Note                  string `json:"note"`
		FailureClassification string `json:"failureClassification"`
		FailureSummary        string `json:"failureSummary"`
	}

	var eventCalls int
	completes := make([]capturedComplete, 0, 1)

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/admin/api/build-runners/heartbeat":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/poll":
			w.Header().Set("Content-Type", "application/json")
			writeBuildAssignmentResponse(t, w, gitURL)
		case "/admin/api/build-runners/builds/build-123/events":
			eventCalls++
			http.Error(w, "start event rejected", http.StatusBadGateway)
		case "/admin/api/build-runners/builds/build-123/complete":
			var complete capturedComplete
			if err := json.NewDecoder(r.Body).Decode(&complete); err != nil {
				t.Fatalf("decode complete: %v", err)
			}
			completes = append(completes, complete)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		default:
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
	}))
	defer server.Close()

	cfg := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           server.URL,
		RootDir:             root,
		PackageAgentBin:     "/path/to/package-agent",
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		Labels:              []string{"ios-release"},
		LLMAdapters:         []string{"codex"},
		Capacity:            1,
	}

	err := RunOnce(context.Background(), cfg, server.Client())
	if err == nil {
		t.Fatal("RunOnce() error = nil, want error")
	}
	if eventCalls != 1 {
		t.Fatalf("event calls = %d, want 1", eventCalls)
	}
	if len(completes) != 1 {
		t.Fatalf("complete count = %d, want 1", len(completes))
	}
	if completes[0].Status != "failed" {
		t.Fatalf("complete status = %q, want failed", completes[0].Status)
	}
	if completes[0].FailureClassification != preAgentStartEventFailureClassification {
		t.Fatalf("complete failureClassification = %q", completes[0].FailureClassification)
	}
	if !strings.Contains(completes[0].Note, "failed to post runner.build.started") {
		t.Fatalf("complete note = %q", completes[0].Note)
	}
}

func TestRunOnceAssignedBuildCompletesNeedsHumanWhenFinishEventPostFails(t *testing.T) {
	root := t.TempDir()
	gitURL := createSourceRepo(t, root, []string{"ios/app/.keep"})
	packageAgentBin := writeFakePackageAgentWithFiles(t, root, 0, `{
  "status": "success",
  "classification": "build_succeeded",
  "summary": "fake package-agent completed",
  "packagePaths": ["app.ipa"],
  "symbolsPaths": ["app.dSYM.zip"],
  "logPaths": ["runner.log"],
  "maxAttempts": 5
}`, map[string]string{
		"app.ipa":      "ipa-bytes",
		"app.dSYM.zip": "symbols-bytes",
		"runner.log":   "runner-log",
	})

	type capturedEvent struct {
		Type string `json:"type"`
	}
	type capturedComplete struct {
		RunnerID              string `json:"runnerId"`
		Status                string `json:"status"`
		Note                  string `json:"note"`
		FailureClassification string `json:"failureClassification"`
		FailureSummary        string `json:"failureSummary"`
		HumanAction           string `json:"humanAction"`
	}

	events := make([]capturedEvent, 0, 2)
	completes := make([]capturedComplete, 0, 1)

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/admin/api/build-runners/heartbeat":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/poll":
			w.Header().Set("Content-Type", "application/json")
			writeBuildAssignmentResponse(t, w, gitURL)
		case "/admin/api/build-runners/builds/build-123/events":
			var event capturedEvent
			if err := json.NewDecoder(r.Body).Decode(&event); err != nil {
				t.Fatalf("decode event: %v", err)
			}
			events = append(events, event)
			if event.Type == "runner.build.finished" {
				http.Error(w, "finish event rejected", http.StatusBadGateway)
				return
			}
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/artifacts":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/complete":
			var complete capturedComplete
			if err := json.NewDecoder(r.Body).Decode(&complete); err != nil {
				t.Fatalf("decode complete: %v", err)
			}
			completes = append(completes, complete)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		default:
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
	}))
	defer server.Close()

	cfg := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           server.URL,
		RootDir:             root,
		PackageAgentBin:     packageAgentBin,
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		Labels:              []string{"ios-release"},
		LLMAdapters:         []string{"codex"},
		Capacity:            1,
	}

	err := RunOnce(context.Background(), cfg, server.Client())
	if err == nil {
		t.Fatal("RunOnce() error = nil, want error")
	}
	if len(events) != 2 {
		t.Fatalf("event count = %d, want 2", len(events))
	}
	if events[0].Type != "runner.build.started" || events[1].Type != "runner.build.finished" {
		t.Fatalf("event types = %#v", events)
	}
	if len(completes) != 1 {
		t.Fatalf("complete count = %d, want 1", len(completes))
	}
	if completes[0].Status != "needs_human" {
		t.Fatalf("complete status = %q, want needs_human", completes[0].Status)
	}
	if completes[0].FailureClassification != postAgentFinishEventFailureClassification {
		t.Fatalf("complete failureClassification = %q", completes[0].FailureClassification)
	}
	if !strings.Contains(completes[0].Note, postAgentFinishEventFailureSummary) {
		t.Fatalf("complete note = %q", completes[0].Note)
	}
	if completes[0].FailureSummary != completes[0].Note {
		t.Fatalf("complete failureSummary = %q, want note match", completes[0].FailureSummary)
	}
	if completes[0].HumanAction != postAgentFinishEventFailureAction {
		t.Fatalf("complete humanAction = %q", completes[0].HumanAction)
	}
}

func TestRunOnceAssignedBuildCompletesNeedsHumanWhenArtifactUploadFails(t *testing.T) {
	root := t.TempDir()
	gitURL := createSourceRepo(t, root, []string{"ios/app/.keep"})
	packageAgentBin := writeFakePackageAgent(t, root, 0, `{
  "status": "success",
  "classification": "build_succeeded",
  "summary": "fake package-agent completed",
  "packagePaths": ["missing.ipa"],
  "symbolsPaths": ["app.dSYM.zip"],
  "logPaths": ["runner.log"],
  "maxAttempts": 5
}`)

	type capturedComplete struct {
		RunnerID              string `json:"runnerId"`
		Status                string `json:"status"`
		Note                  string `json:"note"`
		FailureClassification string `json:"failureClassification"`
		FailureSummary        string `json:"failureSummary"`
		HumanAction           string `json:"humanAction"`
	}

	completes := make([]capturedComplete, 0, 1)
	var eventCalls int

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/admin/api/build-runners/heartbeat":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/poll":
			w.Header().Set("Content-Type", "application/json")
			writeBuildAssignmentResponse(t, w, gitURL)
		case "/admin/api/build-runners/builds/build-123/events":
			eventCalls++
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/complete":
			var complete capturedComplete
			if err := json.NewDecoder(r.Body).Decode(&complete); err != nil {
				t.Fatalf("decode complete: %v", err)
			}
			completes = append(completes, complete)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/artifacts":
			t.Fatal("artifact upload should not be attempted when the first required file is missing")
		default:
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
	}))
	defer server.Close()

	cfg := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           server.URL,
		RootDir:             root,
		PackageAgentBin:     packageAgentBin,
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		Labels:              []string{"ios-release"},
		LLMAdapters:         []string{"codex"},
		Capacity:            1,
	}

	err := RunOnce(context.Background(), cfg, server.Client())
	if err == nil {
		t.Fatal("RunOnce() error = nil, want error")
	}
	if eventCalls != 1 {
		t.Fatalf("event calls = %d, want 1", eventCalls)
	}
	if len(completes) != 1 {
		t.Fatalf("complete count = %d, want 1", len(completes))
	}
	if completes[0].Status != "needs_human" {
		t.Fatalf("complete status = %q, want needs_human", completes[0].Status)
	}
	if completes[0].FailureClassification != postAgentArtifactUploadFailureClassification {
		t.Fatalf("complete failureClassification = %q", completes[0].FailureClassification)
	}
	if !strings.Contains(completes[0].Note, "required artifact upload failed") {
		t.Fatalf("complete note = %q", completes[0].Note)
	}
	if !strings.Contains(completes[0].Note, "missing.ipa") {
		t.Fatalf("complete note = %q", completes[0].Note)
	}
	if completes[0].FailureSummary != completes[0].Note {
		t.Fatalf("complete failureSummary = %q, want note match", completes[0].FailureSummary)
	}
	if completes[0].HumanAction != postAgentArtifactUploadAction {
		t.Fatalf("complete humanAction = %q", completes[0].HumanAction)
	}
}

func TestArtifactUploadsFromReportRejectsPathsOutsideOutputDir(t *testing.T) {
	outputDir := t.TempDir()
	insidePackage := filepath.Join(outputDir, "app.ipa")
	insideSymbols := filepath.Join(outputDir, "app.dSYM.zip")
	insideLog := filepath.Join(outputDir, "runner.log")
	for _, path := range []string{insidePackage, insideSymbols, insideLog} {
		if err := os.WriteFile(path, []byte("artifact"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	outsideDir := t.TempDir()
	outsideFile := filepath.Join(outsideDir, "secret.log")
	if err := os.WriteFile(outsideFile, []byte("secret"), 0o600); err != nil {
		t.Fatal(err)
	}

	cases := []struct {
		name        string
		packagePath string
	}{
		{name: "parent traversal", packagePath: "../secret.ipa"},
		{name: "outside absolute path", packagePath: outsideFile},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			_, err := ArtifactUploadsFromReport(outputDir, AgentReport{
				PackagePaths: []string{tc.packagePath},
				SymbolsPaths: []string{"app.dSYM.zip"},
				LogPaths:     []string{"runner.log"},
			})
			if err == nil {
				t.Fatal("ArtifactUploadsFromReport() error = nil, want error")
			}
			if !strings.Contains(err.Error(), "invalid") {
				t.Fatalf("ArtifactUploadsFromReport() error = %q", err)
			}
		})
	}
}

func TestRunOnceAssignedBuildCompletesNeedsHumanWhenSuccessReportEscapesOutputDir(t *testing.T) {
	root := t.TempDir()
	gitURL := createSourceRepo(t, root, []string{"ios/app/.keep"})
	packageAgentBin := writeFakePackageAgentWithFiles(t, root, 0, `{
  "status": "success",
  "classification": "build_succeeded",
  "summary": "fake package-agent completed",
  "packagePaths": ["../secret.ipa"],
  "symbolsPaths": ["app.dSYM.zip"],
  "logPaths": ["runner.log"],
  "maxAttempts": 5
}`, map[string]string{
		"app.dSYM.zip": "symbols-bytes",
		"runner.log":   "runner-log",
	})

	type capturedComplete struct {
		RunnerID              string `json:"runnerId"`
		Status                string `json:"status"`
		Note                  string `json:"note"`
		FailureClassification string `json:"failureClassification"`
		FailureSummary        string `json:"failureSummary"`
		HumanAction           string `json:"humanAction"`
	}

	completes := make([]capturedComplete, 0, 1)
	var eventCalls int

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/admin/api/build-runners/heartbeat":
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/poll":
			w.Header().Set("Content-Type", "application/json")
			writeBuildAssignmentResponse(t, w, gitURL)
		case "/admin/api/build-runners/builds/build-123/events":
			eventCalls++
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/complete":
			var complete capturedComplete
			if err := json.NewDecoder(r.Body).Decode(&complete); err != nil {
				t.Fatalf("decode complete: %v", err)
			}
			completes = append(completes, complete)
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"ok":true}`))
		case "/admin/api/build-runners/builds/build-123/artifacts":
			t.Fatal("artifact upload should not be attempted when report escapes output directory")
		default:
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
	}))
	defer server.Close()

	cfg := Config{
		RunnerID:            "runner-1",
		Name:                "Runner 1",
		Token:               "token",
		ServerURL:           server.URL,
		RootDir:             root,
		PackageAgentBin:     packageAgentBin,
		Version:             "0.1.0",
		PackageAgentVersion: "0.1.0",
		Labels:              []string{"ios-release"},
		LLMAdapters:         []string{"codex"},
		Capacity:            1,
	}

	err := RunOnce(context.Background(), cfg, server.Client())
	if err == nil {
		t.Fatal("RunOnce() error = nil, want error")
	}
	if eventCalls != 1 {
		t.Fatalf("event calls = %d, want 1", eventCalls)
	}
	if len(completes) != 1 {
		t.Fatalf("complete count = %d, want 1", len(completes))
	}
	if completes[0].Status != "needs_human" {
		t.Fatalf("complete status = %q, want needs_human", completes[0].Status)
	}
	if completes[0].FailureClassification != postAgentArtifactUploadFailureClassification {
		t.Fatalf("complete failureClassification = %q", completes[0].FailureClassification)
	}
	if !strings.Contains(completes[0].Note, "escapes output directory") {
		t.Fatalf("complete note = %q", completes[0].Note)
	}
	if completes[0].HumanAction != postAgentArtifactUploadAction {
		t.Fatalf("complete humanAction = %q", completes[0].HumanAction)
	}
}

func TestCompleteStatusFromReport(t *testing.T) {
	if got := CompleteStatusFromReport("success"); got != "succeeded" {
		t.Fatalf("CompleteStatusFromReport(success) = %q, want succeeded", got)
	}
	if got := CompleteStatusFromReport("needs_human"); got != "needs_human" {
		t.Fatalf("CompleteStatusFromReport(needs_human) = %q, want needs_human", got)
	}
	if got := CompleteStatusFromReport("failed"); got != "failed" {
		t.Fatalf("CompleteStatusFromReport(failed) = %q, want failed", got)
	}
}

func writeFakePackageAgent(t *testing.T, root string, exitCode int, reportJSON string) string {
	return writeFakePackageAgentWithFiles(t, root, exitCode, reportJSON, nil)
}

func writeBuildAssignmentResponse(t *testing.T, w http.ResponseWriter, gitURL string) {
	t.Helper()
	_, _ = fmt.Fprintf(w, `{"build":{
		"id":"build-123",
		"appId":"app-1",
		"platform":"ios",
		"environment":"development",
		"gitUrl":%q,
		"gitRef":"main",
		"artifactType":"ipa",
		"credentialRefs":{"git":"git-main"}
	}}`, gitURL)
}

func createSourceRepo(t *testing.T, root string, files []string) string {
	t.Helper()
	repo := filepath.Join(root, "source-repo")
	if err := os.MkdirAll(repo, 0o755); err != nil {
		t.Fatal(err)
	}
	for _, file := range files {
		path := filepath.Join(repo, file)
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, []byte("fixture"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	runTestGit(t, repo, "init", "-b", "main")
	runTestGit(t, repo, "config", "user.email", "runner@example.test")
	runTestGit(t, repo, "config", "user.name", "Runner Test")
	runTestGit(t, repo, "add", ".")
	runTestGit(t, repo, "commit", "-m", "fixture")
	return repo
}

func runTestGit(t *testing.T, dir string, args ...string) {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = dir
	if output, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("git %s failed: %v: %s", strings.Join(args, " "), err, strings.TrimSpace(string(output)))
	}
}

func writeFakePackageAgentWithFiles(
	t *testing.T,
	root string,
	exitCode int,
	reportJSON string,
	outputFiles map[string]string,
) string {
	t.Helper()

	packageAgentBin := filepath.Join(root, "fake-package-agent.sh")
	fileWriters := ""
	for relativePath, body := range outputFiles {
		fileWriters += fmt.Sprintf(`
mkdir -p "$(dirname "$OUTPUT/%s")"
cat > "$OUTPUT/%s" <<'EOF'
%s
EOF
`, relativePath, relativePath, body)
	}
	script := fmt.Sprintf(`#!/bin/sh
set -eu

INPUT=""
OUTPUT=""

while [ "$#" -gt 0 ]; do
	case "$1" in
		build)
			shift
			;;
		--input)
			INPUT="$2"
			shift 2
			;;
		--output)
			OUTPUT="$2"
			shift 2
			;;
		*)
			echo "unexpected arg: $1" >&2
			exit 1
			;;
	esac
done

mkdir -p "$OUTPUT"
printf 'input=%%s\noutput=%%s\n' "$INPUT" "$OUTPUT" > "$OUTPUT/invocation.txt"
cat > "$OUTPUT/report.json" <<'EOF'
%s
EOF
%s
exit %d
`, reportJSON, fileWriters, exitCode)
	if err := os.WriteFile(packageAgentBin, []byte(script), 0o755); err != nil {
		t.Fatal(err)
	}
	return packageAgentBin
}
