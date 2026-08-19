package connector

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLoadSettingsFromFileMapsActiveConnectorConfig(t *testing.T) {
	path := filepath.Join(t.TempDir(), "config.json")
	data := []byte(`{
		"accountId": "account-a",
		"connectorToken": "token-a",
		"storeMode": "live",
		"centerUrl": "https://center.example.test/",
		"apple": {
			"issuerId": "issuer-a",
			"keyId": "key-a",
			"privateKeyPath": "C:\\ProgramData\\TestFlying\\secrets\\AuthKey_key-a.p8"
		},
		"google": {
			"serviceAccountJsonPath": "C:\\ProgramData\\TestFlying\\secrets\\service-account.json",
			"clientEmail": "robot@example.test",
			"privateKey": "-----BEGIN PRIVATE KEY-----\\nkey\\n-----END PRIVATE KEY-----"
		}
	}`)
	if err := os.WriteFile(path, data, 0o600); err != nil {
		t.Fatal(err)
	}

	settings, err := LoadSettingsFromFile(path, defaultSettings())
	if err != nil {
		t.Fatalf("LoadSettingsFromFile: %v", err)
	}

	if settings.DeveloperAccountID != "account-a" {
		t.Fatalf("DeveloperAccountID = %q", settings.DeveloperAccountID)
	}
	if settings.ConnectorToken != "token-a" {
		t.Fatalf("ConnectorToken = %q", settings.ConnectorToken)
	}
	if settings.StoreMode != StoreModeLive {
		t.Fatalf("StoreMode = %q", settings.StoreMode)
	}
	if settings.CenterURL != "https://center.example.test" {
		t.Fatalf("CenterURL = %q", settings.CenterURL)
	}
	if settings.AppleIssuerID != "issuer-a" || settings.AppleKeyID != "key-a" {
		t.Fatalf("Apple credentials not mapped: %#v", settings)
	}
	if settings.GoogleServiceAccountJSONPath == "" {
		t.Fatal("GoogleServiceAccountJSONPath is empty")
	}
	if settings.GoogleClientEmail != "robot@example.test" {
		t.Fatalf("GoogleClientEmail = %q", settings.GoogleClientEmail)
	}
	if settings.GooglePrivateKey == "" {
		t.Fatal("GooglePrivateKey is empty")
	}
}

func TestLoadSettingsUsesConfigPathAndAllowsEnvironmentOverride(t *testing.T) {
	path := filepath.Join(t.TempDir(), "config.json")
	if err := os.WriteFile(path, []byte(`{"accountId":"from-file","connectorToken":"file-token"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	t.Setenv("TESTFLYING_CONNECTOR_CONFIG_PATH", path)
	t.Setenv("TESTFLYING_CONNECTOR_DEVELOPER_ACCOUNT_ID", "from-env")

	settings, err := LoadSettings()
	if err != nil {
		t.Fatalf("LoadSettings: %v", err)
	}

	if settings.DeveloperAccountID != "from-env" {
		t.Fatalf("DeveloperAccountID = %q, want from-env", settings.DeveloperAccountID)
	}
	if settings.ConnectorToken != "file-token" {
		t.Fatalf("ConnectorToken = %q, want file-token", settings.ConnectorToken)
	}
}
