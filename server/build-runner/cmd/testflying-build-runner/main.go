package main

import (
	"context"
	"errors"
	"log"
	"os"
	"os/signal"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"time"

	"testflying-build-runner/internal/runner"
)

func main() {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	executable, err := os.Executable()
	if err != nil {
		log.Fatal(err)
	}
	pollInterval, err := durationEnvOrDefault(
		"TESTFLYING_BUILD_RUNNER_POLL_INTERVAL",
		5*time.Second,
	)
	if err != nil {
		log.Fatal(err)
	}
	updateInterval, err := durationEnvOrDefault(
		"TESTFLYING_BUILD_RUNNER_UPDATE_INTERVAL",
		30*time.Minute,
	)
	if err != nil {
		log.Fatal(err)
	}

	cfg := runner.Config{
		RunnerID:            os.Getenv("TESTFLYING_BUILD_RUNNER_ID"),
		Name:                os.Getenv("TESTFLYING_BUILD_RUNNER_NAME"),
		Token:               os.Getenv("TESTFLYING_BUILD_RUNNER_TOKEN"),
		ServerURL:           os.Getenv("TESTFLYING_BUILD_RUNNER_SERVER_URL"),
		RootDir:             os.Getenv("TESTFLYING_BUILD_RUNNER_ROOT"),
		PackageAgentBin:     os.Getenv("TESTFLYING_PACKAGE_AGENT_BIN"),
		Version:             envOrDefault("TESTFLYING_BUILD_RUNNER_VERSION", "dev"),
		PackageAgentVersion: envOrDefault("TESTFLYING_PACKAGE_AGENT_VERSION", "dev"),
		InstallDir:          filepath.Dir(executable),
		Platform:            runtime.GOOS,
		Arch:                runtime.GOARCH,
		PollInterval:        pollInterval,
		UpdateInterval:      updateInterval,
		Labels:              splitCSV(os.Getenv("TESTFLYING_BUILD_RUNNER_LABELS")),
		LLMAdapters:         splitCSV(os.Getenv("TESTFLYING_BUILD_RUNNER_LLM_ADAPTERS")),
		Capacity:            1,
	}
	if len(cfg.LLMAdapters) == 0 {
		cfg.LLMAdapters = []string{"codex"}
	}

	if err := runner.Run(ctx, cfg); err != nil {
		if errors.Is(err, runner.ErrUpdateInstalled) {
			log.Print("runner update installed; exiting for supervisor restart")
			return
		}
		log.Fatal(err)
	}
}

func durationEnvOrDefault(key string, fallback time.Duration) (time.Duration, error) {
	raw := strings.TrimSpace(os.Getenv(key))
	if raw == "" {
		return fallback, nil
	}
	value, err := time.ParseDuration(raw)
	if err != nil {
		return 0, errors.New(key + " must be a valid duration")
	}
	if value <= 0 {
		return 0, errors.New(key + " must be positive")
	}
	return value, nil
}

func envOrDefault(key string, fallback string) string {
	if value := strings.TrimSpace(os.Getenv(key)); value != "" {
		return value
	}
	return fallback
}

func splitCSV(raw string) []string {
	if strings.TrimSpace(raw) == "" {
		return nil
	}

	parts := strings.Split(raw, ",")
	values := make([]string, 0, len(parts))
	for _, part := range parts {
		value := strings.TrimSpace(part)
		if value != "" {
			values = append(values, value)
		}
	}
	return values
}
