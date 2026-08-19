package runner

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type BuildInput struct {
	BuildID        string            `json:"buildId"`
	ProjectDir     string            `json:"projectDir"`
	Platform       string            `json:"platform"`
	Environment    string            `json:"environment"`
	ArtifactType   string            `json:"artifactType"`
	GitURL         string            `json:"gitUrl"`
	GitRef         string            `json:"gitRef"`
	CommitSHA      string            `json:"commitSha,omitempty"`
	CredentialRefs map[string]string `json:"credentialRefs"`
	MaxAttempts    int               `json:"maxAttempts"`
}

func WorkspacePath(root string, buildID string) string {
	return filepath.Join(root, "workspaces", buildID)
}

func OutputPath(workspace string) string {
	return filepath.Join(workspace, "output")
}

func CheckoutPath(workspace string) string {
	return filepath.Join(workspace, "project")
}

func CheckoutSource(ctx context.Context, workspace string, gitURL string, gitRef string) (string, error) {
	projectRoot := CheckoutPath(workspace)
	if strings.TrimSpace(gitURL) == "" {
		return "", fmt.Errorf("gitUrl is required")
	}
	if _, err := os.Stat(filepath.Join(projectRoot, ".git")); err == nil {
		if err := runGit(ctx, projectRoot, "fetch", "--all", "--prune"); err != nil {
			return "", err
		}
	} else {
		if err := os.RemoveAll(projectRoot); err != nil {
			return "", err
		}
		if err := os.MkdirAll(filepath.Dir(projectRoot), 0o755); err != nil {
			return "", err
		}
		cmd := exec.CommandContext(ctx, "git", "clone", gitURL, projectRoot)
		if output, err := cmd.CombinedOutput(); err != nil {
			return "", fmt.Errorf("git clone failed: %w: %s", err, strings.TrimSpace(string(output)))
		}
	}
	ref := strings.TrimSpace(gitRef)
	if ref != "" {
		if err := runGit(ctx, projectRoot, "checkout", "--force", ref); err != nil {
			return "", err
		}
	}
	if err := runGit(ctx, projectRoot, "submodule", "update", "--init", "--recursive"); err != nil {
		return "", err
	}
	commit, err := gitOutput(ctx, projectRoot, "rev-parse", "HEAD")
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(commit), nil
}

func runGit(ctx context.Context, dir string, args ...string) error {
	cmd := exec.CommandContext(ctx, "git", args...)
	cmd.Dir = dir
	if output, err := cmd.CombinedOutput(); err != nil {
		return fmt.Errorf("git %s failed: %w: %s", strings.Join(args, " "), err, strings.TrimSpace(string(output)))
	}
	return nil
}

func gitOutput(ctx context.Context, dir string, args ...string) (string, error) {
	cmd := exec.CommandContext(ctx, "git", args...)
	cmd.Dir = dir
	output, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("git %s failed: %w: %s", strings.Join(args, " "), err, strings.TrimSpace(string(output)))
	}
	return string(output), nil
}

func WriteBuildInput(workspace string, input BuildInput) (string, error) {
	path := filepath.Join(workspace, "build-input.json")
	if err := os.MkdirAll(workspace, 0o755); err != nil {
		return "", err
	}
	data, err := json.MarshalIndent(input, "", "  ")
	if err != nil {
		return "", err
	}
	return path, os.WriteFile(path, data, 0o600)
}
