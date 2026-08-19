package runner

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type AgentReport struct {
	Status         string   `json:"status"`
	Classification string   `json:"classification"`
	Summary        string   `json:"summary"`
	HumanAction    string   `json:"humanAction"`
	PackagePaths   []string `json:"packagePaths"`
	SymbolsPaths   []string `json:"symbolsPaths"`
	LogPaths       []string `json:"logPaths"`
	Version        string   `json:"version"`
	BuildNumber    string   `json:"buildNumber"`
	CommitSHA      string   `json:"commitSha"`
	Adapter        *string  `json:"adapter"`
	MaxAttempts    int      `json:"maxAttempts"`
}

type ArtifactUpload struct {
	ArtifactType string
	Path         string
	UploadName   string
}

func RunPackageAgent(ctx context.Context, binary string, inputPath string, outputDir string) error {
	cmd := exec.CommandContext(ctx, binary, "build", "--input", inputPath, "--output", outputDir)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

func ReadAgentReport(outputDir string) (AgentReport, error) {
	var report AgentReport
	data, err := os.ReadFile(filepath.Join(outputDir, "report.json"))
	if err != nil {
		return report, err
	}
	if err := json.Unmarshal(data, &report); err != nil {
		return report, err
	}
	return report, nil
}

func ArtifactUploadsFromReport(outputDir string, report AgentReport) ([]ArtifactUpload, error) {
	uploads := []ArtifactUpload{
		{
			ArtifactType: "report",
			Path:         filepath.Join(outputDir, "report.json"),
			UploadName:   "report.json",
		},
	}

	required := []struct {
		artifactType string
		paths        []string
	}{
		{artifactType: "package", paths: report.PackagePaths},
		{artifactType: "symbols", paths: report.SymbolsPaths},
		{artifactType: "log", paths: report.LogPaths},
	}

	for _, item := range required {
		if len(item.paths) == 0 {
			return nil, fmt.Errorf("report is missing %s paths", item.artifactType)
		}
		for _, artifactPath := range item.paths {
			resolved, err := resolveArtifactPath(outputDir, artifactPath)
			if err != nil {
				return nil, fmt.Errorf("%s artifact path %q is invalid: %w", item.artifactType, artifactPath, err)
			}
			uploadName, err := safeUploadFileName(outputDir, resolved)
			if err != nil {
				return nil, fmt.Errorf("%s artifact path %q has invalid upload name: %w", item.artifactType, artifactPath, err)
			}
			uploads = append(uploads, ArtifactUpload{
				ArtifactType: item.artifactType,
				Path:         resolved,
				UploadName:   uploadName,
			})
		}
	}

	return uploads, nil
}

func CompleteStatusFromReport(status string) string {
	switch status {
	case "success":
		return "succeeded"
	default:
		return status
	}
}

func resolveArtifactPath(outputDir string, pathValue string) (string, error) {
	if pathValue == "" {
		return "", fmt.Errorf("path is blank")
	}

	outputDirAbs, err := filepath.Abs(outputDir)
	if err != nil {
		return "", fmt.Errorf("resolve output dir: %w", err)
	}
	outputDirReal, err := filepath.EvalSymlinks(outputDirAbs)
	if err != nil {
		return "", fmt.Errorf("resolve output dir symlinks: %w", err)
	}

	candidate := pathValue
	if !filepath.IsAbs(candidate) {
		candidate = filepath.Join(outputDirAbs, candidate)
	}
	candidateAbs, err := filepath.Abs(candidate)
	if err != nil {
		return "", fmt.Errorf("resolve path: %w", err)
	}
	if !pathWithinDir(outputDirAbs, candidateAbs) {
		return "", fmt.Errorf("path escapes output directory")
	}
	resolved, err := filepath.EvalSymlinks(candidateAbs)
	if err != nil {
		return "", fmt.Errorf("resolve symlinks: %w", err)
	}
	if !pathWithinDir(outputDirReal, resolved) {
		return "", fmt.Errorf("path escapes output directory via symlink")
	}
	info, err := os.Stat(resolved)
	if err != nil {
		return "", fmt.Errorf("stat artifact: %w", err)
	}
	if !info.Mode().IsRegular() {
		return "", fmt.Errorf("artifact is not a regular file")
	}
	return resolved, nil
}

func pathWithinDir(root string, candidate string) bool {
	rel, err := filepath.Rel(root, candidate)
	if err != nil {
		return false
	}
	return rel == "." || (rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator)))
}

func safeUploadFileName(outputDir string, artifactPath string) (string, error) {
	outputDirAbs, err := filepath.Abs(outputDir)
	if err != nil {
		return "", fmt.Errorf("resolve output dir: %w", err)
	}
	outputDirReal, err := filepath.EvalSymlinks(outputDirAbs)
	if err != nil {
		return "", fmt.Errorf("resolve output dir symlinks: %w", err)
	}
	rel, err := filepath.Rel(outputDirReal, artifactPath)
	if err != nil {
		return "", fmt.Errorf("relative path: %w", err)
	}
	if rel == "." || rel == "" {
		return "", fmt.Errorf("path is not an artifact file")
	}

	base := filepath.Base(rel)
	ext := filepath.Ext(base)
	stem := strings.TrimSuffix(base, ext)
	if stem == "" {
		stem = "artifact"
	}

	sum := sha256.Sum256([]byte(rel))
	shortHash := hex.EncodeToString(sum[:6])

	return fmt.Sprintf("%s-%s%s", stem, shortHash, ext), nil
}
