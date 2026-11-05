package connector

import (
	"encoding/json"
	"fmt"
	"net/http"
	"strconv"
	"strings"
)

type Server struct {
	settings    Settings
	credential  *CredentialManager
	rateLimiter *SlidingWindowRateLimiter
	ratePolicy  *StoreRateLimitPolicy
	store       StoreGateway
}

func NewServer(settings Settings) (*Server, error) {
	if err := settings.Validate(); err != nil {
		return nil, err
	}
	credential := NewCredentialManager(settings)
	ratePolicy := NewStoreRateLimitPolicy(settings)
	var store StoreGateway = MockStoreGateway{}
	if settings.StoreMode == StoreModeLive {
		store = NewLiveStoreGateway(settings, credential, ratePolicy)
	}
	return &Server{
		settings:    settings,
		credential:  credential,
		rateLimiter: NewSlidingWindowRateLimiter(),
		ratePolicy:  ratePolicy,
		store:       store,
	}, nil
}

func (s *Server) ServeHTTP(w http.ResponseWriter, r *http.Request) {
	switch {
	case r.Method == http.MethodGet && r.URL.Path == "/health":
		s.handleHealth(w, r)
	case r.Method == http.MethodPost && r.URL.Path == "/v1/preflight":
		s.withToken(w, r, s.handlePreflight)
	case r.Method == http.MethodPost && r.URL.Path == "/v1/sync-runs":
		s.withToken(w, r, s.handleSyncRun)
	case r.Method == http.MethodGet && strings.HasPrefix(r.URL.Path, "/v1/apps/") && strings.HasSuffix(r.URL.Path, "/supported-locales"):
		s.withToken(w, r, s.handleSupportedLocales)
	case r.Method == http.MethodGet && strings.HasPrefix(r.URL.Path, "/v1/apps/") && strings.HasSuffix(r.URL.Path, "/store-listings"):
		s.withToken(w, r, s.handleStoreListings)
	case r.Method == http.MethodGet && strings.HasPrefix(r.URL.Path, "/v1/apps/") && strings.HasSuffix(r.URL.Path, "/store-images"):
		s.withToken(w, r, s.handleStoreImages)
	case r.Method == http.MethodGet && strings.HasPrefix(r.URL.Path, "/v1/apps/") && strings.HasSuffix(r.URL.Path, "/store-releases"):
		s.withToken(w, r, s.handleStoreReleases)
	case r.Method == http.MethodGet && strings.HasPrefix(r.URL.Path, "/v1/apps/") && strings.HasSuffix(r.URL.Path, "/store-reviews"):
		s.withToken(w, r, s.handleStoreReviews)
	case r.Method == http.MethodGet && strings.HasPrefix(r.URL.Path, "/v1/apps/") && strings.HasSuffix(r.URL.Path, "/product-page-optimizations"):
		s.withToken(w, r, s.handleListProductPageOptimizations)
	case r.Method == http.MethodPost && strings.HasPrefix(r.URL.Path, "/v1/apps/") && strings.HasSuffix(r.URL.Path, "/product-page-optimizations"):
		s.withToken(w, r, s.handleCreateProductPageOptimization)
	default:
		writeError(w, http.StatusNotFound, "not found")
	}
}

func (s *Server) handleHealth(w http.ResponseWriter, _ *http.Request) {
	status := "ok"
	credentials := s.credential.Status()
	if s.settings.StoreMode == StoreModeLive && !s.credential.HasAnyLiveCredential() {
		status = "error"
	}
	writeJSON(w, http.StatusOK, HealthResponse{
		Status:             status,
		DeveloperAccountID: s.settings.DeveloperAccountID,
		Mode:               s.settings.StoreMode,
		Credentials:        credentials,
	})
}

func (s *Server) withToken(w http.ResponseWriter, r *http.Request, next func(http.ResponseWriter, *http.Request)) {
	expected := "Bearer " + s.settings.ConnectorToken
	if r.Header.Get("Authorization") != expected {
		writeError(w, http.StatusUnauthorized, "invalid connector token")
		return
	}
	next(w, r)
}

func (s *Server) handlePreflight(w http.ResponseWriter, r *http.Request) {
	var payload PreflightRequest
	if !decodeJSON(w, r, &payload) {
		return
	}
	if !s.validateAccount(w, payload.DeveloperAccountID) {
		return
	}
	if !s.enforceRateLimit(w, payload.Platform) {
		return
	}
	response, err := s.store.Preflight(r.Context(), payload)
	if err != nil {
		writeError(w, http.StatusBadGateway, friendlyStoreError(err))
		return
	}
	writeJSON(w, http.StatusOK, response)
}

func (s *Server) handleSupportedLocales(w http.ResponseWriter, r *http.Request) {
	accountID := r.URL.Query().Get("developerAccountId")
	if !s.validateAccount(w, accountID) {
		return
	}
	platform := r.URL.Query().Get("platform")
	if !s.enforceRateLimit(w, platform) {
		return
	}
	appID := appIDFromSupportedLocalesPath(r.URL.Path)
	response, err := s.store.SupportedLocales(
		r.Context(),
		appID,
		accountID,
		platform,
		r.URL.Query().Get("version"),
		r.URL.Query().Get("storeAppId"),
		r.URL.Query().Get("packageName"),
	)
	if err != nil {
		writeError(w, http.StatusBadGateway, friendlyStoreError(err))
		return
	}
	writeJSON(w, http.StatusOK, response)
}

func (s *Server) handleStoreListings(w http.ResponseWriter, r *http.Request) {
	accountID := r.URL.Query().Get("developerAccountId")
	if !s.validateAccount(w, accountID) {
		return
	}
	platform := r.URL.Query().Get("platform")
	if !s.enforceRateLimit(w, platform) {
		return
	}
	appID := appIDFromStoreResourcePath(r.URL.Path, "/store-listings")
	response, err := s.store.StoreListings(
		r.Context(),
		appID,
		accountID,
		platform,
		r.URL.Query().Get("version"),
		r.URL.Query().Get("storeAppId"),
		r.URL.Query().Get("packageName"),
	)
	if err != nil {
		writeError(w, http.StatusBadGateway, friendlyStoreError(err))
		return
	}
	writeJSON(w, http.StatusOK, response)
}

func (s *Server) handleStoreImages(w http.ResponseWriter, r *http.Request) {
	accountID := r.URL.Query().Get("developerAccountId")
	if !s.validateAccount(w, accountID) {
		return
	}
	platform := r.URL.Query().Get("platform")
	if !s.enforceRateLimit(w, platform) {
		return
	}
	appID := appIDFromStoreResourcePath(r.URL.Path, "/store-images")
	response, err := s.store.StoreImages(
		r.Context(),
		appID,
		accountID,
		platform,
		r.URL.Query().Get("version"),
		r.URL.Query().Get("storeAppId"),
		r.URL.Query().Get("packageName"),
	)
	if err != nil {
		writeError(w, http.StatusBadGateway, friendlyStoreError(err))
		return
	}
	writeJSON(w, http.StatusOK, response)
}

func (s *Server) handleStoreReleases(w http.ResponseWriter, r *http.Request) {
	accountID := r.URL.Query().Get("developerAccountId")
	if !s.validateAccount(w, accountID) {
		return
	}
	platform := r.URL.Query().Get("platform")
	if !s.enforceRateLimit(w, platform) {
		return
	}
	appID := appIDFromStoreResourcePath(r.URL.Path, "/store-releases")
	response, err := s.store.StoreReleases(
		r.Context(),
		appID,
		accountID,
		platform,
		r.URL.Query().Get("version"),
		r.URL.Query().Get("storeAppId"),
		r.URL.Query().Get("packageName"),
	)
	if err != nil {
		writeError(w, http.StatusBadGateway, friendlyStoreError(err))
		return
	}
	writeJSON(w, http.StatusOK, response)
}

func (s *Server) handleStoreReviews(w http.ResponseWriter, r *http.Request) {
	accountID := r.URL.Query().Get("developerAccountId")
	if !s.validateAccount(w, accountID) {
		return
	}
	platform := r.URL.Query().Get("platform")
	if !s.enforceRateLimit(w, platform) {
		return
	}
	appID := appIDFromStoreResourcePath(r.URL.Path, "/store-reviews")
	response, err := s.store.StoreReviews(
		r.Context(),
		appID,
		accountID,
		platform,
		r.URL.Query().Get("storeAppId"),
		r.URL.Query().Get("packageName"),
		storeReviewsQueryFromRequest(r),
	)
	if err != nil {
		writeError(w, http.StatusBadGateway, friendlyStoreError(err))
		return
	}
	writeJSON(w, http.StatusOK, response)
}

func (s *Server) handleSyncRun(w http.ResponseWriter, r *http.Request) {
	var payload SyncRunRequest
	if !decodeJSON(w, r, &payload) {
		return
	}
	if !s.validateAccount(w, payload.DeveloperAccountID) {
		return
	}
	if !s.enforceRateLimit(w, payload.Platform) {
		return
	}
	response, err := s.store.SyncRun(r.Context(), payload)
	if err != nil {
		writeError(w, http.StatusBadGateway, friendlyStoreError(err))
		return
	}
	writeJSON(w, http.StatusOK, response)
}

func (s *Server) handleListProductPageOptimizations(w http.ResponseWriter, r *http.Request) {
	accountID := r.URL.Query().Get("developerAccountId")
	if !s.validateAccount(w, accountID) {
		return
	}
	platform := r.URL.Query().Get("platform")
	if !s.enforceRateLimit(w, platform) {
		return
	}
	appID := appIDFromProductPageOptimizationsPath(r.URL.Path)
	response, err := s.store.ListProductPageOptimizations(
		r.Context(),
		appID,
		accountID,
		platform,
		r.URL.Query().Get("storeAppId"),
	)
	if err != nil {
		writeError(w, http.StatusBadGateway, friendlyStoreError(err))
		return
	}
	writeJSON(w, http.StatusOK, response)
}

func (s *Server) handleCreateProductPageOptimization(w http.ResponseWriter, r *http.Request) {
	var payload ProductPageOptimizationCreateRequest
	if !decodeJSON(w, r, &payload) {
		return
	}
	if !s.validateAccount(w, payload.DeveloperAccountID) {
		return
	}
	if !s.enforceRateLimit(w, payload.Platform) {
		return
	}
	appID := appIDFromProductPageOptimizationsPath(r.URL.Path)
	response, err := s.store.CreateProductPageOptimization(r.Context(), appID, payload)
	if err != nil {
		writeError(w, http.StatusBadGateway, friendlyStoreError(err))
		return
	}
	writeJSON(w, http.StatusCreated, response)
}

func (s *Server) validateAccount(w http.ResponseWriter, developerAccountID string) bool {
	if developerAccountID != s.settings.DeveloperAccountID {
		writeError(w, http.StatusForbidden, "developer account does not match this connector")
		return false
	}
	return true
}

func (s *Server) enforceRateLimit(w http.ResponseWriter, platform string) bool {
	decision := s.rateLimiter.Check(s.ratePolicy.RuleForPlatform(platform))
	if decision.Allowed {
		return true
	}
	retryAfter := int(decision.RetryAfter.Seconds())
	if retryAfter < 1 {
		retryAfter = 1
	}
	w.Header().Set("Retry-After", strconv.Itoa(retryAfter))
	writeError(w, http.StatusTooManyRequests, "connector rate limit exceeded")
	return false
}

func decodeJSON(w http.ResponseWriter, r *http.Request, target any) bool {
	defer r.Body.Close()
	decoder := json.NewDecoder(r.Body)
	decoder.DisallowUnknownFields()
	if err := decoder.Decode(target); err != nil {
		writeError(w, http.StatusBadRequest, "invalid json payload")
		return false
	}
	return true
}

func writeJSON(w http.ResponseWriter, statusCode int, payload any) {
	w.Header().Set("Content-Type", "application/json; charset=utf-8")
	w.WriteHeader(statusCode)
	_ = json.NewEncoder(w).Encode(payload)
}

func writeError(w http.ResponseWriter, statusCode int, detail string) {
	writeJSON(w, statusCode, ConnectorError{Detail: detail})
}

func friendlyStoreError(err error) string {
	return fmt.Sprintf("商店接口调用失败: %s", err.Error())
}

func appIDFromSupportedLocalesPath(path string) string {
	trimmed := strings.TrimPrefix(path, "/v1/apps/")
	trimmed = strings.TrimSuffix(trimmed, "/supported-locales")
	return strings.Trim(trimmed, "/")
}

func appIDFromProductPageOptimizationsPath(path string) string {
	trimmed := strings.TrimPrefix(path, "/v1/apps/")
	trimmed = strings.TrimSuffix(trimmed, "/product-page-optimizations")
	return strings.Trim(trimmed, "/")
}

func appIDFromStoreResourcePath(path string, suffix string) string {
	trimmed := strings.TrimPrefix(path, "/v1/apps/")
	trimmed = strings.TrimSuffix(trimmed, suffix)
	return strings.Trim(trimmed, "/")
}

func storeReviewsQueryFromRequest(r *http.Request) StoreReviewsQuery {
	pageSize, _ := strconv.Atoi(r.URL.Query().Get("pageSize"))
	return StoreReviewsQuery{
		PageSize:            pageSize,
		PageToken:           r.URL.Query().Get("pageToken"),
		StartIndex:          r.URL.Query().Get("startIndex"),
		TranslationLanguage: r.URL.Query().Get("translationLanguage"),
		Sort:                r.URL.Query().Get("sort"),
	}
}
