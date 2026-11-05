package runner

import (
	"context"
	"errors"
	"fmt"
	"log"
	"net/http"
	"os"
	"os/exec"
	"time"
)

const (
	postAgentArtifactUploadFailureClassification = "artifact_upload_failed"
	postAgentArtifactUploadAction                = "Inspect the package-agent output directory, fix the missing or unreadable artifact, and rerun or manually resolve the build."
	preAgentWorkspaceFailureClassification       = "workspace_setup_failed"
	preAgentCheckoutFailureClassification        = "checkout_failed"
	preAgentInputFailureClassification           = "build_input_write_failed"
	preAgentStartEventFailureClassification      = "start_event_post_failed"
	postAgentFinishEventFailureClassification    = "finish_event_post_failed"
	postAgentFinishEventFailureSummary           = "package-agent completed, but the runner could not persist the finish event or verify artifacts."
	postAgentFinishEventFailureAction            = "Inspect the runner workspace output and verify required artifacts before resolving the build."
)

type BuildAssignment struct {
	ID             string            `json:"id"`
	AppID          string            `json:"appId"`
	Platform       string            `json:"platform"`
	Environment    string            `json:"environment"`
	GitURL         string            `json:"gitUrl"`
	GitRef         string            `json:"gitRef"`
	ArtifactType   string            `json:"artifactType"`
	CredentialRefs map[string]string `json:"credentialRefs"`
}

func Run(ctx context.Context, cfg Config) error {
	if err := cfg.Validate(); err != nil {
		return err
	}
	client := NewClient(cfg.ServerURL, cfg.Token, http.DefaultClient)
	nextUpdate := time.Time{}
	ticker := time.NewTicker(cfg.PollInterval)
	defer ticker.Stop()

	for {
		if time.Now().After(nextUpdate) {
			manifest, err := client.CheckUpdate(ctx, cfg.RunnerID, updateRequestFromConfig(cfg))
			if err != nil {
				log.Printf("update check failed: %s", RedactText(err.Error()))
			} else if manifest != nil {
				err = InstallUpdate(ctx, client, cfg, *manifest)
				if errors.Is(err, ErrUpdateInstalled) {
					return err
				}
				if err != nil {
					log.Printf("update install failed: %s", RedactText(err.Error()))
				}
			}
			nextUpdate = time.Now().Add(cfg.UpdateInterval)
		}

		if err := RunOnce(ctx, cfg, client.httpClient); err != nil {
			log.Printf("runner poll failed: %s", RedactText(err.Error()))
		}
		select {
		case <-ctx.Done():
			return nil
		case <-ticker.C:
		}
	}
}

func updateRequestFromConfig(cfg Config) UpdateCheckRequest {
	return UpdateCheckRequest{
		Platform:            cfg.Platform,
		Arch:                cfg.Arch,
		RunnerVersion:       cfg.Version,
		PackageAgentVersion: cfg.PackageAgentVersion,
	}
}

func RunOnce(ctx context.Context, cfg Config, httpClient *http.Client) error {
	if err := cfg.validateBuildConfig(); err != nil {
		return err
	}

	client := NewClient(cfg.ServerURL, cfg.Token, httpClient)
	if err := client.Heartbeat(ctx, cfg); err != nil {
		return err
	}

	build, err := client.Poll(ctx, cfg.RunnerID)
	if err != nil {
		return err
	}
	if build == nil {
		return nil
	}

	return handleBuild(ctx, client, cfg, *build)
}

func handleBuild(ctx context.Context, client *Client, cfg Config, build BuildAssignment) error {
	workspace := WorkspacePath(cfg.RootDir, build.ID)
	outputDir := OutputPath(workspace)
	if err := os.MkdirAll(outputDir, 0o755); err != nil {
		return completeWithOriginalError(
			ctx,
			client,
			build.ID,
			preAgentFailureRequest(
				cfg.RunnerID,
				preAgentWorkspaceFailureClassification,
				fmt.Sprintf("failed to create build workspace %q", outputDir),
				err,
			),
			err,
		)
	}

	commitSHA, err := CheckoutSource(ctx, workspace, build.GitURL, build.GitRef)
	if err != nil {
		return completeWithOriginalError(
			ctx,
			client,
			build.ID,
			preAgentFailureRequest(
				cfg.RunnerID,
				preAgentCheckoutFailureClassification,
				fmt.Sprintf("failed to checkout %q at %q", build.GitURL, build.GitRef),
				err,
			),
			err,
		)
	}

	inputPath, err := WriteBuildInput(workspace, BuildInput{
		BuildID:        build.ID,
		ProjectDir:     CheckoutPath(workspace),
		Platform:       build.Platform,
		Environment:    build.Environment,
		ArtifactType:   build.ArtifactType,
		GitURL:         build.GitURL,
		GitRef:         build.GitRef,
		CommitSHA:      commitSHA,
		CredentialRefs: build.CredentialRefs,
		MaxAttempts:    BuildRetryLimit,
	})
	if err != nil {
		return completeWithOriginalError(
			ctx,
			client,
			build.ID,
			preAgentFailureRequest(
				cfg.RunnerID,
				preAgentInputFailureClassification,
				fmt.Sprintf("failed to write build input for workspace %q", workspace),
				err,
			),
			err,
		)
	}

	if err := client.PostEvent(ctx, build.ID, eventRequest{
		RunnerID:        cfg.RunnerID,
		Type:            "runner.build.started",
		Message:         "Build runner started package-agent execution.",
		LifecycleStatus: "building",
		Payload: map[string]interface{}{
			"workspace": workspace,
			"gitUrl":    build.GitURL,
			"gitRef":    build.GitRef,
			"commitSha": commitSHA,
		},
	}); err != nil {
		return completeWithOriginalError(
			ctx,
			client,
			build.ID,
			preAgentFailureRequest(
				cfg.RunnerID,
				preAgentStartEventFailureClassification,
				"failed to post runner.build.started before package-agent execution",
				err,
			),
			err,
		)
	}

	runErr := RunPackageAgent(ctx, cfg.PackageAgentBin, inputPath, outputDir)
	report, reportErr := ReadAgentReport(outputDir)
	payload := map[string]interface{}{
		"workspace": workspace,
		"outputDir": outputDir,
		"commitSha": commitSHA,
	}
	if reportErr == nil {
		payload["report"] = report
	} else {
		payload["reportReadError"] = reportErr.Error()
	}

	if runErr != nil {
		payload["error"] = runErr.Error()
		event, request, returnErr := terminalPackageAgentOutcome(cfg.RunnerID, runErr, reportErr, payload["report"])
		event.Payload = payload
		postErr := client.PostEvent(ctx, build.ID, event)
		completeErr := client.Complete(ctx, build.ID, request)
		if postErr != nil && completeErr != nil {
			return fmt.Errorf(
				"package-agent error: %v; event post error: %v; complete error: %w",
				runErr,
				postErr,
				completeErr,
			)
		}
		if postErr != nil {
			return fmt.Errorf("package-agent error: %v; event post error: %w", runErr, postErr)
		}
		if completeErr != nil {
			return fmt.Errorf("package-agent error: %v; complete error: %w", runErr, completeErr)
		}
		return returnErr
	}

	if reportErr != nil {
		return completeWithOriginalError(
			ctx,
			client,
			build.ID,
			artifactUploadFailureCompletionRequest(cfg.RunnerID, fmt.Errorf("read report.json: %w", reportErr)),
			reportErr,
		)
	}

	if CompleteStatusFromReport(report.Status) != "succeeded" {
		request := needsHumanCompletionFromSuccessfulExit(cfg.RunnerID, report)
		event := eventRequest{
			RunnerID: cfg.RunnerID,
			Type:     "runner.build.needs_human",
			Message:  needsHumanEventMessage(request),
			Payload:  payload,
		}
		postErr := client.PostEvent(ctx, build.ID, event)
		completeErr := client.Complete(ctx, build.ID, request)
		if postErr != nil && completeErr != nil {
			return fmt.Errorf(
				"package-agent reported %q after successful exit; event post error: %v; complete error: %w",
				report.Status,
				postErr,
				completeErr,
			)
		}
		if postErr != nil {
			return fmt.Errorf(
				"package-agent reported %q after successful exit; event post error: %w",
				report.Status,
				postErr,
			)
		}
		if completeErr != nil {
			return fmt.Errorf(
				"package-agent reported %q after successful exit; complete error: %w",
				report.Status,
				completeErr,
			)
		}
		return nil
	}

	uploads, err := ArtifactUploadsFromReport(outputDir, report)
	if err != nil {
		return completeWithOriginalError(
			ctx,
			client,
			build.ID,
			artifactUploadFailureCompletionRequest(cfg.RunnerID, err),
			err,
		)
	}
	for _, upload := range uploads {
		if err := client.UploadArtifact(ctx, build.ID, cfg.RunnerID, upload.ArtifactType, upload.Path, upload.UploadName); err != nil {
			return completeWithOriginalError(
				ctx,
				client,
				build.ID,
				artifactUploadFailureCompletionRequest(cfg.RunnerID, err),
				err,
			)
		}
	}

	if err := client.PostEvent(ctx, build.ID, eventRequest{
		RunnerID:        cfg.RunnerID,
		Type:            "runner.build.finished",
		Message:         "package-agent finished.",
		LifecycleStatus: "building",
		Payload:         payload,
	}); err != nil {
		return completeWithOriginalError(
			ctx,
			client,
			build.ID,
			finishEventFailureCompletionRequest(cfg.RunnerID, err),
			err,
		)
	}

	return client.Complete(ctx, build.ID, successfulCompletionRequest(cfg.RunnerID, report))
}

func terminalPackageAgentOutcome(
	runnerID string,
	runErr error,
	reportErr error,
	reportValue interface{},
) (eventRequest, completeRequest, error) {
	if packageAgentExitCode(runErr) == 2 {
		request := needsHumanCompletionFromReport(runnerID, runErr, reportErr, reportValue)
		return eventRequest{
			RunnerID: runnerID,
			Type:     "runner.build.needs_human",
			Message:  needsHumanEventMessage(request),
		}, request, nil
	}

	request := failedCompletionRequest(runnerID, runErr, reportErr, reportValue)
	return eventRequest{
		RunnerID: runnerID,
		Type:     "runner.build.failed",
		Message:  fmt.Sprintf("package-agent failed: %v", runErr),
	}, request, runErr
}

func failedCompletionRequest(
	runnerID string,
	runErr error,
	reportErr error,
	reportValue interface{},
) completeRequest {
	request := completeRequest{
		RunnerID: runnerID,
		Status:   "failed",
	}
	if report, ok := reportValue.(AgentReport); ok {
		request.Note = report.Summary
		request.FailureClassification = report.Classification
		request.FailureSummary = report.Summary
		return request
	}

	request.Note = fmt.Sprintf("package-agent failed: %v", runErr)
	request.FailureClassification = "package_agent_failed"
	request.FailureSummary = request.Note
	if reportErr != nil {
		request.FailureSummary = fmt.Sprintf("%s; report unavailable: %v", request.FailureSummary, reportErr)
	}
	return request
}

func needsHumanCompletionFromReport(
	runnerID string,
	runErr error,
	reportErr error,
	reportValue interface{},
) completeRequest {
	request := completeRequest{
		RunnerID: runnerID,
		Status:   "needs_human",
	}
	if report, ok := reportValue.(AgentReport); ok {
		request.Note = report.Summary
		request.FailureClassification = report.Classification
		request.FailureSummary = report.Summary
		request.HumanAction = report.HumanAction
		if request.HumanAction == "" {
			request.HumanAction = "Inspect the package-agent report and complete the required manual build action."
		}
		return request
	}

	request.Note = fmt.Sprintf("package-agent requires human action: %v", runErr)
	request.FailureClassification = "package_agent_needs_human"
	request.FailureSummary = request.Note
	request.HumanAction = "Inspect the package-agent report and complete the required manual build action."
	if reportErr != nil {
		request.FailureSummary = fmt.Sprintf("%s; report unavailable: %v", request.FailureSummary, reportErr)
	}
	return request
}

func successfulCompletionRequest(runnerID string, report AgentReport) completeRequest {
	return completeRequest{
		RunnerID:    runnerID,
		Status:      "succeeded",
		Version:     report.Version,
		BuildNumber: report.BuildNumber,
		CommitSHA:   report.CommitSHA,
		Note:        report.Summary,
	}
}

func needsHumanCompletionFromSuccessfulExit(runnerID string, report AgentReport) completeRequest {
	request := completeRequest{
		RunnerID:              runnerID,
		Status:                "needs_human",
		Note:                  report.Summary,
		FailureClassification: report.Classification,
		FailureSummary:        report.Summary,
		HumanAction:           report.HumanAction,
	}
	if request.FailureClassification == "" {
		request.FailureClassification = "package_agent_success_exit_status_mismatch"
	}
	if request.FailureSummary == "" {
		request.FailureSummary = fmt.Sprintf("package-agent exited successfully but reported status %q", report.Status)
		request.Note = request.FailureSummary
	}
	if request.HumanAction == "" {
		request.HumanAction = "Inspect the package-agent report and resolve the build manually."
	}
	return request
}

func needsHumanEventMessage(request completeRequest) string {
	if request.HumanAction != "" {
		return request.HumanAction
	}
	if request.FailureSummary != "" {
		return request.FailureSummary
	}
	return "package-agent requires human action."
}

func preAgentFailureRequest(
	runnerID string,
	classification string,
	summary string,
	cause error,
) completeRequest {
	note := fmt.Sprintf("%s: %v", summary, cause)
	return completeRequest{
		RunnerID:              runnerID,
		Status:                "failed",
		Note:                  note,
		FailureClassification: classification,
		FailureSummary:        note,
	}
}

func finishEventFailureCompletionRequest(runnerID string, cause error) completeRequest {
	note := fmt.Sprintf("%s: %v", postAgentFinishEventFailureSummary, cause)
	return completeRequest{
		RunnerID:              runnerID,
		Status:                "needs_human",
		Note:                  note,
		FailureClassification: postAgentFinishEventFailureClassification,
		FailureSummary:        note,
		HumanAction:           postAgentFinishEventFailureAction,
	}
}

func artifactUploadFailureCompletionRequest(runnerID string, cause error) completeRequest {
	note := fmt.Sprintf("package-agent completed, but required artifact upload failed: %v", cause)
	return completeRequest{
		RunnerID:              runnerID,
		Status:                "needs_human",
		Note:                  note,
		FailureClassification: postAgentArtifactUploadFailureClassification,
		FailureSummary:        note,
		HumanAction:           postAgentArtifactUploadAction,
	}
}

func completeWithOriginalError(
	ctx context.Context,
	client *Client,
	buildID string,
	request completeRequest,
	originalErr error,
) error {
	if completeErr := client.Complete(ctx, buildID, request); completeErr != nil {
		return fmt.Errorf("original error: %v; complete error: %w", originalErr, completeErr)
	}
	return originalErr
}

func packageAgentExitCode(err error) int {
	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		return exitErr.ExitCode()
	}
	return -1
}
