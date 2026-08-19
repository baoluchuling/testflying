package runner

import (
	"archive/zip"
	"context"
	"crypto/sha256"
	"crypto/subtle"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

var ErrUpdateInstalled = errors.New("runner update installed")

var updateBinaryNames = []string{"testflying-build-runner", "package-agent"}

func IsNewerVersion(current string, target string) bool {
	comparison, ok := compareSemanticVersion(current, target)
	return ok && comparison > 0
}

func compareSemanticVersion(current string, target string) (int, bool) {
	targetParts, ok := parseSemanticVersion(target)
	if !ok {
		return 0, false
	}
	if strings.TrimSpace(current) == "dev" {
		return 1, true
	}
	currentParts, ok := parseSemanticVersion(current)
	if !ok {
		return 0, false
	}
	for index := range currentParts {
		if targetParts[index] > currentParts[index] {
			return 1, true
		}
		if targetParts[index] < currentParts[index] {
			return -1, true
		}
	}
	return 0, true
}

func parseSemanticVersion(value string) ([3]int, bool) {
	var parsed [3]int
	parts := strings.Split(strings.TrimSpace(value), ".")
	if len(parts) != len(parsed) {
		return parsed, false
	}
	for index, part := range parts {
		if part == "" {
			return parsed, false
		}
		number, err := strconv.Atoi(part)
		if err != nil || number < 0 {
			return parsed, false
		}
		parsed[index] = number
	}
	return parsed, true
}

func InstallUpdate(
	ctx context.Context,
	client *Client,
	cfg Config,
	manifest UpdateManifest,
) error {
	runnerComparison, runnerValid := compareSemanticVersion(cfg.Version, manifest.RunnerVersion)
	agentComparison, agentValid := compareSemanticVersion(
		cfg.PackageAgentVersion,
		manifest.PackageAgentVersion,
	)
	if !runnerValid || !agentValid {
		return nil
	}
	if runnerComparison < 0 || agentComparison < 0 {
		return fmt.Errorf("update would downgrade an installed component")
	}
	if runnerComparison == 0 && agentComparison == 0 {
		return nil
	}
	if strings.TrimSpace(cfg.InstallDir) == "" {
		return fmt.Errorf("InstallDir is required for updates")
	}
	if len(manifest.SHA256) != sha256.Size*2 {
		return fmt.Errorf("update SHA-256 is invalid")
	}
	if err := os.MkdirAll(cfg.InstallDir, 0o755); err != nil {
		return err
	}

	body, err := client.download(ctx, manifest.BundleURL)
	if err != nil {
		return err
	}
	defer body.Close()
	bundle, err := os.CreateTemp(cfg.InstallDir, ".testflying-update-*.zip")
	if err != nil {
		return err
	}
	bundlePath := bundle.Name()
	defer os.Remove(bundlePath)
	hasher := sha256.New()
	if _, err := io.Copy(io.MultiWriter(bundle, hasher), body); err != nil {
		bundle.Close()
		return err
	}
	if err := bundle.Close(); err != nil {
		return err
	}
	actualDigest := fmt.Sprintf("%x", hasher.Sum(nil))
	if subtle.ConstantTimeCompare(
		[]byte(actualDigest),
		[]byte(strings.ToLower(manifest.SHA256)),
	) != 1 {
		return fmt.Errorf("update SHA-256 mismatch")
	}

	extractedDir, err := os.MkdirTemp(cfg.InstallDir, ".testflying-update-extracted-*")
	if err != nil {
		return err
	}
	defer os.RemoveAll(extractedDir)
	if err := extractUpdateBundle(bundlePath, extractedDir); err != nil {
		return err
	}
	if err := replaceUpdateFiles(cfg.InstallDir, extractedDir, os.Rename); err != nil {
		return err
	}
	return ErrUpdateInstalled
}

func extractUpdateBundle(bundlePath string, destination string) error {
	reader, err := zip.OpenReader(bundlePath)
	if err != nil {
		return err
	}
	defer reader.Close()
	if len(reader.File) != len(updateBinaryNames) {
		return fmt.Errorf("update bundle must contain exactly two files")
	}
	seen := make(map[string]bool, len(updateBinaryNames))
	for _, entry := range reader.File {
		if !isExpectedUpdateBinary(entry.Name) || seen[entry.Name] {
			return fmt.Errorf("unsafe update bundle entry %q", entry.Name)
		}
		mode := entry.Mode()
		if mode&os.ModeSymlink != 0 || !mode.IsRegular() {
			return fmt.Errorf("update bundle entry %q is not a regular file", entry.Name)
		}
		input, err := entry.Open()
		if err != nil {
			return err
		}
		destinationPath := filepath.Join(destination, entry.Name)
		output, err := os.OpenFile(destinationPath, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o755)
		if err != nil {
			input.Close()
			return err
		}
		_, copyErr := io.Copy(output, input)
		closeOutputErr := output.Close()
		closeInputErr := input.Close()
		if copyErr != nil {
			return copyErr
		}
		if closeOutputErr != nil {
			return closeOutputErr
		}
		if closeInputErr != nil {
			return closeInputErr
		}
		seen[entry.Name] = true
	}
	for _, name := range updateBinaryNames {
		if !seen[name] {
			return fmt.Errorf("update bundle missing %s", name)
		}
	}
	return nil
}

func isExpectedUpdateBinary(name string) bool {
	if filepath.Base(name) != name || strings.Contains(name, "\\") {
		return false
	}
	for _, expected := range updateBinaryNames {
		if name == expected {
			return true
		}
	}
	return false
}

type renameFileFunc func(oldPath string, newPath string) error

func replaceUpdateFiles(installDir string, extractedDir string, rename renameFileFunc) error {
	for _, name := range updateBinaryNames {
		currentPath := filepath.Join(installDir, name)
		stagedPath := filepath.Join(extractedDir, name)
		if info, err := os.Stat(stagedPath); err != nil || !info.Mode().IsRegular() {
			return fmt.Errorf("staged update binary %s is invalid", name)
		}
		if err := os.Chmod(stagedPath, 0o755); err != nil {
			return err
		}
		if info, err := os.Stat(currentPath); err != nil || !info.Mode().IsRegular() {
			return fmt.Errorf("installed update binary %s is invalid", name)
		}
	}

	backedUp := make([]string, 0, len(updateBinaryNames))
	for _, name := range updateBinaryNames {
		currentPath := filepath.Join(installDir, name)
		backupPath := currentPath + ".previous"
		if err := os.Remove(backupPath); err != nil && !errors.Is(err, os.ErrNotExist) {
			return err
		}
		if err := rename(currentPath, backupPath); err != nil {
			restoreUpdateBackups(installDir, backedUp, rename)
			return err
		}
		backedUp = append(backedUp, name)
	}

	installed := make([]string, 0, len(updateBinaryNames))
	for _, name := range updateBinaryNames {
		if err := rename(
			filepath.Join(extractedDir, name),
			filepath.Join(installDir, name),
		); err != nil {
			for _, installedName := range installed {
				_ = os.Remove(filepath.Join(installDir, installedName))
			}
			restoreUpdateBackups(installDir, backedUp, rename)
			return err
		}
		installed = append(installed, name)
	}
	for _, name := range backedUp {
		if err := os.Remove(filepath.Join(installDir, name) + ".previous"); err != nil {
			return err
		}
	}
	return nil
}

func restoreUpdateBackups(installDir string, names []string, rename renameFileFunc) {
	for index := len(names) - 1; index >= 0; index-- {
		name := names[index]
		_ = rename(
			filepath.Join(installDir, name)+".previous",
			filepath.Join(installDir, name),
		)
	}
}
