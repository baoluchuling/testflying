package runner

import (
	"fmt"
	"strings"
	"time"
)

const BuildRetryLimit = 5

type Config struct {
	RunnerID            string
	Name                string
	Token               string
	ServerURL           string
	RootDir             string
	PackageAgentBin     string
	Version             string
	PackageAgentVersion string
	InstallDir          string
	Platform            string
	Arch                string
	PollInterval        time.Duration
	UpdateInterval      time.Duration
	Labels              []string
	LLMAdapters         []string
	Capacity            int
}

func (c Config) Validate() error {
	if err := c.validateBuildConfig(); err != nil {
		return err
	}
	if strings.TrimSpace(c.InstallDir) == "" {
		return fmt.Errorf("InstallDir is required")
	}
	if c.PollInterval <= 0 {
		return fmt.Errorf("PollInterval must be positive")
	}
	if c.UpdateInterval <= 0 {
		return fmt.Errorf("UpdateInterval must be positive")
	}
	if c.Platform != "darwin" {
		return fmt.Errorf("Platform must be darwin")
	}
	if c.Arch != "arm64" && c.Arch != "amd64" {
		return fmt.Errorf("Arch must be arm64 or amd64")
	}
	return nil
}

func (c Config) validateBuildConfig() error {
	required := map[string]string{
		"RunnerID":        c.RunnerID,
		"Name":            c.Name,
		"Token":           c.Token,
		"ServerURL":       c.ServerURL,
		"RootDir":         c.RootDir,
		"PackageAgentBin": c.PackageAgentBin,
	}
	for field, value := range required {
		if strings.TrimSpace(value) == "" {
			return fmt.Errorf("%s is required", field)
		}
	}
	if c.Capacity == 0 {
		return fmt.Errorf("Capacity is required")
	}
	if c.Capacity != 1 {
		return fmt.Errorf("Capacity must be 1 in the current implementation")
	}
	return nil
}
