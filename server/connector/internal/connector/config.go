package connector

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

const (
	StoreModeMock = "mock"
	StoreModeLive = "live"
)

type Settings struct {
	DeveloperAccountID string
	ConnectorToken     string
	StoreMode          string
	ListenAddr         string
	CenterURL          string

	AppleIssuerID       string
	AppleKeyID          string
	ApplePrivateKeyPath string
	ApplePrivateKeyPEM  string
	AppleAPIBaseURL     string

	GoogleServiceAccountJSONPath string
	GoogleServiceAccountJSON     string
	GoogleClientEmail            string
	GooglePrivateKey             string
	GoogleDeveloperID            string
	GoogleTokenURL               string
	GoogleAPIBaseURL             string

	GoogleRateLimitMaxRequests int
	GoogleRateLimitWindow      time.Duration
	AppleRateLimitFallbackMax  int
	AppleRateLimitWindow       time.Duration
	AppleRateLimitSafetyRatio  float64
}

type FileConfig struct {
	AccountID          string `json:"accountId"`
	DeveloperAccountID string `json:"developerAccountId"`
	ConnectorToken     string `json:"connectorToken"`
	Token              string `json:"token"`
	StoreMode          string `json:"storeMode"`
	ListenAddr         string `json:"listenAddr"`
	CenterURL          string `json:"centerUrl"`
	Apple              struct {
		IssuerID       string `json:"issuerId"`
		KeyID          string `json:"keyId"`
		PrivateKeyPath string `json:"privateKeyPath"`
		PrivateKey     string `json:"privateKey"`
		APIBaseURL     string `json:"apiBaseUrl"`
	} `json:"apple"`
	Google struct {
		ServiceAccountJSONPath string `json:"serviceAccountJsonPath"`
		ServiceAccountJSON     string `json:"serviceAccountJson"`
		ClientEmail            string `json:"clientEmail"`
		ClientEmailSnake       string `json:"client_email"`
		PrivateKey             string `json:"privateKey"`
		PrivateKeySnake        string `json:"private_key"`
		DeveloperID            string `json:"developerId"`
		TokenURL               string `json:"tokenUrl"`
		APIBaseURL             string `json:"apiBaseUrl"`
	} `json:"google"`
}

func LoadSettings() (Settings, error) {
	settings := defaultSettings()
	configPath := connectorConfigPath()
	if configPath != "" {
		loaded, err := LoadSettingsFromFile(configPath, settings)
		if err != nil {
			return Settings{}, err
		}
		settings = loaded
	}
	applyEnvOverrides(&settings)
	return settings, nil
}

func LoadSettingsFromEnv() Settings {
	settings, err := LoadSettings()
	if err != nil {
		settings = defaultSettings()
		applyEnvOverrides(&settings)
	}
	return settings
}

func LoadSettingsFromFile(path string, base Settings) (Settings, error) {
	data, err := os.ReadFile(path) // noqa: G304 - connector config path is local operator configured.
	if err != nil {
		return Settings{}, err
	}
	var config FileConfig
	if err := json.Unmarshal(data, &config); err != nil {
		return Settings{}, err
	}
	settings := base
	if value := strings.TrimSpace(config.AccountID); value != "" {
		settings.DeveloperAccountID = value
	}
	if value := strings.TrimSpace(config.DeveloperAccountID); value != "" {
		settings.DeveloperAccountID = value
	}
	if value := strings.TrimSpace(config.ConnectorToken); value != "" {
		settings.ConnectorToken = value
	}
	if value := strings.TrimSpace(config.Token); value != "" {
		settings.ConnectorToken = value
	}
	if value := strings.TrimSpace(config.StoreMode); value != "" {
		settings.StoreMode = normalizeStoreMode(value)
	}
	if value := strings.TrimSpace(config.ListenAddr); value != "" {
		settings.ListenAddr = value
	}
	if value := strings.TrimSpace(config.CenterURL); value != "" {
		settings.CenterURL = strings.TrimRight(value, "/")
	}
	if value := strings.TrimSpace(config.Apple.IssuerID); value != "" {
		settings.AppleIssuerID = value
	}
	if value := strings.TrimSpace(config.Apple.KeyID); value != "" {
		settings.AppleKeyID = value
	}
	if value := strings.TrimSpace(config.Apple.PrivateKeyPath); value != "" {
		settings.ApplePrivateKeyPath = value
	}
	if value := strings.TrimSpace(config.Apple.PrivateKey); value != "" {
		settings.ApplePrivateKeyPEM = value
	}
	if value := strings.TrimSpace(config.Apple.APIBaseURL); value != "" {
		settings.AppleAPIBaseURL = strings.TrimRight(value, "/")
	}
	if value := strings.TrimSpace(config.Google.ServiceAccountJSONPath); value != "" {
		settings.GoogleServiceAccountJSONPath = value
	}
	if value := strings.TrimSpace(config.Google.ServiceAccountJSON); value != "" {
		settings.GoogleServiceAccountJSON = value
	}
	if value := firstNonEmpty(config.Google.ClientEmail, config.Google.ClientEmailSnake); value != "" {
		settings.GoogleClientEmail = value
	}
	if value := firstNonEmpty(config.Google.PrivateKey, config.Google.PrivateKeySnake); value != "" {
		settings.GooglePrivateKey = value
	}
	if value := strings.TrimSpace(config.Google.DeveloperID); value != "" {
		settings.GoogleDeveloperID = value
	}
	if value := strings.TrimSpace(config.Google.TokenURL); value != "" {
		settings.GoogleTokenURL = value
	}
	if value := strings.TrimSpace(config.Google.APIBaseURL); value != "" {
		settings.GoogleAPIBaseURL = strings.TrimRight(value, "/")
	}
	return settings, nil
}

func defaultSettings() Settings {
	return Settings{
		DeveloperAccountID: "account-apple-enterprise",
		ConnectorToken:     "dev-connector-token",
		StoreMode:          StoreModeMock,
		ListenAddr:         ":8100",
		AppleAPIBaseURL:    "https://api.appstoreconnect.apple.com",

		GoogleTokenURL:    "https://oauth2.googleapis.com/token",
		GoogleAPIBaseURL:  "https://androidpublisher.googleapis.com",
		GoogleDeveloperID: "",

		GoogleRateLimitMaxRequests: 200,
		GoogleRateLimitWindow:      time.Minute,
		AppleRateLimitFallbackMax:  2880,
		AppleRateLimitWindow:       time.Hour,
		AppleRateLimitSafetyRatio:  0.8,
	}
}

func applyEnvOverrides(settings *Settings) {
	overrideString(&settings.DeveloperAccountID, "TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID")
	overrideString(&settings.ConnectorToken, "TESTFLYING_CONNECTOR_TOKEN")
	if value := strings.TrimSpace(os.Getenv("TESTFLYING_CONNECTOR_STORE_MODE")); value != "" {
		settings.StoreMode = normalizeStoreMode(value)
	}
	overrideString(&settings.ListenAddr, "TESTFLYING_CONNECTOR_LISTEN_ADDR")
	overrideURL(&settings.CenterURL, "TESTFLYING_CONNECTOR_CENTER_URL")

	overrideString(&settings.AppleIssuerID, "TESTFLYING_CONNECTOR_APPLE_ISSUER_ID")
	overrideString(&settings.AppleKeyID, "TESTFLYING_CONNECTOR_APPLE_KEY_ID")
	overrideString(&settings.ApplePrivateKeyPath, "TESTFLYING_CONNECTOR_APPLE_PRIVATE_KEY_PATH")
	overrideString(&settings.ApplePrivateKeyPEM, "TESTFLYING_CONNECTOR_APPLE_PRIVATE_KEY")
	overrideURL(&settings.AppleAPIBaseURL, "TESTFLYING_CONNECTOR_APPLE_API_BASE_URL")

	overrideString(&settings.GoogleServiceAccountJSONPath, "TESTFLYING_CONNECTOR_GOOGLE_SERVICE_ACCOUNT_JSON_PATH")
	overrideString(&settings.GoogleServiceAccountJSON, "TESTFLYING_CONNECTOR_GOOGLE_SERVICE_ACCOUNT_JSON")
	overrideString(&settings.GoogleClientEmail, "TESTFLYING_CONNECTOR_GOOGLE_CLIENT_EMAIL")
	overrideString(&settings.GooglePrivateKey, "TESTFLYING_CONNECTOR_GOOGLE_PRIVATE_KEY")
	overrideString(&settings.GoogleDeveloperID, "TESTFLYING_CONNECTOR_GOOGLE_DEVELOPER_ID")
	overrideString(&settings.GoogleTokenURL, "TESTFLYING_CONNECTOR_GOOGLE_TOKEN_URL")
	overrideURL(&settings.GoogleAPIBaseURL, "TESTFLYING_CONNECTOR_GOOGLE_API_BASE_URL")

	settings.GoogleRateLimitMaxRequests = intFromEnv("TESTFLYING_CONNECTOR_GOOGLE_RATE_LIMIT_MAX_REQUESTS", settings.GoogleRateLimitMaxRequests)
	settings.GoogleRateLimitWindow = secondsFromEnv("TESTFLYING_CONNECTOR_GOOGLE_RATE_LIMIT_WINDOW_SECONDS", int(settings.GoogleRateLimitWindow.Seconds()))
	settings.AppleRateLimitFallbackMax = intFromEnv("TESTFLYING_CONNECTOR_APPLE_RATE_LIMIT_FALLBACK_MAX_REQUESTS", settings.AppleRateLimitFallbackMax)
	settings.AppleRateLimitWindow = secondsFromEnv("TESTFLYING_CONNECTOR_APPLE_RATE_LIMIT_WINDOW_SECONDS", int(settings.AppleRateLimitWindow.Seconds()))
	settings.AppleRateLimitSafetyRatio = floatFromEnv("TESTFLYING_CONNECTOR_APPLE_RATE_LIMIT_SAFETY_RATIO", settings.AppleRateLimitSafetyRatio)
}

func connectorConfigPath() string {
	if path := strings.TrimSpace(os.Getenv("TESTFLYING_CONNECTOR_CONFIG_PATH")); path != "" {
		return path
	}
	executable, err := os.Executable()
	if err != nil {
		return ""
	}
	candidate := filepath.Join(filepath.Dir(executable), "config.json")
	if _, err := os.Stat(candidate); err == nil {
		return candidate
	}
	return ""
}

func overrideString(target *string, envName string) {
	if value := strings.TrimSpace(os.Getenv(envName)); value != "" {
		*target = value
	}
}

func overrideURL(target *string, envName string) {
	if value := strings.TrimSpace(os.Getenv(envName)); value != "" {
		*target = strings.TrimRight(value, "/")
	}
}

func (s Settings) Validate() error {
	if strings.TrimSpace(s.DeveloperAccountID) == "" {
		return configError("TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID 不能为空")
	}
	if strings.TrimSpace(s.ConnectorToken) == "" {
		return configError("TESTFLYING_CONNECTOR_TOKEN 不能为空")
	}
	return nil
}

func env(name string, fallback string) string {
	value := strings.TrimSpace(os.Getenv(name))
	if value == "" {
		return fallback
	}
	return value
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if trimmed := strings.TrimSpace(value); trimmed != "" {
			return trimmed
		}
	}
	return ""
}

func normalizeStoreMode(value string) string {
	switch strings.ToLower(strings.TrimSpace(value)) {
	case StoreModeLive:
		return StoreModeLive
	default:
		return StoreModeMock
	}
}

func intFromEnv(name string, fallback int) int {
	raw := strings.TrimSpace(os.Getenv(name))
	if raw == "" {
		return fallback
	}
	value, err := strconv.Atoi(raw)
	if err != nil || value < 1 {
		return fallback
	}
	return value
}

func secondsFromEnv(name string, fallback int) time.Duration {
	return time.Duration(intFromEnv(name, fallback)) * time.Second
}

func floatFromEnv(name string, fallback float64) float64 {
	raw := strings.TrimSpace(os.Getenv(name))
	if raw == "" {
		return fallback
	}
	value, err := strconv.ParseFloat(raw, 64)
	if err != nil {
		return fallback
	}
	if value < 0.1 {
		return 0.1
	}
	if value > 1 {
		return 1
	}
	return value
}

type configError string

func (e configError) Error() string {
	return string(e)
}
