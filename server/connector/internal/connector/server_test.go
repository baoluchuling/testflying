package connector

import (
	"bytes"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"crypto/x509"
	"encoding/json"
	"encoding/pem"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"
)

func testSettings() Settings {
	return Settings{
		DeveloperAccountID:         "account-apple-enterprise",
		ConnectorToken:             "dev-connector-token",
		StoreMode:                  StoreModeMock,
		ListenAddr:                 ":8100",
		GoogleRateLimitMaxRequests: 200,
		GoogleRateLimitWindow:      time.Minute,
		AppleRateLimitFallbackMax:  2880,
		AppleRateLimitWindow:       time.Hour,
		AppleRateLimitSafetyRatio:  0.8,
		AppleAPIBaseURL:            "https://api.appstoreconnect.apple.com",
		GoogleTokenURL:             "https://oauth2.googleapis.com/token",
		GoogleAPIBaseURL:           "https://androidpublisher.googleapis.com",
	}
}

func testServer(t *testing.T, settings Settings) *Server {
	t.Helper()
	server, err := NewServer(settings)
	if err != nil {
		t.Fatalf("NewServer: %v", err)
	}
	return server
}

func testHeaders() http.Header {
	headers := http.Header{}
	headers.Set("Authorization", "Bearer dev-connector-token")
	headers.Set("Content-Type", "application/json")
	return headers
}

func testPayload(version string) map[string]any {
	return map[string]any{
		"developerAccountId": "account-apple-enterprise",
		"operation":          "update_release_notes",
		"platform":           "ios",
		"version":            version,
		"locale":             "zh-Hans",
		"app": map[string]any{
			"appId":            "app-aurora-ios",
			"bundleIdentifier": "com.internal.aurora",
			"storeAppId":       "1234567890",
			"packageName":      "com.internal.aurora",
		},
	}
}

func performJSON(server http.Handler, method string, path string, headers http.Header, payload any) *httptest.ResponseRecorder {
	var body bytes.Buffer
	if payload != nil {
		_ = json.NewEncoder(&body).Encode(payload)
	}
	request := httptest.NewRequest(method, path, &body)
	for key, values := range headers {
		for _, value := range values {
			request.Header.Add(key, value)
		}
	}
	response := httptest.NewRecorder()
	server.ServeHTTP(response, request)
	return response
}

func TestConnectorRequiresToken(t *testing.T) {
	server := testServer(t, testSettings())

	response := performJSON(server, http.MethodPost, "/v1/preflight", nil, testPayload("2.4.0"))

	if response.Code != http.StatusUnauthorized {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusUnauthorized)
	}
}

func TestConnectorRejectsOtherAccount(t *testing.T) {
	server := testServer(t, testSettings())
	payload := testPayload("2.4.0")
	payload["developerAccountId"] = "account-other"

	response := performJSON(server, http.MethodPost, "/v1/preflight", testHeaders(), payload)

	if response.Code != http.StatusForbidden {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusForbidden)
	}
}

func TestConnectorPreflightReportsMissingVersion(t *testing.T) {
	server := testServer(t, testSettings())

	response := performJSON(server, http.MethodPost, "/v1/preflight", testHeaders(), testPayload("missing-2.4.1"))

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var payload PreflightResponse
	if err := json.Unmarshal(response.Body.Bytes(), &payload); err != nil {
		t.Fatal(err)
	}
	if payload.CanSync {
		t.Fatal("canSync = true, want false")
	}
	if payload.ReasonCode == nil || *payload.ReasonCode != "store_version_missing" {
		t.Fatalf("reasonCode = %v, want store_version_missing", payload.ReasonCode)
	}
}

func TestConnectorSyncRunSucceeds(t *testing.T) {
	server := testServer(t, testSettings())
	payload := testPayload("2.4.0")
	payload["runId"] = "sync-001"
	payload["releaseNotes"] = "修复已知问题。"

	response := performJSON(server, http.MethodPost, "/v1/sync-runs", testHeaders(), payload)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var result SyncRunResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if result.Status != "succeeded" {
		t.Fatalf("sync status = %s, want succeeded", result.Status)
	}
}

func TestConnectorMetadataSyncRunSucceeds(t *testing.T) {
	server := testServer(t, testSettings())
	payload := testPayload("2.4.0")
	payload["operation"] = "update_app_metadata"
	payload["runId"] = "sync-metadata-001"
	payload["metadata"] = map[string]any{
		"contentSet": map[string]any{
			"id":   "summer-launch",
			"name": "暑期活动投放",
		},
		"keywords":        "internal,test",
		"promotionalText": "更稳定的测试体验。",
		"description":     "用于内部测试包分发和回归验证。",
		"storeImages": map[string]any{
			"feature_graphic_url": map[string]any{"urls": []string{}, "assets": []any{}},
			"phone_screenshots":   map[string]any{"urls": []string{}, "assets": []any{}},
			"tablet_screenshots":  map[string]any{"urls": []string{}, "assets": []any{}},
		},
	}

	response := performJSON(server, http.MethodPost, "/v1/sync-runs", testHeaders(), payload)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var result SyncRunResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if result.Status != "succeeded" || result.Message != "商店元数据已同步。" {
		t.Fatalf("result = %#v", result)
	}
}

func TestConnectorMetadataSyncRunSupportsImageOnlyScope(t *testing.T) {
	server := testServer(t, testSettings())
	payload := testPayload("2.4.0")
	payload["operation"] = "update_app_metadata"
	payload["runId"] = "sync-metadata-images"
	payload["syncScopes"] = []string{"store_images"}
	payload["metadata"] = map[string]any{
		"contentSet": map[string]any{
			"id":   "default",
			"name": "默认上架内容",
		},
		"storeImages": map[string]any{
			"feature_graphic_url": map[string]any{"urls": []string{}, "assets": []any{}},
			"phone_screenshots":   map[string]any{"urls": []string{}, "assets": []any{}},
			"tablet_screenshots":  map[string]any{"urls": []string{}, "assets": []any{}},
		},
	}

	response := performJSON(server, http.MethodPost, "/v1/sync-runs", testHeaders(), payload)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var result SyncRunResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if result.Status != "succeeded" {
		t.Fatalf("result = %#v", result)
	}
}

func TestGoogleCommitEditPathKeepsChangesOutOfReview(t *testing.T) {
	path := googleCommitEditPath("com.example.app", "edit 1")

	if !strings.HasPrefix(path, "/androidpublisher/v3/applications/com.example.app/edits/edit%201:commit?") {
		t.Fatalf("commit path = %s", path)
	}
	query := strings.TrimPrefix(path, "/androidpublisher/v3/applications/com.example.app/edits/edit%201:commit?")
	if !strings.Contains(query, "changesNotSentForReview=true") {
		t.Fatalf("commit query = %s, want changesNotSentForReview=true", query)
	}
	if !strings.Contains(query, "changesInReviewBehavior=ERROR_IF_IN_REVIEW") {
		t.Fatalf("commit query = %s, want ERROR_IF_IN_REVIEW", query)
	}
}

func TestConnectorListsStoreReleases(t *testing.T) {
	server := testServer(t, testSettings())

	response := performJSON(
		server,
		http.MethodGet,
		"/v1/apps/app-dataflow-android/store-releases?developerAccountId=account-apple-enterprise&platform=android&packageName=com.example.app",
		testHeaders(),
		nil,
	)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var result StoreReleasesResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Releases) != 2 || result.Releases[0].Track != "production" {
		t.Fatalf("store releases = %#v", result.Releases)
	}
}

func TestGoogleResolveReleaseTargetUsesLatestVersionCode(t *testing.T) {
	tracks := []googleTrack{
		{
			Track: "production",
			Releases: []googleRelease{
				{Name: "3.1.0", VersionCodes: []string{"310"}},
			},
		},
		{
			Track: "internal",
			Releases: []googleRelease{
				{Name: "3.2.0", VersionCodes: []string{"320"}},
			},
		},
	}

	target, err := googleResolveReleaseTarget(tracks, nil)

	if err != nil {
		t.Fatal(err)
	}
	if target.Track != "internal" || target.VersionCode != "320" || target.VersionName != "3.2.0" {
		t.Fatalf("target = %#v", target)
	}
}

func TestGoogleResolveReleaseTargetHonorsExplicitVersionCode(t *testing.T) {
	tracks := []googleTrack{
		{
			Track: "production",
			Releases: []googleRelease{
				{Name: "3.1.0", VersionCodes: []string{"310"}},
			},
		},
		{
			Track: "internal",
			Releases: []googleRelease{
				{Name: "3.2.0", VersionCodes: []string{"320"}},
			},
		},
	}

	target, err := googleResolveReleaseTarget(tracks, &StoreReleaseTarget{VersionCode: "310"})

	if err != nil {
		t.Fatal(err)
	}
	if target.Track != "production" || target.VersionCode != "310" || target.VersionName != "3.1.0" {
		t.Fatalf("target = %#v", target)
	}
}

func TestStoreMetadataScopeAttributesArePlatformSpecific(t *testing.T) {
	metadata := &StoreMetadata{
		Title:            "Readink",
		Keywords:         "books,novels",
		PromotionalText:  "Read anywhere.",
		ShortDescription: "Books and stories.",
		Description:      "Full store description.",
		FullDescription:  "Full Google Play description.",
		VideoURL:         "https://example.test/video",
	}

	appleAttrs := appleMetadataAttributes(metadata, []string{"description", "keywords"})
	googleAttrs := googleListingAttributes(metadata, []string{"short_description", "description", "video"})

	if appleAttrs["description"] != "Full store description." || appleAttrs["keywords"] != "books,novels" {
		t.Fatalf("apple attributes = %#v", appleAttrs)
	}
	if _, exists := appleAttrs["promotionalText"]; exists {
		t.Fatalf("apple attributes should not include promotionalText: %#v", appleAttrs)
	}
	if googleAttrs["shortDescription"] != "Books and stories." {
		t.Fatalf("google shortDescription = %#v", googleAttrs)
	}
	if googleAttrs["fullDescription"] != "Full Google Play description." {
		t.Fatalf("google fullDescription = %#v", googleAttrs)
	}
	if googleAttrs["video"] != "https://example.test/video" {
		t.Fatalf("google video = %#v", googleAttrs)
	}
}

func TestConnectorMarketingPageSyncRunSucceeds(t *testing.T) {
	server := testServer(t, testSettings())
	payload := testPayload("page-launch")
	payload["operation"] = "update_marketing_page"
	payload["runId"] = "sync-marketing-001"
	payload["syncScopes"] = []string{"marketing_text", "store_images"}
	payload["marketingPage"] = map[string]any{
		"pageId":          "page-launch",
		"pageName":        "冷启动投放页",
		"pageType":        "custom_product_page",
		"applePageId":     "",
		"deepLinkUrl":     "",
		"locale":          "en-US",
		"keywords":        "books,stories",
		"promotionalText": "Read stories anytime.",
		"storeImages": map[string]any{
			"feature_graphic_url": map[string]any{"urls": []string{}, "assets": []any{}},
			"phone_screenshots":   map[string]any{"urls": []string{}, "assets": []any{}},
			"tablet_screenshots":  map[string]any{"urls": []string{}, "assets": []any{}},
		},
	}

	response := performJSON(server, http.MethodPost, "/v1/sync-runs", testHeaders(), payload)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var result SyncRunResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if result.Status != "succeeded" || result.Message != "营销页面已同步。" {
		t.Fatalf("result = %#v", result)
	}
}

func TestConnectorListsSupportedLocales(t *testing.T) {
	server := testServer(t, testSettings())

	response := performJSON(
		server,
		http.MethodGet,
		"/v1/apps/app-aurora-ios/supported-locales?developerAccountId=account-apple-enterprise&platform=ios&version=2.4.0",
		testHeaders(),
		nil,
	)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var result SupportedLocalesResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	got := strings.Join(result.Locales, ",")
	want := "zh-Hans,en-US,ja,ko"
	if got != want {
		t.Fatalf("locales = %s, want %s", got, want)
	}
}

func TestConnectorListsStoreListings(t *testing.T) {
	server := testServer(t, testSettings())

	response := performJSON(
		server,
		http.MethodGet,
		"/v1/apps/app-aurora-ios/store-listings?developerAccountId=account-apple-enterprise&platform=ios&version=2.4.0",
		testHeaders(),
		nil,
	)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var result StoreListingsResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Listings) != 4 {
		t.Fatalf("listings = %d, want 4", len(result.Listings))
	}
	if result.Listings[0].Locale != "zh-Hans" || result.Listings[0].Description == "" {
		t.Fatalf("first listing = %#v", result.Listings[0])
	}
}

func TestConnectorListsStoreImages(t *testing.T) {
	server := testServer(t, testSettings())

	response := performJSON(
		server,
		http.MethodGet,
		"/v1/apps/app-aurora-ios/store-images?developerAccountId=account-apple-enterprise&platform=ios&version=2.4.0",
		testHeaders(),
		nil,
	)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var result StoreImagesResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Locales) != 4 {
		t.Fatalf("locales = %d, want 4", len(result.Locales))
	}
	phoneScreenshots := result.Locales[0].Images["phone_screenshots"]
	if len(phoneScreenshots) != 1 || phoneScreenshots[0].Width != 1290 {
		t.Fatalf("phone screenshots = %#v", phoneScreenshots)
	}
}

func TestConnectorListsStoreReviews(t *testing.T) {
	server := testServer(t, testSettings())

	response := performJSON(
		server,
		http.MethodGet,
		"/v1/apps/app-aurora-ios/store-reviews?developerAccountId=account-apple-enterprise&platform=ios&storeAppId=1234567890&pageSize=20",
		testHeaders(),
		nil,
	)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var result StoreReviewsResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Reviews) != 2 {
		t.Fatalf("reviews = %d, want 2", len(result.Reviews))
	}
	if result.Reviews[0].ID != "review-app-aurora-ios-1" || result.Reviews[0].Rating != 5 {
		t.Fatalf("first review = %#v", result.Reviews[0])
	}
}

func TestConnectorListsProductPageOptimizations(t *testing.T) {
	server := testServer(t, testSettings())

	response := performJSON(
		server,
		http.MethodGet,
		"/v1/apps/app-aurora-ios/product-page-optimizations?developerAccountId=account-apple-enterprise&platform=ios&storeAppId=1234567890",
		testHeaders(),
		nil,
	)

	if response.Code != http.StatusOK {
		t.Fatalf("status = %d, want %d", response.Code, http.StatusOK)
	}
	var result ProductPageOptimizationsResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if len(result.Experiments) != 1 {
		t.Fatalf("experiments = %d, want 1", len(result.Experiments))
	}
	experiment := result.Experiments[0]
	if experiment.State != "PREPARE_FOR_SUBMISSION" || experiment.TrafficProportion != 50 {
		t.Fatalf("experiment = %#v", experiment)
	}
	if len(experiment.Treatments) != 1 || experiment.Treatments[0].Name != "Variant A" {
		t.Fatalf("treatments = %#v", experiment.Treatments)
	}
}

func TestConnectorCreatesProductPageOptimization(t *testing.T) {
	server := testServer(t, testSettings())
	payload := map[string]any{
		"developerAccountId": "account-apple-enterprise",
		"platform":           "ios",
		"name":               "Summer Landing Test",
		"trafficProportion":  40,
		"locales":            []string{"en-US", "zh-Hant"},
		"app": map[string]any{
			"appId":            "app-aurora-ios",
			"bundleIdentifier": "com.internal.aurora",
			"storeAppId":       "1234567890",
		},
		"treatments": []map[string]any{
			{"name": "Variant A"},
			{"name": "Variant B", "locales": []string{"en-US"}},
		},
	}

	response := performJSON(
		server,
		http.MethodPost,
		"/v1/apps/app-aurora-ios/product-page-optimizations",
		testHeaders(),
		payload,
	)

	if response.Code != http.StatusCreated {
		t.Fatalf("status = %d, want %d body=%s", response.Code, http.StatusCreated, response.Body.String())
	}
	var result ProductPageOptimizationCreateResponse
	if err := json.Unmarshal(response.Body.Bytes(), &result); err != nil {
		t.Fatal(err)
	}
	if result.Experiment.Name != "Summer Landing Test" || result.Experiment.TrafficProportion != 40 {
		t.Fatalf("experiment = %#v", result.Experiment)
	}
	if len(result.Experiment.Treatments) != 2 {
		t.Fatalf("treatments = %d, want 2", len(result.Experiment.Treatments))
	}
	if strings.Join(result.Experiment.Treatments[0].Locales, ",") != "en-US,zh-Hant" {
		t.Fatalf("first locales = %#v", result.Experiment.Treatments[0].Locales)
	}
	if strings.Join(result.Experiment.Treatments[1].Locales, ",") != "en-US" {
		t.Fatalf("second locales = %#v", result.Experiment.Treatments[1].Locales)
	}
}

func TestConnectorRateLimitsGoogleRequests(t *testing.T) {
	settings := testSettings()
	settings.GoogleRateLimitMaxRequests = 2
	settings.GoogleRateLimitWindow = time.Minute
	server := testServer(t, settings)
	payload := testPayload("2.4.0")
	payload["platform"] = "android"
	payload["runId"] = "sync-rate-limit"
	payload["releaseNotes"] = "修复已知问题。"

	first := performJSON(server, http.MethodPost, "/v1/sync-runs", testHeaders(), payload)
	second := performJSON(server, http.MethodPost, "/v1/sync-runs", testHeaders(), payload)
	limited := performJSON(server, http.MethodPost, "/v1/sync-runs", testHeaders(), payload)
	health := performJSON(server, http.MethodGet, "/health", nil, nil)

	if first.Code != http.StatusOK || second.Code != http.StatusOK {
		t.Fatalf("first=%d second=%d, want both 200", first.Code, second.Code)
	}
	if limited.Code != http.StatusTooManyRequests {
		t.Fatalf("limited status = %d, want %d", limited.Code, http.StatusTooManyRequests)
	}
	if limited.Header().Get("Retry-After") == "" {
		t.Fatal("Retry-After header missing")
	}
	if health.Code != http.StatusOK {
		t.Fatalf("health status = %d, want 200", health.Code)
	}
}

func TestAppleRateLimitHeaderUsesSafetyMargin(t *testing.T) {
	settings := testSettings()
	settings.AppleRateLimitFallbackMax = 100
	settings.AppleRateLimitSafetyRatio = 0.8
	policy := NewStoreRateLimitPolicy(settings)

	policy.RecordAppleRateLimitHeader("user-hour-lim:10;user-hour-rem:8;")
	rule := policy.RuleForPlatform("ios")

	limit, ok := ParseAppleUserHourLimit("user-hour-lim:10;user-hour-rem:8;")
	if !ok || limit != 10 {
		t.Fatalf("limit=%d ok=%v, want 10 true", limit, ok)
	}
	if rule.MaxRequests != 8 {
		t.Fatalf("max requests = %d, want 8", rule.MaxRequests)
	}
	if rule.Window != time.Hour {
		t.Fatalf("window = %s, want 1h", rule.Window)
	}
}

func TestCredentialManagerValidatesAppleAndGoogleCredentials(t *testing.T) {
	applePath := writeAppleKey(t)
	googlePath := writeGoogleServiceAccount(t)
	settings := testSettings()
	settings.StoreMode = StoreModeLive
	settings.AppleIssuerID = "issuer-123"
	settings.AppleKeyID = "key-123"
	settings.ApplePrivateKeyPath = applePath
	settings.GoogleServiceAccountJSONPath = googlePath

	manager := NewCredentialManager(settings)
	status := manager.Status()
	appleToken, err := manager.AppleJWT(time.Unix(1_700_000_000, 0))
	if err != nil {
		t.Fatalf("AppleJWT: %v", err)
	}
	googleAccount, err := manager.googleServiceAccount()
	if err != nil {
		t.Fatalf("googleServiceAccount: %v", err)
	}
	googleAssertion, err := googleAccount.JWTAssertion(time.Unix(1_700_000_000, 0), settings.GoogleTokenURL)
	if err != nil {
		t.Fatalf("JWTAssertion: %v", err)
	}

	if !status["apple"].Valid || !status["google"].Valid {
		t.Fatalf("status = %#v", status)
	}
	if len(strings.Split(appleToken, ".")) != 3 {
		t.Fatalf("apple token = %s", appleToken)
	}
	if len(strings.Split(googleAssertion, ".")) != 3 {
		t.Fatalf("google assertion = %s", googleAssertion)
	}
}

func TestCredentialManagerAcceptsSplitGoogleCredentials(t *testing.T) {
	settings := testSettings()
	settings.StoreMode = StoreModeLive
	settings.GoogleClientEmail = "service-account@example.iam.gserviceaccount.com"
	settings.GooglePrivateKey = strings.ReplaceAll(googlePrivateKeyPEM(t), "\n", `\n`)

	manager := NewCredentialManager(settings)
	status := manager.GoogleStatus()
	account, err := manager.googleServiceAccount()
	if err != nil {
		t.Fatalf("googleServiceAccount: %v", err)
	}
	googleAssertion, err := account.JWTAssertion(time.Unix(1_700_000_000, 0), settings.GoogleTokenURL)
	if err != nil {
		t.Fatalf("JWTAssertion: %v", err)
	}

	if !status.Valid {
		t.Fatalf("status = %#v", status)
	}
	if len(strings.Split(googleAssertion, ".")) != 3 {
		t.Fatalf("google assertion = %s", googleAssertion)
	}
}

func writeAppleKey(t *testing.T) string {
	t.Helper()
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		t.Fatal(err)
	}
	return writePEM(t, "PRIVATE KEY", raw)
}

func writeGoogleServiceAccount(t *testing.T) string {
	t.Helper()
	privateKey := googlePrivateKeyPEM(t)
	payload := map[string]string{
		"client_email": "service-account@example.iam.gserviceaccount.com",
		"private_key":  privateKey,
		"token_uri":    "https://oauth2.googleapis.com/token",
	}
	data, err := json.Marshal(payload)
	if err != nil {
		t.Fatal(err)
	}
	file, err := os.CreateTemp(t.TempDir(), "service-account-*.json")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.Write(data); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	return file.Name()
}

func googlePrivateKeyPEM(t *testing.T) string {
	t.Helper()
	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		t.Fatal(err)
	}
	raw, err := x509.MarshalPKCS8PrivateKey(key)
	if err != nil {
		t.Fatal(err)
	}
	privateKey := pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: raw})
	return string(privateKey)
}

func writePEM(t *testing.T, blockType string, raw []byte) string {
	t.Helper()
	file, err := os.CreateTemp(t.TempDir(), "key-*.pem")
	if err != nil {
		t.Fatal(err)
	}
	if err := pem.Encode(file, &pem.Block{Type: blockType, Bytes: raw}); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	return file.Name()
}
