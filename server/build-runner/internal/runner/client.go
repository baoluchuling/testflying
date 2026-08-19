package runner

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"mime/multipart"
	"net/http"
	"os"
	"path/filepath"
	"strings"
)

type Client struct {
	baseURL    string
	token      string
	httpClient *http.Client
}

type heartbeatRequest struct {
	RunnerID            string                 `json:"runnerId"`
	Name                string                 `json:"name"`
	Labels              []string               `json:"labels"`
	Version             string                 `json:"version"`
	PackageAgentVersion string                 `json:"packageAgentVersion"`
	Capabilities        map[string]interface{} `json:"capabilities"`
}

type pollRequest struct {
	RunnerID       string  `json:"runnerId"`
	TimeoutSeconds float64 `json:"timeoutSeconds"`
}

type pollResponse struct {
	Build *BuildAssignment `json:"build"`
}

type UpdateCheckRequest struct {
	Platform            string `json:"platform"`
	Arch                string `json:"arch"`
	RunnerVersion       string `json:"runnerVersion"`
	PackageAgentVersion string `json:"packageAgentVersion"`
}

type UpdateManifest struct {
	Version             string `json:"version"`
	RunnerVersion       string `json:"runnerVersion"`
	PackageAgentVersion string `json:"packageAgentVersion"`
	BundleURL           string `json:"bundleUrl"`
	SHA256              string `json:"sha256"`
}

type updateCheckResponse struct {
	UpdateAvailable     bool   `json:"updateAvailable"`
	Version             string `json:"version"`
	RunnerVersion       string `json:"runnerVersion"`
	PackageAgentVersion string `json:"packageAgentVersion"`
	BundleURL           string `json:"bundleUrl"`
	SHA256              string `json:"sha256"`
}

type eventRequest struct {
	RunnerID        string                 `json:"runnerId"`
	Type            string                 `json:"type"`
	Message         string                 `json:"message"`
	LifecycleStatus string                 `json:"lifecycleStatus,omitempty"`
	Payload         map[string]interface{} `json:"payload,omitempty"`
}

type completeRequest struct {
	RunnerID              string `json:"runnerId"`
	Status                string `json:"status"`
	Version               string `json:"version,omitempty"`
	BuildNumber           string `json:"buildNumber,omitempty"`
	CommitSHA             string `json:"commitSha,omitempty"`
	Note                  string `json:"note,omitempty"`
	FailureClassification string `json:"failureClassification,omitempty"`
	FailureSummary        string `json:"failureSummary,omitempty"`
	HumanAction           string `json:"humanAction,omitempty"`
}

func NewClient(baseURL string, token string, httpClient *http.Client) *Client {
	if httpClient == nil {
		httpClient = http.DefaultClient
	}
	return &Client{
		baseURL:    strings.TrimRight(baseURL, "/"),
		token:      token,
		httpClient: httpClient,
	}
}

func (c *Client) Heartbeat(ctx context.Context, cfg Config) error {
	payload := heartbeatRequest{
		RunnerID:            cfg.RunnerID,
		Name:                cfg.Name,
		Labels:              cfg.Labels,
		Version:             cfg.Version,
		PackageAgentVersion: cfg.PackageAgentVersion,
		Capabilities: map[string]interface{}{
			"platforms":    []string{"ios", "android"},
			"llmAdapters":  cfg.LLMAdapters,
			"capacity":     cfg.Capacity,
			"hostPlatform": cfg.Platform,
			"arch":         cfg.Arch,
		},
	}
	return c.postJSON(ctx, "/admin/api/build-runners/heartbeat", payload, nil)
}

func (c *Client) Poll(ctx context.Context, runnerID string) (*BuildAssignment, error) {
	var response pollResponse
	if err := c.postJSON(ctx, "/admin/api/build-runners/poll", pollRequest{
		RunnerID:       runnerID,
		TimeoutSeconds: 0,
	}, &response); err != nil {
		return nil, err
	}
	return response.Build, nil
}

func (c *Client) CheckUpdate(
	ctx context.Context,
	runnerID string,
	payload UpdateCheckRequest,
) (*UpdateManifest, error) {
	var response updateCheckResponse
	path := fmt.Sprintf("/admin/api/build-runners/%s/updates/check", runnerID)
	if err := c.postJSON(ctx, path, payload, &response); err != nil {
		return nil, err
	}
	if !response.UpdateAvailable {
		return nil, nil
	}
	return &UpdateManifest{
		Version:             response.Version,
		RunnerVersion:       response.RunnerVersion,
		PackageAgentVersion: response.PackageAgentVersion,
		BundleURL:           response.BundleURL,
		SHA256:              response.SHA256,
	}, nil
}

func (c *Client) PostEvent(
	ctx context.Context,
	buildID string,
	request eventRequest,
) error {
	request.Message = RedactText(request.Message)
	if request.Payload != nil {
		request.Payload = RedactMap(request.Payload)
	}
	return c.postJSON(ctx, fmt.Sprintf("/admin/api/build-runners/builds/%s/events", buildID), request, nil)
}

func (c *Client) Complete(ctx context.Context, buildID string, request completeRequest) error {
	request.Note = RedactText(request.Note)
	request.FailureSummary = RedactText(request.FailureSummary)
	request.HumanAction = RedactText(request.HumanAction)
	return c.postJSON(ctx, fmt.Sprintf("/admin/api/build-runners/builds/%s/complete", buildID), request, nil)
}

func (c *Client) UploadArtifact(
	ctx context.Context,
	buildID string,
	runnerID string,
	artifactType string,
	filePath string,
	uploadName string,
) error {
	file, err := os.Open(filePath)
	if err != nil {
		return err
	}
	defer file.Close()

	var body bytes.Buffer
	writer := multipart.NewWriter(&body)
	if err := writer.WriteField("runnerId", runnerID); err != nil {
		return err
	}
	if err := writer.WriteField("artifactType", artifactType); err != nil {
		return err
	}
	fileName := uploadName
	if fileName == "" {
		fileName = filepath.Base(filePath)
	}
	part, err := writer.CreateFormFile("file", fileName)
	if err != nil {
		return err
	}
	if _, err := io.Copy(part, file); err != nil {
		return err
	}
	if err := writer.Close(); err != nil {
		return err
	}

	request, err := http.NewRequestWithContext(
		ctx,
		http.MethodPost,
		c.baseURL+fmt.Sprintf("/admin/api/build-runners/builds/%s/artifacts", buildID),
		&body,
	)
	if err != nil {
		return err
	}
	request.Header.Set("Authorization", "Bearer "+c.token)
	request.Header.Set("Content-Type", writer.FormDataContentType())

	response, err := c.httpClient.Do(request)
	if err != nil {
		return err
	}
	defer response.Body.Close()

	if response.StatusCode >= 300 {
		data, _ := io.ReadAll(io.LimitReader(response.Body, 4096))
		return fmt.Errorf(
			"request /admin/api/build-runners/builds/%s/artifacts failed: status=%d body=%s",
			buildID,
			response.StatusCode,
			strings.TrimSpace(string(data)),
		)
	}
	return nil
}

func (c *Client) download(ctx context.Context, path string) (io.ReadCloser, error) {
	if !strings.HasPrefix(path, "/") || strings.HasPrefix(path, "//") {
		return nil, fmt.Errorf("update bundle path must be relative to the server")
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, c.baseURL+path, nil)
	if err != nil {
		return nil, err
	}
	request.Header.Set("Authorization", "Bearer "+c.token)
	response, err := c.httpClient.Do(request)
	if err != nil {
		return nil, err
	}
	if response.StatusCode >= 300 {
		defer response.Body.Close()
		data, _ := io.ReadAll(io.LimitReader(response.Body, 4096))
		return nil, fmt.Errorf(
			"update bundle request failed: status=%d body=%s",
			response.StatusCode,
			strings.TrimSpace(string(data)),
		)
	}
	return response.Body, nil
}

func (c *Client) postJSON(ctx context.Context, path string, requestBody any, responseBody any) error {
	body, err := json.Marshal(requestBody)
	if err != nil {
		return err
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, c.baseURL+path, bytes.NewReader(body))
	if err != nil {
		return err
	}
	request.Header.Set("Authorization", "Bearer "+c.token)
	request.Header.Set("Content-Type", "application/json")

	response, err := c.httpClient.Do(request)
	if err != nil {
		return err
	}
	defer response.Body.Close()

	if response.StatusCode >= 300 {
		data, _ := io.ReadAll(io.LimitReader(response.Body, 4096))
		return fmt.Errorf("request %s failed: status=%d body=%s", path, response.StatusCode, strings.TrimSpace(string(data)))
	}
	if responseBody == nil {
		return nil
	}
	return json.NewDecoder(response.Body).Decode(responseBody)
}
