package connector

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"net/http/httptest"
	"strings"
	"time"
)

type AgentTask struct {
	ID      string            `json:"id"`
	Method  string            `json:"method"`
	Path    string            `json:"path"`
	Headers map[string]string `json:"headers"`
	Body    string            `json:"body"`
}

type agentPollResponse struct {
	Task *AgentTask `json:"task"`
}

type agentPollRequest struct {
	AccountID      string  `json:"accountId"`
	TimeoutSeconds float64 `json:"timeoutSeconds"`
}

type agentResultRequest struct {
	AccountID  string `json:"accountId"`
	TaskID     string `json:"taskId"`
	StatusCode int    `json:"statusCode"`
	Body       string `json:"body"`
}

func RunActiveAgent(ctx context.Context, settings Settings, server http.Handler) error {
	client := &http.Client{Timeout: 40 * time.Second}
	centerURL := strings.TrimRight(settings.CenterURL, "/")
	if centerURL == "" {
		return configError("TESTFLYING_CONNECTOR_CENTER_URL 不能为空")
	}
	for {
		select {
		case <-ctx.Done():
			return ctx.Err()
		default:
		}

		task, err := pollActiveTask(ctx, client, centerURL, settings)
		if err != nil {
			log.Printf("active connector poll failed: %v", err)
			sleepContext(ctx, 5*time.Second)
			continue
		}
		if task == nil {
			continue
		}

		statusCode, body := executeAgentTask(server, *task)
		if err := postActiveResult(ctx, client, centerURL, settings, task.ID, statusCode, body); err != nil {
			log.Printf("active connector result failed task=%s: %v", task.ID, err)
			sleepContext(ctx, 2*time.Second)
		}
	}
}

func executeAgentTask(server http.Handler, task AgentTask) (int, string) {
	method := strings.ToUpper(strings.TrimSpace(task.Method))
	if method == "" {
		method = http.MethodGet
	}
	request := httptest.NewRequest(method, task.Path, strings.NewReader(task.Body))
	for key, value := range task.Headers {
		request.Header.Set(key, value)
	}
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	return response.Code, response.Body.String()
}

func pollActiveTask(
	ctx context.Context,
	client *http.Client,
	centerURL string,
	settings Settings,
) (*AgentTask, error) {
	payload := agentPollRequest{
		AccountID:      settings.DeveloperAccountID,
		TimeoutSeconds: 25,
	}
	var response agentPollResponse
	if err := postCenterJSON(
		ctx,
		client,
		centerURL+"/connector-agent/v1/poll",
		settings.ConnectorToken,
		payload,
		&response,
	); err != nil {
		return nil, err
	}
	return response.Task, nil
}

func postActiveResult(
	ctx context.Context,
	client *http.Client,
	centerURL string,
	settings Settings,
	taskID string,
	statusCode int,
	body string,
) error {
	payload := agentResultRequest{
		AccountID:  settings.DeveloperAccountID,
		TaskID:     taskID,
		StatusCode: statusCode,
		Body:       body,
	}
	var response map[string]any
	return postCenterJSON(
		ctx,
		client,
		centerURL+"/connector-agent/v1/results",
		settings.ConnectorToken,
		payload,
		&response,
	)
}

func postCenterJSON(
	ctx context.Context,
	client *http.Client,
	url string,
	token string,
	payload any,
	target any,
) error {
	var body bytes.Buffer
	if err := json.NewEncoder(&body).Encode(payload); err != nil {
		return err
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, url, &body)
	if err != nil {
		return err
	}
	request.Header.Set("Authorization", "Bearer "+token)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Accept", "application/json")
	response, err := client.Do(request)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	data, err := io.ReadAll(response.Body)
	if err != nil {
		return err
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return fmt.Errorf("center returned HTTP %d: %s", response.StatusCode, strings.TrimSpace(string(data)))
	}
	if len(bytes.TrimSpace(data)) == 0 || target == nil {
		return nil
	}
	return json.Unmarshal(data, target)
}

func sleepContext(ctx context.Context, duration time.Duration) {
	timer := time.NewTimer(duration)
	defer timer.Stop()
	select {
	case <-ctx.Done():
	case <-timer.C:
	}
}
