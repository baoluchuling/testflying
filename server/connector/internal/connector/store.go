package connector

import (
	"bytes"
	"context"
	"crypto/md5"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"mime"
	"net/http"
	"net/url"
	"path"
	"sort"
	"strconv"
	"strings"
	"time"
)

const (
	syncScopeMetadata         = "metadata"
	syncScopeDescription      = "description"
	syncScopeKeywords         = "keywords"
	syncScopePromotionalText  = "promotional_text"
	syncScopeShortDescription = "short_description"
	syncScopeTitle            = "title"
	syncScopeVideo            = "video"
	syncScopeStoreImages      = "store_images"
	syncScopeMarketingText    = "marketing_text"
)

type StoreGateway interface {
	Preflight(ctx context.Context, payload PreflightRequest) (PreflightResponse, error)
	SupportedLocales(ctx context.Context, appID string, accountID string, platform string, version string, storeAppID string, packageName string) (SupportedLocalesResponse, error)
	StoreListings(ctx context.Context, appID string, accountID string, platform string, version string, storeAppID string, packageName string) (StoreListingsResponse, error)
	StoreImages(ctx context.Context, appID string, accountID string, platform string, version string, storeAppID string, packageName string) (StoreImagesResponse, error)
	StoreReleases(ctx context.Context, appID string, accountID string, platform string, version string, storeAppID string, packageName string) (StoreReleasesResponse, error)
	StoreReviews(ctx context.Context, appID string, accountID string, platform string, storeAppID string, packageName string, query StoreReviewsQuery) (StoreReviewsResponse, error)
	ListProductPageOptimizations(ctx context.Context, appID string, accountID string, platform string, storeAppID string) (ProductPageOptimizationsResponse, error)
	CreateProductPageOptimization(ctx context.Context, appID string, payload ProductPageOptimizationCreateRequest) (ProductPageOptimizationCreateResponse, error)
	SyncRun(ctx context.Context, payload SyncRunRequest) (SyncRunResponse, error)
}

type MockStoreGateway struct{}

func (g MockStoreGateway) Preflight(_ context.Context, payload PreflightRequest) (PreflightResponse, error) {
	operationLabel := operationLabel(payload.Operation)
	if payload.Operation == "update_marketing_page" {
		return PreflightResponse{
			CanSync:    true,
			ReasonCode: nil,
			Message:    "营销页面可同步；同步前会按勾选项提交文案或商店图。",
			StoreState: map[string]any{
				"versionExists": true,
				"editable":      true,
				"currentStatus": "marketing_page_editable",
			},
		}, nil
	}
	if strings.TrimSpace(payload.Version) == "" || strings.Contains(strings.ToLower(payload.Version), "missing") {
		return PreflightResponse{
			CanSync:    false,
			ReasonCode: stringPtr("store_version_missing"),
			Message:    fmt.Sprintf("商店中还没有创建 %s 版本，暂不能同步%s。", defaultString(payload.Version, "目标"), operationLabel),
			StoreState: map[string]any{
				"versionExists": false,
				"editable":      false,
				"currentStatus": "missing",
			},
		}, nil
	}
	return PreflightResponse{
		CanSync:    true,
		ReasonCode: nil,
		Message:    fmt.Sprintf("商店中已存在 %s 版本，当前状态允许修改%s。", payload.Version, operationLabel),
		StoreState: map[string]any{
			"versionExists": true,
			"editable":      true,
			"currentStatus": "prepare_for_submission",
		},
	}, nil
}

func (g MockStoreGateway) SupportedLocales(_ context.Context, _ string, _ string, platform string, _ string, _ string, _ string) (SupportedLocalesResponse, error) {
	if normalizePlatform(platform) == "ios" {
		return SupportedLocalesResponse{Locales: []string{"zh-Hans", "en-US", "ja", "ko"}}, nil
	}
	return SupportedLocalesResponse{Locales: []string{"zh-Hans", "en-US"}}, nil
}

func (g MockStoreGateway) StoreListings(_ context.Context, _ string, _ string, platform string, _ string, _ string, _ string) (StoreListingsResponse, error) {
	locales, _ := g.SupportedLocales(context.Background(), "", "", platform, "", "", "")
	listings := make([]StoreListing, 0, len(locales.Locales))
	for _, locale := range locales.Locales {
		listing := StoreListing{
			Locale:          locale,
			Description:     "Mock store description.",
			FullDescription: "Mock store description.",
			Keywords:        "mock,test",
			PromotionalText: "Mock promotional text.",
			ReleaseNotes:    "Mock release notes.",
		}
		if normalizePlatform(platform) == "android" {
			listing.Title = "Mock Android App"
			listing.ShortDescription = "Mock short description."
			listing.VideoURL = "https://example.test/video"
		}
		listings = append(listings, listing)
	}
	return StoreListingsResponse{Listings: listings}, nil
}

func (g MockStoreGateway) StoreImages(_ context.Context, _ string, _ string, platform string, _ string, _ string, _ string) (StoreImagesResponse, error) {
	locales, _ := g.SupportedLocales(context.Background(), "", "", platform, "", "", "")
	rows := make([]StoreImageLocale, 0, len(locales.Locales))
	for _, locale := range locales.Locales {
		rows = append(rows, StoreImageLocale{
			Locale: locale,
			Images: map[string][]StoreImageRef{
				"phone_screenshots": {
					{
						ID:       "mock-phone-1-" + locale,
						FileName: "phone-1.png",
						URL:      "https://cdn.example.test/" + locale + "/phone-1.png",
						Width:    1290,
						Height:   2796,
					},
				},
				"tablet_screenshots": {},
			},
		})
	}
	return StoreImagesResponse{Locales: rows}, nil
}

func (g MockStoreGateway) StoreReleases(_ context.Context, _ string, _ string, platform string, _ string, _ string, _ string) (StoreReleasesResponse, error) {
	if normalizePlatform(platform) != "android" {
		return StoreReleasesResponse{Releases: []StoreRelease{}}, nil
	}
	return StoreReleasesResponse{
		Releases: []StoreRelease{
			{
				Track:        "production",
				Name:         "3.1.0",
				Status:       "completed",
				VersionCodes: []string{"310"},
				ReleaseNotes: []StoreReleaseNote{
					{Language: "en-US", Text: "Mock release notes."},
				},
			},
			{
				Track:        "internal",
				Name:         "3.2.0",
				Status:       "draft",
				VersionCodes: []string{"320"},
			},
		},
	}, nil
}

func (g MockStoreGateway) StoreReviews(_ context.Context, appID string, _ string, platform string, _ string, _ string, _ StoreReviewsQuery) (StoreReviewsResponse, error) {
	normalizedPlatform := normalizePlatform(platform)
	if normalizedPlatform == "" {
		normalizedPlatform = "ios"
	}
	return StoreReviewsResponse{
		Reviews: []StoreReview{
			{
				ID:               "review-" + defaultString(appID, "mock") + "-1",
				Platform:         normalizedPlatform,
				Rating:           5,
				Title:            "Mock review",
				Body:             "The app works well for internal testing.",
				AuthorName:       "mock-user",
				ReviewerNickname: "mock-user",
				Locale:           "en-US",
				Territory:        "US",
				AppVersion:       "1.0.0",
				CreatedAt:        "2026-06-24T10:00:00Z",
				UpdatedAt:        "2026-06-24T10:00:00Z",
			},
			{
				ID:               "review-" + defaultString(appID, "mock") + "-2",
				Platform:         normalizedPlatform,
				Rating:           3,
				Title:            "Mock review old",
				Body:             "Needs more checks.",
				AuthorName:       "mock-user-2",
				ReviewerNickname: "mock-user-2",
				Locale:           "zh-Hans",
				Territory:        "CN",
				AppVersion:       "1.0.0",
				CreatedAt:        "2026-06-23T10:00:00Z",
				UpdatedAt:        "2026-06-23T10:00:00Z",
			},
		},
	}, nil
}

func (g MockStoreGateway) ListProductPageOptimizations(_ context.Context, appID string, _ string, platform string, _ string) (ProductPageOptimizationsResponse, error) {
	if normalizePlatform(platform) != "ios" {
		return ProductPageOptimizationsResponse{Experiments: []ProductPageOptimization{}}, nil
	}
	return ProductPageOptimizationsResponse{
		Experiments: []ProductPageOptimization{
			{
				ID:                "ppo-" + defaultString(appID, "mock"),
				Name:              "Mock Product Page Optimization",
				Platform:          "IOS",
				State:             "PREPARE_FOR_SUBMISSION",
				TrafficProportion: 50,
				ReviewRequired:    false,
				Treatments: []ProductPageOptimizationTreatment{
					{
						ID:      "treatment-mock-a",
						Name:    "Variant A",
						Locales: []string{"en-US", "zh-Hant"},
					},
				},
			},
		},
	}, nil
}

func (g MockStoreGateway) CreateProductPageOptimization(_ context.Context, appID string, payload ProductPageOptimizationCreateRequest) (ProductPageOptimizationCreateResponse, error) {
	if normalizePlatform(payload.Platform) != "ios" {
		return ProductPageOptimizationCreateResponse{}, errors.New("产品页面优化当前仅支持 App Store Connect")
	}
	if strings.TrimSpace(payload.Name) == "" {
		return ProductPageOptimizationCreateResponse{}, errors.New("产品页面优化名称不能为空")
	}
	treatments := payload.Treatments
	if len(treatments) == 0 {
		treatments = []ProductPageOptimizationTreatment{{Name: "Variant A"}}
	}
	resultTreatments := make([]ProductPageOptimizationTreatment, 0, len(treatments))
	for index, treatment := range treatments {
		locales := treatment.Locales
		if len(locales) == 0 {
			locales = payload.Locales
		}
		resultTreatments = append(resultTreatments, ProductPageOptimizationTreatment{
			ID:          fmt.Sprintf("treatment-%s-%d", defaultString(appID, "mock"), index+1),
			Name:        defaultString(treatment.Name, fmt.Sprintf("Variant %d", index+1)),
			AppIconName: treatment.AppIconName,
			Locales:     locales,
		})
	}
	return ProductPageOptimizationCreateResponse{
		Experiment: ProductPageOptimization{
			ID:                "ppo-created-" + defaultString(appID, "mock"),
			Name:              payload.Name,
			Platform:          "IOS",
			State:             "PREPARE_FOR_SUBMISSION",
			TrafficProportion: payload.TrafficProportion,
			ReviewRequired:    true,
			Treatments:        resultTreatments,
		},
	}, nil
}

func (g MockStoreGateway) SyncRun(_ context.Context, payload SyncRunRequest) (SyncRunResponse, error) {
	if payload.Operation == "update_marketing_page" {
		if payload.MarketingPage == nil {
			return failedSync("empty_marketing_page", "营销页面内容不能为空。"), nil
		}
		if strings.TrimSpace(payload.MarketingPage.PageName) == "" {
			return failedSync("empty_marketing_page_name", "营销页面名称不能为空。"), nil
		}
		return SyncRunResponse{Status: "succeeded", Message: "营销页面已同步。"}, nil
	}
	if payload.Operation == "update_app_metadata" {
		if payload.Metadata == nil {
			return failedSync("empty_metadata", "商店元数据不能为空。"), nil
		}
		scopes := syncScopeSet(payload.SyncScopes)
		hasTextScope := len(scopes) == 0 || scopes[syncScopeMetadata] || scopes[syncScopeDescription] ||
			scopes[syncScopeKeywords] || scopes[syncScopePromotionalText] || scopes[syncScopeShortDescription] ||
			scopes[syncScopeTitle] || scopes[syncScopeVideo]
		hasImages := scopes[syncScopeStoreImages] && payload.Metadata.StoreImages != nil
		if hasTextScope {
			hasText := len(appleMetadataAttributes(payload.Metadata, payload.SyncScopes)) > 0 ||
				len(googleListingAttributes(payload.Metadata, payload.SyncScopes)) > 0
			if !hasText && !hasImages {
				return failedSync("empty_metadata", "商店元数据内容不能为空。"), nil
			}
		}
		if !hasTextScope && !hasImages {
			return failedSync("empty_sync_payload", "没有可同步的商店内容。"), nil
		}
		return SyncRunResponse{Status: "succeeded", Message: "商店元数据已同步。"}, nil
	}
	if strings.TrimSpace(payload.ReleaseNotes) == "" {
		return failedSync("empty_release_notes", "版本说明不能为空。"), nil
	}
	return SyncRunResponse{Status: "succeeded", Message: "版本说明已同步。"}, nil
}

type LiveStoreGateway struct {
	settings   Settings
	credential *CredentialManager
	httpClient *http.Client
	ratePolicy *StoreRateLimitPolicy
}

func NewLiveStoreGateway(settings Settings, credential *CredentialManager, ratePolicy *StoreRateLimitPolicy) *LiveStoreGateway {
	return &LiveStoreGateway{
		settings:   settings,
		credential: credential,
		httpClient: &http.Client{Timeout: 20 * time.Second},
		ratePolicy: ratePolicy,
	}
}

func (g *LiveStoreGateway) Preflight(ctx context.Context, payload PreflightRequest) (PreflightResponse, error) {
	switch normalizePlatform(payload.Platform) {
	case "ios":
		return g.applePreflight(ctx, payload)
	case "android":
		return g.googlePreflight(ctx, payload)
	default:
		return PreflightResponse{
			CanSync:    false,
			ReasonCode: stringPtr("unsupported_platform"),
			Message:    "当前平台暂不支持商店同步。",
			StoreState: map[string]any{"platform": payload.Platform},
		}, nil
	}
}

func (g *LiveStoreGateway) SupportedLocales(ctx context.Context, appID string, accountID string, platform string, version string, storeAppID string, packageName string) (SupportedLocalesResponse, error) {
	switch normalizePlatform(platform) {
	case "ios":
		return g.appleSupportedLocales(ctx, storeAppID, version)
	case "android":
		return g.googleSupportedLocales(ctx, packageName)
	default:
		return SupportedLocalesResponse{Locales: []string{}}, nil
	}
}

func (g *LiveStoreGateway) StoreListings(ctx context.Context, appID string, accountID string, platform string, version string, storeAppID string, packageName string) (StoreListingsResponse, error) {
	switch normalizePlatform(platform) {
	case "ios":
		return g.appleStoreListings(ctx, storeAppID, version)
	case "android":
		return g.googleStoreListings(ctx, packageName)
	default:
		return StoreListingsResponse{Listings: []StoreListing{}}, nil
	}
}

func (g *LiveStoreGateway) StoreImages(ctx context.Context, appID string, accountID string, platform string, version string, storeAppID string, packageName string) (StoreImagesResponse, error) {
	switch normalizePlatform(platform) {
	case "ios":
		return g.appleStoreImages(ctx, storeAppID, version)
	case "android":
		return g.googleStoreImages(ctx, packageName)
	default:
		return StoreImagesResponse{Locales: []StoreImageLocale{}}, nil
	}
}

func (g *LiveStoreGateway) StoreReleases(ctx context.Context, _ string, _ string, platform string, _ string, _ string, packageName string) (StoreReleasesResponse, error) {
	switch normalizePlatform(platform) {
	case "android":
		return g.googleStoreReleases(ctx, packageName)
	default:
		return StoreReleasesResponse{Releases: []StoreRelease{}}, nil
	}
}

func (g *LiveStoreGateway) StoreReviews(ctx context.Context, _ string, _ string, platform string, storeAppID string, packageName string, query StoreReviewsQuery) (StoreReviewsResponse, error) {
	switch normalizePlatform(platform) {
	case "ios":
		return g.appleStoreReviews(ctx, storeAppID, query)
	case "android":
		return g.googleStoreReviews(ctx, packageName, query)
	default:
		return StoreReviewsResponse{Reviews: []StoreReview{}}, nil
	}
}

func (g *LiveStoreGateway) ListProductPageOptimizations(ctx context.Context, appID string, accountID string, platform string, storeAppID string) (ProductPageOptimizationsResponse, error) {
	switch normalizePlatform(platform) {
	case "ios":
		return g.appleProductPageOptimizations(ctx, storeAppID)
	default:
		return ProductPageOptimizationsResponse{}, errors.New("产品页面优化当前仅支持 App Store Connect")
	}
}

func (g *LiveStoreGateway) CreateProductPageOptimization(ctx context.Context, appID string, payload ProductPageOptimizationCreateRequest) (ProductPageOptimizationCreateResponse, error) {
	switch normalizePlatform(payload.Platform) {
	case "ios":
		return g.appleCreateProductPageOptimization(ctx, payload)
	default:
		return ProductPageOptimizationCreateResponse{}, errors.New("产品页面优化当前仅支持 App Store Connect")
	}
}

func (g *LiveStoreGateway) SyncRun(ctx context.Context, payload SyncRunRequest) (SyncRunResponse, error) {
	switch normalizePlatform(payload.Platform) {
	case "ios":
		return g.appleSyncRun(ctx, payload)
	case "android":
		return g.googleSyncRun(ctx, payload)
	default:
		return failedSync("unsupported_platform", "当前平台暂不支持商店同步。"), nil
	}
}

type appleVersion struct {
	ID         string
	State      string
	Attributes map[string]any
}

type appleLocalization struct {
	ID         string
	Locale     string
	Attributes map[string]any
}

type appleResource struct {
	ID         string `json:"id"`
	Type       string `json:"type"`
	Attributes struct {
		Name              string `json:"name"`
		Platform          string `json:"platform"`
		State             string `json:"state"`
		TrafficProportion int    `json:"trafficProportion"`
		ReviewRequired    bool   `json:"reviewRequired"`
		StartDate         string `json:"startDate"`
		EndDate           string `json:"endDate"`
		AppIconName       string `json:"appIconName"`
		Locale            string `json:"locale"`
	} `json:"attributes"`
	Relationships map[string]struct {
		Data []struct {
			ID   string `json:"id"`
			Type string `json:"type"`
		} `json:"data"`
	} `json:"relationships"`
}

type appleScreenshotSet struct {
	ID            string
	DisplayType   string
	ScreenshotIDs []string
}

type appleUploadOperation struct {
	Method         string `json:"method"`
	URL            string `json:"url"`
	Offset         int64  `json:"offset"`
	Length         int64  `json:"length"`
	RequestHeaders []struct {
		Name  string `json:"name"`
		Value string `json:"value"`
	} `json:"requestHeaders"`
}

type downloadedStoreImage struct {
	FileName    string
	ContentType string
	Body        []byte
}

func (g *LiveStoreGateway) applePreflight(ctx context.Context, payload PreflightRequest) (PreflightResponse, error) {
	if strings.TrimSpace(payload.App.StoreAppID) == "" {
		return PreflightResponse{
			CanSync:    false,
			ReasonCode: stringPtr("store_app_id_missing"),
			Message:    "iOS App 缺少 Apple App ID，暂不能同步。",
			StoreState: map[string]any{"versionExists": false, "editable": false},
		}, nil
	}
	if payload.Operation == "update_marketing_page" {
		return PreflightResponse{
			CanSync:    true,
			ReasonCode: nil,
			Message:    "App Store Connect App 已配置，营销页面可提交。",
			StoreState: map[string]any{
				"versionExists": true,
				"editable":      true,
				"currentStatus": "marketing_page_editable",
			},
		}, nil
	}
	version, err := g.appleFindVersion(ctx, payload.App.StoreAppID, payload.Version)
	if err != nil {
		return PreflightResponse{}, err
	}
	if version == nil {
		return PreflightResponse{
			CanSync:    false,
			ReasonCode: stringPtr("store_version_missing"),
			Message:    fmt.Sprintf("商店中还没有创建 %s 版本，暂不能同步%s。", defaultString(payload.Version, "目标"), operationLabel(payload.Operation)),
			StoreState: map[string]any{
				"versionExists": false,
				"editable":      false,
				"currentStatus": "missing",
			},
		}, nil
	}
	return PreflightResponse{
		CanSync:    true,
		ReasonCode: nil,
		Message:    fmt.Sprintf("商店中已存在 %s 版本，当前状态允许修改%s。", payload.Version, operationLabel(payload.Operation)),
		StoreState: map[string]any{
			"versionExists": true,
			"editable":      true,
			"currentStatus": strings.ToLower(version.State),
		},
	}, nil
}

func (g *LiveStoreGateway) appleSupportedLocales(ctx context.Context, storeAppID string, version string) (SupportedLocalesResponse, error) {
	if strings.TrimSpace(storeAppID) == "" {
		return SupportedLocalesResponse{Locales: []string{}}, nil
	}
	versionInfo, err := g.appleFindVersion(ctx, storeAppID, version)
	if err != nil {
		return SupportedLocalesResponse{}, err
	}
	if versionInfo != nil {
		locales, err := g.appleVersionLocalizations(ctx, versionInfo.ID)
		if err != nil {
			return SupportedLocalesResponse{}, err
		}
		return SupportedLocalesResponse{Locales: localizationLocales(locales)}, nil
	}
	locales, err := g.appleInfoLocalizations(ctx, storeAppID)
	if err != nil {
		return SupportedLocalesResponse{}, err
	}
	return SupportedLocalesResponse{Locales: localizationLocales(locales)}, nil
}

func (g *LiveStoreGateway) appleStoreListings(ctx context.Context, storeAppID string, version string) (StoreListingsResponse, error) {
	if strings.TrimSpace(storeAppID) == "" {
		return StoreListingsResponse{Listings: []StoreListing{}}, nil
	}
	infoLocalizations, err := g.appleInfoLocalizations(ctx, storeAppID)
	if err != nil {
		return StoreListingsResponse{}, err
	}
	infoByLocale := map[string]appleLocalization{}
	for _, item := range infoLocalizations {
		if strings.TrimSpace(item.Locale) != "" {
			infoByLocale[item.Locale] = item
		}
	}

	versionLocalizations := []appleLocalization{}
	if strings.TrimSpace(version) != "" {
		versionInfo, err := g.appleFindVersion(ctx, storeAppID, version)
		if err != nil {
			return StoreListingsResponse{}, err
		}
		if versionInfo != nil {
			versionLocalizations, err = g.appleVersionLocalizations(ctx, versionInfo.ID)
			if err != nil {
				return StoreListingsResponse{}, err
			}
		}
	}

	listingsByLocale := map[string]StoreListing{}
	for _, item := range infoLocalizations {
		listingsByLocale[item.Locale] = StoreListing{
			Locale:   item.Locale,
			Title:    stringAttribute(item.Attributes, "name"),
			Subtitle: stringAttribute(item.Attributes, "subtitle"),
		}
	}
	for _, item := range versionLocalizations {
		listing := listingsByLocale[item.Locale]
		listing.Locale = item.Locale
		if info, ok := infoByLocale[item.Locale]; ok {
			listing.Title = stringAttribute(info.Attributes, "name")
			listing.Subtitle = stringAttribute(info.Attributes, "subtitle")
		}
		listing.Description = stringAttribute(item.Attributes, "description")
		listing.FullDescription = listing.Description
		listing.Keywords = stringAttribute(item.Attributes, "keywords")
		listing.PromotionalText = stringAttribute(item.Attributes, "promotionalText")
		listing.ReleaseNotes = stringAttribute(item.Attributes, "whatsNew")
		listingsByLocale[item.Locale] = listing
	}
	return StoreListingsResponse{Listings: sortedStoreListings(listingsByLocale)}, nil
}

func (g *LiveStoreGateway) appleStoreImages(ctx context.Context, storeAppID string, version string) (StoreImagesResponse, error) {
	if strings.TrimSpace(storeAppID) == "" || strings.TrimSpace(version) == "" {
		return StoreImagesResponse{Locales: []StoreImageLocale{}}, nil
	}
	versionInfo, err := g.appleFindVersion(ctx, storeAppID, version)
	if err != nil {
		return StoreImagesResponse{}, err
	}
	if versionInfo == nil {
		return StoreImagesResponse{Locales: []StoreImageLocale{}}, nil
	}
	localizations, err := g.appleVersionLocalizations(ctx, versionInfo.ID)
	if err != nil {
		return StoreImagesResponse{}, err
	}
	locales := make([]StoreImageLocale, 0, len(localizations))
	for _, localization := range localizations {
		images, err := g.appleImagesForVersionLocalization(ctx, localization.ID)
		if err != nil {
			return StoreImagesResponse{}, err
		}
		locales = append(locales, StoreImageLocale{
			Locale: localization.Locale,
			Images: images,
		})
	}
	return StoreImagesResponse{Locales: locales}, nil
}

func (g *LiveStoreGateway) appleStoreReviews(ctx context.Context, storeAppID string, query StoreReviewsQuery) (StoreReviewsResponse, error) {
	if strings.TrimSpace(storeAppID) == "" {
		return StoreReviewsResponse{Reviews: []StoreReview{}}, nil
	}
	values := url.Values{}
	values.Set("limit", strconv.Itoa(clampStoreReviewPageSize(query.PageSize, 200)))
	if token := strings.TrimSpace(query.PageToken); token != "" {
		values.Set("cursor", token)
	}
	values.Set("sort", defaultString(strings.TrimSpace(query.Sort), "-createdDate"))
	var response struct {
		Data []struct {
			ID         string         `json:"id"`
			Attributes map[string]any `json:"attributes"`
		} `json:"data"`
		Links struct {
			Next string `json:"next"`
		} `json:"links"`
		Meta map[string]any `json:"meta"`
	}
	path := "/v1/apps/" + url.PathEscape(storeAppID) + "/customerReviews?" + values.Encode()
	if _, err := g.appleRequest(ctx, http.MethodGet, path, nil, &response); err != nil {
		return StoreReviewsResponse{}, err
	}
	reviews := make([]StoreReview, 0, len(response.Data))
	for _, item := range response.Data {
		reviews = append(reviews, StoreReview{
			ID:               item.ID,
			Platform:         "ios",
			Rating:           intAttribute(item.Attributes, "rating"),
			Title:            stringAttribute(item.Attributes, "title"),
			Body:             stringAttribute(item.Attributes, "body"),
			ReviewerNickname: stringAttribute(item.Attributes, "reviewerNickname"),
			AuthorName:       stringAttribute(item.Attributes, "reviewerNickname"),
			Locale:           stringAttribute(item.Attributes, "locale"),
			Territory:        stringAttribute(item.Attributes, "territory"),
			AppVersion:       defaultString(stringAttribute(item.Attributes, "appVersionString"), stringAttribute(item.Attributes, "appVersion")),
			CreatedAt:        stringAttribute(item.Attributes, "createdDate"),
			UpdatedAt:        stringAttribute(item.Attributes, "createdDate"),
			Raw:              item.Attributes,
		})
	}
	return StoreReviewsResponse{
		Reviews:       reviews,
		NextPageToken: appleNextPageToken(response.Links.Next),
		RawPageInfo:   response.Meta,
	}, nil
}

func (g *LiveStoreGateway) appleImagesForVersionLocalization(ctx context.Context, localizationID string) (map[string][]StoreImageRef, error) {
	query := url.Values{}
	query.Set("include", "appScreenshots")
	query.Set("limit", "200")
	query.Set("limit[appScreenshots]", "200")
	var response struct {
		Data []struct {
			ID            string         `json:"id"`
			Attributes    map[string]any `json:"attributes"`
			Relationships map[string]struct {
				Data []struct {
					ID   string `json:"id"`
					Type string `json:"type"`
				} `json:"data"`
			} `json:"relationships"`
		} `json:"data"`
		Included []struct {
			ID         string         `json:"id"`
			Type       string         `json:"type"`
			Attributes map[string]any `json:"attributes"`
		} `json:"included"`
	}
	path := "/v1/appStoreVersionLocalizations/" + url.PathEscape(localizationID) + "/appScreenshotSets?" + query.Encode()
	if _, err := g.appleRequest(ctx, http.MethodGet, path, nil, &response); err != nil {
		return nil, err
	}
	screenshotsByID := map[string]StoreImageRef{}
	for _, item := range response.Included {
		if item.Type != "appScreenshots" {
			continue
		}
		screenshotsByID[item.ID] = appleImageRefFromAttributes(item.ID, item.Attributes)
	}
	images := map[string][]StoreImageRef{
		"phone_screenshots":  {},
		"tablet_screenshots": {},
	}
	for _, set := range response.Data {
		slot := appleScreenshotSlot(stringAttribute(set.Attributes, "screenshotDisplayType"))
		if slot == "" {
			continue
		}
		relationship, ok := set.Relationships["appScreenshots"]
		if !ok {
			continue
		}
		for _, linkage := range relationship.Data {
			if image, exists := screenshotsByID[linkage.ID]; exists {
				images[slot] = append(images[slot], image)
			}
		}
	}
	return images, nil
}

func (g *LiveStoreGateway) appleProductPageOptimizations(ctx context.Context, storeAppID string) (ProductPageOptimizationsResponse, error) {
	if strings.TrimSpace(storeAppID) == "" {
		return ProductPageOptimizationsResponse{}, errors.New("iOS App 缺少 Apple App ID")
	}
	query := url.Values{}
	query.Set("include", "appStoreVersionExperimentTreatments")
	query.Set("limit", "200")
	query.Set("limit[appStoreVersionExperimentTreatments]", "50")
	var response struct {
		Data     []appleResource `json:"data"`
		Included []appleResource `json:"included"`
	}
	if _, err := g.appleRequest(ctx, http.MethodGet, "/v1/apps/"+url.PathEscape(storeAppID)+"/appStoreVersionExperimentsV2?"+query.Encode(), nil, &response); err != nil {
		return ProductPageOptimizationsResponse{}, err
	}
	treatmentsByID := map[string]ProductPageOptimizationTreatment{}
	for _, included := range response.Included {
		if included.Type != "appStoreVersionExperimentTreatments" {
			continue
		}
		treatmentsByID[included.ID] = ProductPageOptimizationTreatment{
			ID:          included.ID,
			Name:        included.Attributes.Name,
			AppIconName: included.Attributes.AppIconName,
		}
	}
	experiments := make([]ProductPageOptimization, 0, len(response.Data))
	for _, item := range response.Data {
		experiment := appleProductPageOptimizationFromResource(item)
		if relationship, ok := item.Relationships["appStoreVersionExperimentTreatments"]; ok {
			for _, linkage := range relationship.Data {
				if treatment, exists := treatmentsByID[linkage.ID]; exists {
					experiment.Treatments = append(experiment.Treatments, treatment)
				}
			}
		}
		experiments = append(experiments, experiment)
	}
	return ProductPageOptimizationsResponse{Experiments: experiments}, nil
}

func (g *LiveStoreGateway) appleCreateProductPageOptimization(ctx context.Context, payload ProductPageOptimizationCreateRequest) (ProductPageOptimizationCreateResponse, error) {
	storeAppID := strings.TrimSpace(payload.App.StoreAppID)
	if storeAppID == "" {
		return ProductPageOptimizationCreateResponse{}, errors.New("iOS App 缺少 Apple App ID")
	}
	name := strings.TrimSpace(payload.Name)
	if name == "" {
		return ProductPageOptimizationCreateResponse{}, errors.New("产品页面优化名称不能为空")
	}
	trafficProportion := payload.TrafficProportion
	if trafficProportion <= 0 || trafficProportion > 100 {
		return ProductPageOptimizationCreateResponse{}, errors.New("trafficProportion 必须在 1 到 100 之间")
	}
	createBody := map[string]any{
		"data": map[string]any{
			"type": "appStoreVersionExperiments",
			"attributes": map[string]any{
				"name":              name,
				"platform":          "IOS",
				"trafficProportion": trafficProportion,
			},
			"relationships": map[string]any{
				"app": map[string]any{
					"data": map[string]string{
						"type": "apps",
						"id":   storeAppID,
					},
				},
			},
		},
	}
	var createResponse struct {
		Data appleResource `json:"data"`
	}
	if _, err := g.appleRequest(ctx, http.MethodPost, "/v2/appStoreVersionExperiments", createBody, &createResponse); err != nil {
		return ProductPageOptimizationCreateResponse{}, err
	}
	experiment := appleProductPageOptimizationFromResource(createResponse.Data)
	if experiment.ID == "" {
		return ProductPageOptimizationCreateResponse{}, errors.New("App Store Connect 没有返回产品页面优化 ID")
	}
	treatments, err := g.appleCreateExperimentTreatments(ctx, experiment.ID, payload)
	if err != nil {
		return ProductPageOptimizationCreateResponse{}, err
	}
	experiment.Treatments = treatments
	return ProductPageOptimizationCreateResponse{Experiment: experiment}, nil
}

func (g *LiveStoreGateway) appleCreateExperimentTreatments(ctx context.Context, experimentID string, payload ProductPageOptimizationCreateRequest) ([]ProductPageOptimizationTreatment, error) {
	if len(payload.Treatments) == 0 {
		return []ProductPageOptimizationTreatment{}, nil
	}
	created := make([]ProductPageOptimizationTreatment, 0, len(payload.Treatments))
	for _, treatment := range payload.Treatments {
		name := strings.TrimSpace(treatment.Name)
		if name == "" {
			return nil, errors.New("treatment 名称不能为空")
		}
		attributes := map[string]any{"name": name}
		if strings.TrimSpace(treatment.AppIconName) != "" {
			attributes["appIconName"] = strings.TrimSpace(treatment.AppIconName)
		}
		body := map[string]any{
			"data": map[string]any{
				"type":       "appStoreVersionExperimentTreatments",
				"attributes": attributes,
				"relationships": map[string]any{
					"appStoreVersionExperimentV2": map[string]any{
						"data": map[string]string{
							"type": "appStoreVersionExperiments",
							"id":   experimentID,
						},
					},
				},
			},
		}
		var response struct {
			Data appleResource `json:"data"`
		}
		if _, err := g.appleRequest(ctx, http.MethodPost, "/v1/appStoreVersionExperimentTreatments", body, &response); err != nil {
			return nil, err
		}
		createdTreatment := ProductPageOptimizationTreatment{
			ID:          response.Data.ID,
			Name:        response.Data.Attributes.Name,
			AppIconName: response.Data.Attributes.AppIconName,
		}
		locales := treatment.Locales
		if len(locales) == 0 {
			locales = payload.Locales
		}
		for _, locale := range locales {
			normalizedLocale := strings.TrimSpace(locale)
			if normalizedLocale == "" {
				continue
			}
			if err := g.appleCreateExperimentTreatmentLocalization(ctx, createdTreatment.ID, normalizedLocale); err != nil {
				return nil, err
			}
			createdTreatment.Locales = append(createdTreatment.Locales, normalizedLocale)
		}
		created = append(created, createdTreatment)
	}
	return created, nil
}

func (g *LiveStoreGateway) appleCreateExperimentTreatmentLocalization(ctx context.Context, treatmentID string, locale string) error {
	body := map[string]any{
		"data": map[string]any{
			"type": "appStoreVersionExperimentTreatmentLocalizations",
			"attributes": map[string]any{
				"locale": locale,
			},
			"relationships": map[string]any{
				"appStoreVersionExperimentTreatment": map[string]any{
					"data": map[string]string{
						"type": "appStoreVersionExperimentTreatments",
						"id":   treatmentID,
					},
				},
			},
		},
	}
	_, err := g.appleRequest(ctx, http.MethodPost, "/v1/appStoreVersionExperimentTreatmentLocalizations", body, nil)
	return err
}

func (g *LiveStoreGateway) appleSyncRun(ctx context.Context, payload SyncRunRequest) (SyncRunResponse, error) {
	if payload.Operation == "update_marketing_page" {
		return g.appleSyncMarketingPage(ctx, payload)
	}
	version, err := g.appleFindVersion(ctx, payload.App.StoreAppID, payload.Version)
	if err != nil {
		return SyncRunResponse{}, err
	}
	if version == nil {
		return failedSync("store_version_missing", "商店中还没有创建目标版本。"), nil
	}
	versionLocalizations, err := g.appleVersionLocalizations(ctx, version.ID)
	if err != nil {
		return SyncRunResponse{}, err
	}
	versionLocalization := findLocalization(versionLocalizations, payload.Locale)
	if versionLocalization == nil {
		return failedSync("store_locale_missing", "商店中还没有创建目标语言。"), nil
	}
	if payload.Operation == "update_app_metadata" {
		if payload.Metadata == nil {
			return failedSync("empty_metadata", "商店元数据不能为空。"), nil
		}
		if attributes := appleMetadataAttributes(payload.Metadata, payload.SyncScopes); len(attributes) > 0 {
			if err := g.applePatchVersionLocalization(ctx, versionLocalization.ID, attributes); err != nil {
				return SyncRunResponse{}, err
			}
		}
		if shouldSyncStoreImages(payload.SyncScopes) && payload.Metadata.StoreImages != nil {
			if err := g.appleSyncStoreImages(ctx, "appStoreVersionLocalization", versionLocalization.ID, payload.Metadata.StoreImages); err != nil {
				return SyncRunResponse{}, err
			}
		}
		return SyncRunResponse{Status: "succeeded", Message: "商店元数据已同步。"}, nil
	}
	if strings.TrimSpace(payload.ReleaseNotes) == "" {
		return failedSync("empty_release_notes", "版本说明不能为空。"), nil
	}
	if err := g.applePatchVersionLocalization(ctx, versionLocalization.ID, map[string]string{
		"whatsNew": payload.ReleaseNotes,
	}); err != nil {
		return SyncRunResponse{}, err
	}
	return SyncRunResponse{Status: "succeeded", Message: "版本说明已同步。"}, nil
}

func (g *LiveStoreGateway) appleSyncMarketingPage(ctx context.Context, payload SyncRunRequest) (SyncRunResponse, error) {
	if payload.MarketingPage == nil {
		return failedSync("empty_marketing_page", "营销页面内容不能为空。"), nil
	}
	if strings.TrimSpace(payload.MarketingPage.PageName) == "" {
		return failedSync("empty_marketing_page_name", "营销页面名称不能为空。"), nil
	}
	pageID, localizationID, created, err := g.appleEnsureCustomProductPageLocalization(ctx, payload)
	if err != nil {
		return SyncRunResponse{}, err
	}
	scopes := syncScopeSet(payload.SyncScopes)
	if len(scopes) == 0 || scopes[syncScopeMarketingText] {
		attributes := map[string]string{"promotionalText": payload.MarketingPage.PromotionalText}
		if err := g.applePatchCustomProductPageLocalization(ctx, localizationID, attributes); err != nil {
			return SyncRunResponse{}, err
		}
	}
	if scopes[syncScopeStoreImages] && payload.MarketingPage.StoreImages != nil {
		if err := g.appleSyncStoreImages(ctx, "appCustomProductPageLocalization", localizationID, payload.MarketingPage.StoreImages); err != nil {
			return SyncRunResponse{}, err
		}
	}
	return SyncRunResponse{
		Status:  "succeeded",
		Message: "营销页面已同步。",
		StoreState: map[string]any{
			"applePageId": pageID,
			"created":     created,
		},
	}, nil
}

func appleMetadataAttributes(metadata *StoreMetadata, scopes []string) map[string]string {
	scopeSet := syncScopeSet(scopes)
	allText := len(scopeSet) == 0 || scopeSet[syncScopeMetadata]
	attributes := map[string]string{}
	if allText || scopeSet[syncScopeDescription] {
		attributes["description"] = metadata.Description
	}
	if allText || scopeSet[syncScopeKeywords] {
		attributes["keywords"] = metadata.Keywords
	}
	if allText || scopeSet[syncScopePromotionalText] {
		attributes["promotionalText"] = metadata.PromotionalText
	}
	return cleanStringMap(attributes)
}

func googleListingAttributes(metadata *StoreMetadata, scopes []string) map[string]string {
	scopeSet := syncScopeSet(scopes)
	allText := len(scopeSet) == 0 || scopeSet[syncScopeMetadata]
	attributes := map[string]string{}
	if allText || scopeSet[syncScopeTitle] {
		attributes["title"] = metadata.Title
	}
	if allText || scopeSet[syncScopeDescription] {
		attributes["fullDescription"] = defaultString(metadata.FullDescription, metadata.Description)
	}
	if allText || scopeSet[syncScopePromotionalText] || scopeSet[syncScopeShortDescription] {
		attributes["shortDescription"] = defaultString(metadata.ShortDescription, metadata.PromotionalText)
	}
	if allText || scopeSet[syncScopeVideo] {
		attributes["video"] = metadata.VideoURL
	}
	return cleanStringMap(attributes)
}

func syncScopeSet(scopes []string) map[string]bool {
	result := map[string]bool{}
	for _, scope := range scopes {
		normalized := strings.TrimSpace(scope)
		if normalized != "" {
			result[normalized] = true
		}
	}
	return result
}

func shouldSyncStoreImages(scopes []string) bool {
	return syncScopeSet(scopes)[syncScopeStoreImages]
}

func cleanStringMap(values map[string]string) map[string]string {
	cleaned := map[string]string{}
	for key, value := range values {
		if strings.TrimSpace(value) != "" {
			cleaned[key] = value
		}
	}
	return cleaned
}

func (g *LiveStoreGateway) appleFindVersion(ctx context.Context, storeAppID string, version string) (*appleVersion, error) {
	if strings.TrimSpace(version) == "" {
		return nil, nil
	}
	query := url.Values{}
	query.Set("filter[versionString]", version)
	query.Set("limit", "10")
	var response struct {
		Data []struct {
			ID         string         `json:"id"`
			Attributes map[string]any `json:"attributes"`
		} `json:"data"`
	}
	if _, err := g.appleRequest(ctx, http.MethodGet, "/v1/apps/"+url.PathEscape(storeAppID)+"/appStoreVersions?"+query.Encode(), nil, &response); err != nil {
		return nil, err
	}
	for _, item := range response.Data {
		state, _ := item.Attributes["appStoreState"].(string)
		return &appleVersion{ID: item.ID, State: state, Attributes: item.Attributes}, nil
	}
	return nil, nil
}

func (g *LiveStoreGateway) appleVersionLocalizations(ctx context.Context, versionID string) ([]appleLocalization, error) {
	var response struct {
		Data []struct {
			ID         string         `json:"id"`
			Attributes map[string]any `json:"attributes"`
		} `json:"data"`
	}
	if _, err := g.appleRequest(ctx, http.MethodGet, "/v1/appStoreVersions/"+url.PathEscape(versionID)+"/appStoreVersionLocalizations?limit=200", nil, &response); err != nil {
		return nil, err
	}
	return appleLocalizationsFromResponse(response.Data), nil
}

func (g *LiveStoreGateway) appleInfoLocalizations(ctx context.Context, storeAppID string) ([]appleLocalization, error) {
	var infos struct {
		Data []struct {
			ID string `json:"id"`
		} `json:"data"`
	}
	if _, err := g.appleRequest(ctx, http.MethodGet, "/v1/apps/"+url.PathEscape(storeAppID)+"/appInfos?limit=1", nil, &infos); err != nil {
		return nil, err
	}
	if len(infos.Data) == 0 {
		return nil, nil
	}
	var response struct {
		Data []struct {
			ID         string         `json:"id"`
			Attributes map[string]any `json:"attributes"`
		} `json:"data"`
	}
	if _, err := g.appleRequest(ctx, http.MethodGet, "/v1/appInfos/"+url.PathEscape(infos.Data[0].ID)+"/appInfoLocalizations?limit=200", nil, &response); err != nil {
		return nil, err
	}
	return appleLocalizationsFromResponse(response.Data), nil
}

func (g *LiveStoreGateway) appleEnsureCustomProductPageLocalization(ctx context.Context, payload SyncRunRequest) (string, string, bool, error) {
	page := payload.MarketingPage
	if page == nil {
		return "", "", false, errors.New("营销页面内容不能为空")
	}
	locale := defaultString(page.Locale, payload.Locale)
	pageID := strings.TrimSpace(page.ApplePageID)
	if pageID == "" {
		createdPageID, localizationID, err := g.appleCreateCustomProductPage(ctx, payload, locale)
		return createdPageID, localizationID, true, err
	}
	versionID, err := g.appleFirstCustomProductPageVersion(ctx, pageID)
	if err != nil {
		return "", "", false, err
	}
	if strings.TrimSpace(versionID) == "" {
		return "", "", false, errors.New("App Store Connect 没有返回自定义产品页面版本")
	}
	localization, err := g.appleFindCustomProductPageLocalization(ctx, versionID, locale)
	if err != nil {
		return "", "", false, err
	}
	if localization != nil {
		return pageID, localization.ID, false, nil
	}
	createdLocalization, err := g.appleCreateCustomProductPageLocalization(ctx, versionID, locale, page.PromotionalText)
	if err != nil {
		return "", "", false, err
	}
	return pageID, createdLocalization.ID, false, nil
}

func (g *LiveStoreGateway) appleCreateCustomProductPage(ctx context.Context, payload SyncRunRequest, locale string) (string, string, error) {
	storeAppID := strings.TrimSpace(payload.App.StoreAppID)
	if storeAppID == "" {
		return "", "", errors.New("iOS App 缺少 Apple App ID")
	}
	templateVersion, err := g.appleLatestVersion(ctx, storeAppID)
	if err != nil {
		return "", "", err
	}
	if templateVersion == nil {
		return "", "", errors.New("创建自定义产品页面需要 App Store Connect 中至少存在一个商店版本")
	}
	pageName := strings.TrimSpace(payload.MarketingPage.PageName)
	refSeed := defaultString(payload.RunID, "custom-product-page")
	versionRef := "version-" + refSeed
	localizationRef := "localization-" + refSeed
	localizationAttributes := map[string]string{"locale": locale}
	if strings.TrimSpace(payload.MarketingPage.PromotionalText) != "" {
		localizationAttributes["promotionalText"] = payload.MarketingPage.PromotionalText
	}
	body := map[string]any{
		"data": map[string]any{
			"type": "appCustomProductPages",
			"attributes": map[string]any{
				"name": pageName,
			},
			"relationships": map[string]any{
				"app": map[string]any{
					"data": map[string]string{"type": "apps", "id": storeAppID},
				},
				"appStoreVersionTemplate": map[string]any{
					"data": map[string]string{"type": "appStoreVersions", "id": templateVersion.ID},
				},
				"appCustomProductPageVersions": map[string]any{
					"data": []map[string]string{
						{"type": "appCustomProductPageVersions", "id": versionRef},
					},
				},
			},
		},
		"included": []map[string]any{
			{
				"type": "appCustomProductPageVersions",
				"id":   versionRef,
				"relationships": map[string]any{
					"appCustomProductPageLocalizations": map[string]any{
						"data": []map[string]string{
							{"type": "appCustomProductPageLocalizations", "id": localizationRef},
						},
					},
				},
			},
			{
				"type":       "appCustomProductPageLocalizations",
				"id":         localizationRef,
				"attributes": localizationAttributes,
			},
		},
	}
	var response struct {
		Data     appleResource   `json:"data"`
		Included []appleResource `json:"included"`
	}
	if _, err := g.appleRequest(ctx, http.MethodPost, "/v1/appCustomProductPages", body, &response); err != nil {
		return "", "", err
	}
	pageID := response.Data.ID
	localizationID := ""
	for _, item := range response.Included {
		if item.Type == "appCustomProductPageLocalizations" {
			localizationID = item.ID
			break
		}
	}
	if pageID == "" {
		return "", "", errors.New("App Store Connect 没有返回自定义产品页面 ID")
	}
	if localizationID != "" {
		return pageID, localizationID, nil
	}
	versionID, err := g.appleFirstCustomProductPageVersion(ctx, pageID)
	if err != nil {
		return "", "", err
	}
	localization, err := g.appleFindCustomProductPageLocalization(ctx, versionID, locale)
	if err != nil {
		return "", "", err
	}
	if localization == nil {
		return "", "", errors.New("App Store Connect 没有返回自定义产品页面语言")
	}
	return pageID, localization.ID, nil
}

func (g *LiveStoreGateway) appleLatestVersion(ctx context.Context, storeAppID string) (*appleVersion, error) {
	query := url.Values{}
	query.Set("limit", "1")
	query.Set("sort", "-createdDate")
	var response struct {
		Data []struct {
			ID         string         `json:"id"`
			Attributes map[string]any `json:"attributes"`
		} `json:"data"`
	}
	if _, err := g.appleRequest(ctx, http.MethodGet, "/v1/apps/"+url.PathEscape(storeAppID)+"/appStoreVersions?"+query.Encode(), nil, &response); err != nil {
		return nil, err
	}
	if len(response.Data) == 0 {
		return nil, nil
	}
	state, _ := response.Data[0].Attributes["appStoreState"].(string)
	return &appleVersion{ID: response.Data[0].ID, State: state, Attributes: response.Data[0].Attributes}, nil
}

func (g *LiveStoreGateway) appleFirstCustomProductPageVersion(ctx context.Context, pageID string) (string, error) {
	var response struct {
		Data []struct {
			ID string `json:"id"`
		} `json:"data"`
	}
	path := "/v1/appCustomProductPages/" + url.PathEscape(pageID) + "/appCustomProductPageVersions?limit=1"
	if _, err := g.appleRequest(ctx, http.MethodGet, path, nil, &response); err != nil {
		return "", err
	}
	if len(response.Data) == 0 {
		return "", nil
	}
	return response.Data[0].ID, nil
}

func (g *LiveStoreGateway) appleFindCustomProductPageLocalization(ctx context.Context, versionID string, locale string) (*appleLocalization, error) {
	localizations, err := g.appleCustomProductPageLocalizations(ctx, versionID)
	if err != nil {
		return nil, err
	}
	return findLocalization(localizations, locale), nil
}

func (g *LiveStoreGateway) appleCustomProductPageLocalizations(ctx context.Context, versionID string) ([]appleLocalization, error) {
	var response struct {
		Data []struct {
			ID         string         `json:"id"`
			Attributes map[string]any `json:"attributes"`
		} `json:"data"`
	}
	path := "/v1/appCustomProductPageVersions/" + url.PathEscape(versionID) + "/appCustomProductPageLocalizations?limit=200"
	if _, err := g.appleRequest(ctx, http.MethodGet, path, nil, &response); err != nil {
		return nil, err
	}
	return appleLocalizationsFromResponse(response.Data), nil
}

func (g *LiveStoreGateway) appleCreateCustomProductPageLocalization(ctx context.Context, versionID string, locale string, promotionalText string) (*appleLocalization, error) {
	attributes := map[string]string{"locale": locale}
	if strings.TrimSpace(promotionalText) != "" {
		attributes["promotionalText"] = promotionalText
	}
	body := map[string]any{
		"data": map[string]any{
			"type":       "appCustomProductPageLocalizations",
			"attributes": attributes,
			"relationships": map[string]any{
				"appCustomProductPageVersion": map[string]any{
					"data": map[string]string{"type": "appCustomProductPageVersions", "id": versionID},
				},
			},
		},
	}
	var response struct {
		Data struct {
			ID         string         `json:"id"`
			Attributes map[string]any `json:"attributes"`
		} `json:"data"`
	}
	if _, err := g.appleRequest(ctx, http.MethodPost, "/v1/appCustomProductPageLocalizations", body, &response); err != nil {
		return nil, err
	}
	localization := appleLocalizationsFromResponse([]struct {
		ID         string         `json:"id"`
		Attributes map[string]any `json:"attributes"`
	}{response.Data})
	if len(localization) == 0 {
		return nil, errors.New("App Store Connect 没有返回自定义产品页面语言")
	}
	return &localization[0], nil
}

func (g *LiveStoreGateway) applePatchCustomProductPageLocalization(ctx context.Context, localizationID string, attributes map[string]string) error {
	if len(cleanStringMap(attributes)) == 0 {
		return nil
	}
	return g.applePatchLocalization(ctx, "appCustomProductPageLocalizations", localizationID, attributes)
}

func (g *LiveStoreGateway) appleFindInfoLocalization(ctx context.Context, storeAppID string, locale string) (*appleLocalization, error) {
	localizations, err := g.appleInfoLocalizations(ctx, storeAppID)
	if err != nil {
		return nil, err
	}
	return findLocalization(localizations, locale), nil
}

func (g *LiveStoreGateway) applePatchVersionLocalization(ctx context.Context, localizationID string, attributes map[string]string) error {
	return g.applePatchLocalization(ctx, "appStoreVersionLocalizations", localizationID, attributes)
}

func (g *LiveStoreGateway) applePatchInfoLocalization(ctx context.Context, localizationID string, attributes map[string]string) error {
	return g.applePatchLocalization(ctx, "appInfoLocalizations", localizationID, attributes)
}

func (g *LiveStoreGateway) applePatchLocalization(ctx context.Context, resourceType string, localizationID string, attributes map[string]string) error {
	cleaned := map[string]string{}
	for key, value := range attributes {
		if strings.TrimSpace(value) != "" {
			cleaned[key] = value
		}
	}
	if len(cleaned) == 0 {
		return nil
	}
	body := map[string]any{
		"data": map[string]any{
			"type":       resourceType,
			"id":         localizationID,
			"attributes": cleaned,
		},
	}
	_, err := g.appleRequest(ctx, http.MethodPatch, "/v1/"+resourceType+"/"+url.PathEscape(localizationID), body, nil)
	return err
}

func (g *LiveStoreGateway) appleSyncStoreImages(ctx context.Context, ownerRelationship string, ownerID string, storeImages *StoreImages) error {
	if storeImages == nil {
		return nil
	}
	slots := map[string]StoreImageSlot{
		"phone_screenshots":  storeImages.PhoneScreenshots,
		"tablet_screenshots": storeImages.TabletScreenshots,
	}
	for slotName, slot := range slots {
		sources := storeImageSources(slot)
		if len(sources) == 0 {
			continue
		}
		displayType := appleScreenshotDisplayType(slotName)
		if displayType == "" {
			continue
		}
		set, err := g.appleFindOrCreateScreenshotSet(ctx, ownerRelationship, ownerID, displayType)
		if err != nil {
			return err
		}
		for _, screenshotID := range set.ScreenshotIDs {
			if err := g.appleDeleteScreenshot(ctx, screenshotID); err != nil {
				return err
			}
		}
		for _, source := range sources {
			image, err := g.downloadStoreImage(ctx, source)
			if err != nil {
				return err
			}
			if err := g.appleUploadScreenshot(ctx, set.ID, image); err != nil {
				return err
			}
		}
	}
	return nil
}

func (g *LiveStoreGateway) appleFindOrCreateScreenshotSet(ctx context.Context, ownerRelationship string, ownerID string, displayType string) (*appleScreenshotSet, error) {
	sets, err := g.appleScreenshotSets(ctx, ownerRelationship, ownerID)
	if err != nil {
		return nil, err
	}
	for i := range sets {
		if strings.EqualFold(sets[i].DisplayType, displayType) {
			return &sets[i], nil
		}
	}
	ownerType := appleScreenshotOwnerType(ownerRelationship)
	if ownerType == "" {
		return nil, fmt.Errorf("不支持的 Apple 截图归属关系: %s", ownerRelationship)
	}
	body := map[string]any{
		"data": map[string]any{
			"type": "appScreenshotSets",
			"attributes": map[string]string{
				"screenshotDisplayType": displayType,
			},
			"relationships": map[string]any{
				ownerRelationship: map[string]any{
					"data": map[string]string{"type": ownerType, "id": ownerID},
				},
			},
		},
	}
	var response struct {
		Data struct {
			ID string `json:"id"`
		} `json:"data"`
	}
	if _, err := g.appleRequest(ctx, http.MethodPost, "/v1/appScreenshotSets", body, &response); err != nil {
		return nil, err
	}
	if response.Data.ID == "" {
		return nil, errors.New("App Store Connect 没有返回截图集合 ID")
	}
	return &appleScreenshotSet{ID: response.Data.ID, DisplayType: displayType}, nil
}

func (g *LiveStoreGateway) appleScreenshotSets(ctx context.Context, ownerRelationship string, ownerID string) ([]appleScreenshotSet, error) {
	ownerType := appleScreenshotOwnerType(ownerRelationship)
	if ownerType == "" {
		return nil, fmt.Errorf("不支持的 Apple 截图归属关系: %s", ownerRelationship)
	}
	query := url.Values{}
	query.Set("include", "appScreenshots")
	query.Set("limit", "200")
	query.Set("limit[appScreenshots]", "200")
	path := "/v1/" + ownerType + "/" + url.PathEscape(ownerID) + "/appScreenshotSets?" + query.Encode()
	var response struct {
		Data []struct {
			ID            string         `json:"id"`
			Attributes    map[string]any `json:"attributes"`
			Relationships map[string]struct {
				Data []struct {
					ID string `json:"id"`
				} `json:"data"`
			} `json:"relationships"`
		} `json:"data"`
	}
	if _, err := g.appleRequest(ctx, http.MethodGet, path, nil, &response); err != nil {
		return nil, err
	}
	sets := make([]appleScreenshotSet, 0, len(response.Data))
	for _, item := range response.Data {
		set := appleScreenshotSet{
			ID:          item.ID,
			DisplayType: stringAttribute(item.Attributes, "screenshotDisplayType"),
		}
		if relationship, ok := item.Relationships["appScreenshots"]; ok {
			for _, linkage := range relationship.Data {
				if strings.TrimSpace(linkage.ID) != "" {
					set.ScreenshotIDs = append(set.ScreenshotIDs, linkage.ID)
				}
			}
		}
		sets = append(sets, set)
	}
	return sets, nil
}

func (g *LiveStoreGateway) appleDeleteScreenshot(ctx context.Context, screenshotID string) error {
	if strings.TrimSpace(screenshotID) == "" {
		return nil
	}
	_, err := g.appleRequest(ctx, http.MethodDelete, "/v1/appScreenshots/"+url.PathEscape(screenshotID), nil, nil)
	return err
}

func (g *LiveStoreGateway) appleUploadScreenshot(ctx context.Context, screenshotSetID string, image downloadedStoreImage) error {
	checksum := md5.Sum(image.Body)
	attributes := map[string]any{
		"fileName":           image.FileName,
		"fileSize":           len(image.Body),
		"sourceFileChecksum": hex.EncodeToString(checksum[:]),
	}
	body := map[string]any{
		"data": map[string]any{
			"type":       "appScreenshots",
			"attributes": attributes,
			"relationships": map[string]any{
				"appScreenshotSet": map[string]any{
					"data": map[string]string{"type": "appScreenshotSets", "id": screenshotSetID},
				},
			},
		},
	}
	var response struct {
		Data struct {
			ID         string `json:"id"`
			Attributes struct {
				UploadOperations []appleUploadOperation `json:"uploadOperations"`
			} `json:"attributes"`
		} `json:"data"`
	}
	if _, err := g.appleRequest(ctx, http.MethodPost, "/v1/appScreenshots", body, &response); err != nil {
		return err
	}
	if response.Data.ID == "" {
		return errors.New("App Store Connect 没有返回截图 ID")
	}
	for _, operation := range response.Data.Attributes.UploadOperations {
		if err := g.appleRunUploadOperation(ctx, operation, image.Body); err != nil {
			return err
		}
	}
	commit := map[string]any{
		"data": map[string]any{
			"type": "appScreenshots",
			"id":   response.Data.ID,
			"attributes": map[string]bool{
				"uploaded": true,
			},
		},
	}
	_, err := g.appleRequest(ctx, http.MethodPatch, "/v1/appScreenshots/"+url.PathEscape(response.Data.ID), commit, nil)
	return err
}

func (g *LiveStoreGateway) appleRunUploadOperation(ctx context.Context, operation appleUploadOperation, body []byte) error {
	if strings.TrimSpace(operation.URL) == "" {
		return errors.New("App Store Connect 上传操作缺少 URL")
	}
	start := operation.Offset
	end := int64(len(body))
	if operation.Length > 0 {
		end = start + operation.Length
	}
	if start < 0 || end < start || end > int64(len(body)) {
		return fmt.Errorf("App Store Connect 返回了非法上传分片: offset=%d length=%d size=%d", operation.Offset, operation.Length, len(body))
	}
	headers := http.Header{}
	for _, header := range operation.RequestHeaders {
		if strings.TrimSpace(header.Name) != "" {
			headers.Set(header.Name, header.Value)
		}
	}
	method := defaultString(operation.Method, http.MethodPut)
	return doRaw(ctx, g.httpClient, method, operation.URL, "", body[start:end], "", headers, nil, nil)
}

func (g *LiveStoreGateway) appleRequest(ctx context.Context, method string, path string, body any, out any) (http.Header, error) {
	token, err := g.credential.AppleJWT(time.Now())
	if err != nil {
		return nil, err
	}
	return doJSON(ctx, g.httpClient, method, g.settings.AppleAPIBaseURL+path, "Bearer "+token, body, out, func(headers http.Header) {
		g.ratePolicy.RecordAppleRateLimitHeader(headers.Get("X-Rate-Limit"))
	})
}

func (g *LiveStoreGateway) googlePreflight(ctx context.Context, payload PreflightRequest) (PreflightResponse, error) {
	packageName := defaultString(payload.App.PackageName, payload.App.BundleIdentifier)
	if strings.TrimSpace(packageName) == "" {
		return PreflightResponse{
			CanSync:    false,
			ReasonCode: stringPtr("package_name_missing"),
			Message:    "Android App 缺少 package，暂不能同步。",
			StoreState: map[string]any{"editable": false},
		}, nil
	}
	editID, err := g.googleInsertEdit(ctx, packageName)
	if err != nil {
		return PreflightResponse{}, err
	}
	return PreflightResponse{
		CanSync:    true,
		ReasonCode: nil,
		Message:    fmt.Sprintf("Google Play 已允许创建编辑会话 %s，当前状态允许修改%s。", editID, operationLabel(payload.Operation)),
		StoreState: map[string]any{
			"versionExists": true,
			"editable":      true,
			"currentStatus": "editable",
			"editId":        editID,
		},
	}, nil
}

func (g *LiveStoreGateway) googleSupportedLocales(ctx context.Context, packageName string) (SupportedLocalesResponse, error) {
	if strings.TrimSpace(packageName) == "" {
		return SupportedLocalesResponse{Locales: []string{}}, nil
	}
	editID, err := g.googleInsertEdit(ctx, packageName)
	if err != nil {
		return SupportedLocalesResponse{}, err
	}
	defer g.googleDeleteEdit(ctx, packageName, editID)
	var response struct {
		Listings []struct {
			Language string `json:"language"`
		} `json:"listings"`
	}
	if err := g.googleRequest(ctx, http.MethodGet, "/androidpublisher/v3/applications/"+url.PathEscape(packageName)+"/edits/"+url.PathEscape(editID)+"/listings", nil, &response); err != nil {
		return SupportedLocalesResponse{}, err
	}
	locales := make([]string, 0, len(response.Listings))
	for _, listing := range response.Listings {
		if strings.TrimSpace(listing.Language) != "" {
			locales = append(locales, listing.Language)
		}
	}
	return SupportedLocalesResponse{Locales: locales}, nil
}

func (g *LiveStoreGateway) googleStoreListings(ctx context.Context, packageName string) (StoreListingsResponse, error) {
	if strings.TrimSpace(packageName) == "" {
		return StoreListingsResponse{Listings: []StoreListing{}}, nil
	}
	editID, err := g.googleInsertEdit(ctx, packageName)
	if err != nil {
		return StoreListingsResponse{}, err
	}
	defer g.googleDeleteEdit(ctx, packageName, editID)
	var response struct {
		Listings []struct {
			Language         string `json:"language"`
			Title            string `json:"title"`
			FullDescription  string `json:"fullDescription"`
			ShortDescription string `json:"shortDescription"`
			Video            string `json:"video"`
		} `json:"listings"`
	}
	path := "/androidpublisher/v3/applications/" + url.PathEscape(packageName) +
		"/edits/" + url.PathEscape(editID) + "/listings"
	if err := g.googleRequest(ctx, http.MethodGet, path, nil, &response); err != nil {
		return StoreListingsResponse{}, err
	}
	listings := make([]StoreListing, 0, len(response.Listings))
	for _, item := range response.Listings {
		if strings.TrimSpace(item.Language) == "" {
			continue
		}
		listings = append(listings, StoreListing{
			Locale:           item.Language,
			Title:            item.Title,
			ShortDescription: item.ShortDescription,
			FullDescription:  item.FullDescription,
			Description:      item.FullDescription,
			VideoURL:         item.Video,
		})
	}
	sort.SliceStable(listings, func(i int, j int) bool {
		return listings[i].Locale < listings[j].Locale
	})
	return StoreListingsResponse{Listings: listings}, nil
}

func (g *LiveStoreGateway) googleStoreImages(ctx context.Context, packageName string) (StoreImagesResponse, error) {
	if strings.TrimSpace(packageName) == "" {
		return StoreImagesResponse{Locales: []StoreImageLocale{}}, nil
	}
	editID, err := g.googleInsertEdit(ctx, packageName)
	if err != nil {
		return StoreImagesResponse{}, err
	}
	defer g.googleDeleteEdit(ctx, packageName, editID)
	locales, err := g.googleListingLanguages(ctx, packageName, editID)
	if err != nil {
		return StoreImagesResponse{}, err
	}
	rows := make([]StoreImageLocale, 0, len(locales))
	for _, locale := range locales {
		images := map[string][]StoreImageRef{}
		for _, imageType := range googleImageTypes() {
			items, err := g.googleImagesForType(ctx, packageName, editID, locale, imageType)
			if err != nil {
				return StoreImagesResponse{}, err
			}
			images[googleImageSlot(imageType)] = items
		}
		rows = append(rows, StoreImageLocale{
			Locale: locale,
			Images: images,
		})
	}
	return StoreImagesResponse{Locales: rows}, nil
}

func (g *LiveStoreGateway) googleListingLanguages(ctx context.Context, packageName string, editID string) ([]string, error) {
	var response struct {
		Listings []struct {
			Language string `json:"language"`
		} `json:"listings"`
	}
	path := "/androidpublisher/v3/applications/" + url.PathEscape(packageName) +
		"/edits/" + url.PathEscape(editID) + "/listings"
	if err := g.googleRequest(ctx, http.MethodGet, path, nil, &response); err != nil {
		return nil, err
	}
	locales := make([]string, 0, len(response.Listings))
	for _, item := range response.Listings {
		if strings.TrimSpace(item.Language) != "" {
			locales = append(locales, item.Language)
		}
	}
	sort.Strings(locales)
	return locales, nil
}

type googleTrack struct {
	Track    string          `json:"track"`
	Releases []googleRelease `json:"releases"`
}

type googleRelease struct {
	Name         string             `json:"name,omitempty"`
	Status       string             `json:"status,omitempty"`
	VersionCodes []string           `json:"versionCodes,omitempty"`
	ReleaseNotes []StoreReleaseNote `json:"releaseNotes,omitempty"`
}

func (g *LiveStoreGateway) googleStoreReleases(ctx context.Context, packageName string) (StoreReleasesResponse, error) {
	if strings.TrimSpace(packageName) == "" {
		return StoreReleasesResponse{Releases: []StoreRelease{}}, nil
	}
	editID, err := g.googleInsertEdit(ctx, packageName)
	if err != nil {
		return StoreReleasesResponse{}, err
	}
	defer g.googleDeleteEdit(ctx, packageName, editID)
	tracks, err := g.googleTracks(ctx, packageName, editID)
	if err != nil {
		return StoreReleasesResponse{}, err
	}
	releases := make([]StoreRelease, 0)
	for _, track := range tracks {
		for _, release := range track.Releases {
			if len(release.VersionCodes) == 0 {
				continue
			}
			releases = append(releases, StoreRelease{
				Track:        track.Track,
				Name:         release.Name,
				Status:       release.Status,
				VersionCodes: append([]string{}, release.VersionCodes...),
				ReleaseNotes: append([]StoreReleaseNote{}, release.ReleaseNotes...),
			})
		}
	}
	sort.SliceStable(releases, func(i int, j int) bool {
		left := maxVersionCode(releases[i].VersionCodes)
		right := maxVersionCode(releases[j].VersionCodes)
		if left == right {
			return releases[i].Track < releases[j].Track
		}
		return left > right
	})
	return StoreReleasesResponse{Releases: releases}, nil
}

type googleReviewTimestamp struct {
	Seconds string `json:"seconds"`
	Nanos   int    `json:"nanos"`
}

type googleUserComment struct {
	Text             string                `json:"text"`
	LastModified     googleReviewTimestamp `json:"lastModified"`
	StarRating       int                   `json:"starRating"`
	ReviewerLanguage string                `json:"reviewerLanguage"`
	Device           string                `json:"device"`
	AppVersionCode   int64                 `json:"appVersionCode"`
	AppVersionName   string                `json:"appVersionName"`
	OriginalText     string                `json:"originalText"`
}

func (g *LiveStoreGateway) googleStoreReviews(ctx context.Context, packageName string, query StoreReviewsQuery) (StoreReviewsResponse, error) {
	if strings.TrimSpace(packageName) == "" {
		return StoreReviewsResponse{Reviews: []StoreReview{}}, nil
	}
	values := url.Values{}
	values.Set("maxResults", strconv.Itoa(clampStoreReviewPageSize(query.PageSize, 100)))
	if token := strings.TrimSpace(query.PageToken); token != "" {
		values.Set("token", token)
	}
	if startIndex := strings.TrimSpace(query.StartIndex); startIndex != "" {
		values.Set("startIndex", startIndex)
	}
	if language := strings.TrimSpace(query.TranslationLanguage); language != "" {
		values.Set("translationLanguage", language)
	}
	var response struct {
		Reviews []struct {
			ReviewID   string `json:"reviewId"`
			AuthorName string `json:"authorName"`
			Comments   []struct {
				UserComment *googleUserComment `json:"userComment"`
			} `json:"comments"`
		} `json:"reviews"`
		TokenPagination struct {
			NextPageToken string `json:"nextPageToken"`
		} `json:"tokenPagination"`
	}
	path := "/androidpublisher/v3/applications/" + url.PathEscape(packageName) + "/reviews?" + values.Encode()
	if err := g.googleRequest(ctx, http.MethodGet, path, nil, &response); err != nil {
		return StoreReviewsResponse{}, err
	}
	reviews := make([]StoreReview, 0, len(response.Reviews))
	for _, item := range response.Reviews {
		comment := latestGoogleUserComment(item.Comments)
		if comment == nil {
			continue
		}
		updatedAt := googleReviewTime(comment.LastModified)
		reviews = append(reviews, StoreReview{
			ID:             item.ReviewID,
			Platform:       "android",
			Rating:         comment.StarRating,
			Body:           comment.Text,
			AuthorName:     item.AuthorName,
			Locale:         comment.ReviewerLanguage,
			AppVersion:     comment.AppVersionName,
			AppVersionCode: strconv.FormatInt(comment.AppVersionCode, 10),
			CreatedAt:      updatedAt,
			UpdatedAt:      updatedAt,
			OriginalText:   comment.OriginalText,
		})
	}
	return StoreReviewsResponse{
		Reviews:       reviews,
		NextPageToken: response.TokenPagination.NextPageToken,
	}, nil
}

func (g *LiveStoreGateway) googleTracks(ctx context.Context, packageName string, editID string) ([]googleTrack, error) {
	var response struct {
		Tracks []googleTrack `json:"tracks"`
	}
	path := "/androidpublisher/v3/applications/" + url.PathEscape(packageName) +
		"/edits/" + url.PathEscape(editID) + "/tracks"
	if err := g.googleRequest(ctx, http.MethodGet, path, nil, &response); err != nil {
		return nil, err
	}
	return response.Tracks, nil
}

func (g *LiveStoreGateway) googleImagesForType(ctx context.Context, packageName string, editID string, locale string, imageType string) ([]StoreImageRef, error) {
	var response struct {
		Images []struct {
			ID     string `json:"id"`
			URL    string `json:"url"`
			SHA1   string `json:"sha1"`
			SHA256 string `json:"sha256"`
		} `json:"images"`
	}
	path := "/androidpublisher/v3/applications/" + url.PathEscape(packageName) +
		"/edits/" + url.PathEscape(editID) +
		"/listings/" + url.PathEscape(locale) +
		"/" + url.PathEscape(imageType)
	if err := g.googleRequest(ctx, http.MethodGet, path, nil, &response); err != nil {
		return nil, err
	}
	images := make([]StoreImageRef, 0, len(response.Images))
	for _, item := range response.Images {
		images = append(images, StoreImageRef{
			ID:     item.ID,
			URL:    item.URL,
			SHA1:   item.SHA1,
			SHA256: item.SHA256,
		})
	}
	return images, nil
}

func (g *LiveStoreGateway) googleSyncRun(ctx context.Context, payload SyncRunRequest) (SyncRunResponse, error) {
	if payload.Operation == "update_marketing_page" {
		return failedSync("google_marketing_page_unsupported", "营销页面控制台当前仅支持 App Store Connect。"), nil
	}
	if payload.Operation == "update_release_notes" {
		return g.googleSyncReleaseNotes(ctx, payload)
	}
	if payload.Operation != "update_app_metadata" {
		return failedSync("unsupported_operation", "Google Play 暂不支持这个同步操作。"), nil
	}
	if payload.Metadata == nil {
		return failedSync("empty_metadata", "商店元数据不能为空。"), nil
	}
	packageName := defaultString(payload.App.PackageName, payload.App.BundleIdentifier)
	editID, err := g.googleInsertEdit(ctx, packageName)
	if err != nil {
		return SyncRunResponse{}, err
	}
	committed := false
	if body := googleListingAttributes(payload.Metadata, payload.SyncScopes); len(body) > 0 {
		path := "/androidpublisher/v3/applications/" + url.PathEscape(packageName) +
			"/edits/" + url.PathEscape(editID) + "/listings/" + url.PathEscape(payload.Locale)
		if err := g.googleRequest(ctx, http.MethodPatch, path, body, nil); err != nil {
			return SyncRunResponse{}, err
		}
		committed = true
	}
	if shouldSyncStoreImages(payload.SyncScopes) && payload.Metadata.StoreImages != nil {
		if err := g.googleSyncStoreImages(ctx, packageName, editID, payload.Locale, payload.Metadata.StoreImages); err != nil {
			return SyncRunResponse{}, err
		}
		committed = true
	}
	if !committed {
		return failedSync("empty_sync_payload", "没有可同步的 Google Play 商店内容。"), nil
	}
	if err := g.googleCommitEdit(ctx, packageName, editID); err != nil {
		return SyncRunResponse{}, err
	}
	return SyncRunResponse{Status: "succeeded", Message: "商店元数据已同步。"}, nil
}

func (g *LiveStoreGateway) googleSyncReleaseNotes(ctx context.Context, payload SyncRunRequest) (SyncRunResponse, error) {
	if strings.TrimSpace(payload.ReleaseNotes) == "" {
		return failedSync("empty_release_notes", "版本说明不能为空。"), nil
	}
	if strings.TrimSpace(payload.Locale) == "" {
		return failedSync("locale_missing", "版本说明需要指定语言。"), nil
	}
	packageName := defaultString(payload.App.PackageName, payload.App.BundleIdentifier)
	editID, err := g.googleInsertEdit(ctx, packageName)
	if err != nil {
		return SyncRunResponse{}, err
	}
	committed := false
	defer func() {
		if !committed {
			g.googleDeleteEdit(context.Background(), packageName, editID)
		}
	}()
	tracks, err := g.googleTracks(ctx, packageName, editID)
	if err != nil {
		return SyncRunResponse{}, err
	}
	target, err := googleResolveReleaseTarget(tracks, payload.StoreRelease)
	if err != nil {
		return failedSync("google_release_target_missing", err.Error()), nil
	}
	trackIndex, releaseIndex := googleFindRelease(tracks, target.Track, target.VersionCode)
	if trackIndex < 0 || releaseIndex < 0 {
		return failedSync("google_release_target_missing", "Google Play 没有找到目标版本。"), nil
	}
	release := &tracks[trackIndex].Releases[releaseIndex]
	release.ReleaseNotes = upsertReleaseNote(release.ReleaseNotes, payload.Locale, payload.ReleaseNotes)
	path := "/androidpublisher/v3/applications/" + url.PathEscape(packageName) +
		"/edits/" + url.PathEscape(editID) + "/tracks/" + url.PathEscape(tracks[trackIndex].Track)
	if err := g.googleRequest(ctx, http.MethodPut, path, tracks[trackIndex], nil); err != nil {
		return SyncRunResponse{}, err
	}
	if err := g.googleCommitEdit(ctx, packageName, editID); err != nil {
		return SyncRunResponse{}, err
	}
	committed = true
	return SyncRunResponse{
		Status:  "succeeded",
		Message: "版本说明已同步。",
		StoreState: map[string]any{
			"track":       target.Track,
			"versionCode": target.VersionCode,
			"versionName": target.VersionName,
		},
	}, nil
}

func googleResolveReleaseTarget(tracks []googleTrack, explicit *StoreReleaseTarget) (StoreReleaseTarget, error) {
	requestedTrack := ""
	requestedVersionCode := ""
	if explicit != nil {
		requestedTrack = strings.TrimSpace(explicit.Track)
		requestedVersionCode = strings.TrimSpace(explicit.VersionCode)
	}
	var best StoreReleaseTarget
	bestValue := int64(-1)
	for _, track := range tracks {
		if requestedTrack != "" && track.Track != requestedTrack {
			continue
		}
		for _, release := range track.Releases {
			for _, versionCode := range release.VersionCodes {
				versionCode = strings.TrimSpace(versionCode)
				if versionCode == "" {
					continue
				}
				if requestedVersionCode != "" && versionCode != requestedVersionCode {
					continue
				}
				value, ok := parseVersionCode(versionCode)
				if requestedVersionCode != "" {
					return StoreReleaseTarget{Track: track.Track, VersionCode: versionCode, VersionName: release.Name}, nil
				}
				if !ok {
					continue
				}
				if value > bestValue {
					bestValue = value
					best = StoreReleaseTarget{Track: track.Track, VersionCode: versionCode, VersionName: release.Name}
				}
			}
		}
	}
	if best.VersionCode != "" {
		return best, nil
	}
	switch {
	case requestedTrack != "" && requestedVersionCode != "":
		return StoreReleaseTarget{}, fmt.Errorf("Google Play 没有找到 track=%s versionCode=%s 的 release", requestedTrack, requestedVersionCode)
	case requestedTrack != "":
		return StoreReleaseTarget{}, fmt.Errorf("Google Play 没有找到 track=%s 下可同步的 release", requestedTrack)
	case requestedVersionCode != "":
		return StoreReleaseTarget{}, fmt.Errorf("Google Play 没有找到 versionCode=%s 的 release", requestedVersionCode)
	default:
		return StoreReleaseTarget{}, errors.New("Google Play 没有可同步的 release")
	}
}

func googleFindRelease(tracks []googleTrack, targetTrack string, targetVersionCode string) (int, int) {
	for trackIndex, track := range tracks {
		if track.Track != targetTrack {
			continue
		}
		for releaseIndex, release := range track.Releases {
			for _, versionCode := range release.VersionCodes {
				if strings.TrimSpace(versionCode) == targetVersionCode {
					return trackIndex, releaseIndex
				}
			}
		}
	}
	return -1, -1
}

func upsertReleaseNote(notes []StoreReleaseNote, locale string, text string) []StoreReleaseNote {
	locale = strings.TrimSpace(locale)
	for index := range notes {
		if strings.EqualFold(notes[index].Language, locale) {
			notes[index].Language = locale
			notes[index].Text = text
			return notes
		}
	}
	return append(notes, StoreReleaseNote{Language: locale, Text: text})
}

func maxVersionCode(versionCodes []string) int64 {
	maxValue := int64(-1)
	for _, versionCode := range versionCodes {
		value, ok := parseVersionCode(versionCode)
		if ok && value > maxValue {
			maxValue = value
		}
	}
	return maxValue
}

func parseVersionCode(versionCode string) (int64, bool) {
	value, err := strconv.ParseInt(strings.TrimSpace(versionCode), 10, 64)
	if err != nil {
		return 0, false
	}
	return value, true
}

func (g *LiveStoreGateway) googleInsertEdit(ctx context.Context, packageName string) (string, error) {
	if strings.TrimSpace(packageName) == "" {
		return "", errors.New("Android package 不能为空")
	}
	var response struct {
		ID string `json:"id"`
	}
	if err := g.googleRequest(ctx, http.MethodPost, "/androidpublisher/v3/applications/"+url.PathEscape(packageName)+"/edits", map[string]any{}, &response); err != nil {
		return "", err
	}
	if response.ID == "" {
		return "", errors.New("Google Play 没有返回 edit id")
	}
	return response.ID, nil
}

func (g *LiveStoreGateway) googleCommitEdit(ctx context.Context, packageName string, editID string) error {
	return g.googleRequest(ctx, http.MethodPost, googleCommitEditPath(packageName, editID), nil, nil)
}

func googleCommitEditPath(packageName string, editID string) string {
	values := url.Values{}
	values.Set("changesInReviewBehavior", "ERROR_IF_IN_REVIEW")
	values.Set("changesNotSentForReview", "true")
	return "/androidpublisher/v3/applications/" + url.PathEscape(packageName) +
		"/edits/" + url.PathEscape(editID) + ":commit?" + values.Encode()
}

func (g *LiveStoreGateway) googleDeleteEdit(ctx context.Context, packageName string, editID string) {
	if strings.TrimSpace(packageName) == "" || strings.TrimSpace(editID) == "" {
		return
	}
	_ = g.googleRequest(
		ctx,
		http.MethodDelete,
		"/androidpublisher/v3/applications/"+url.PathEscape(packageName)+"/edits/"+url.PathEscape(editID),
		nil,
		nil,
	)
}

func (g *LiveStoreGateway) googleSyncStoreImages(ctx context.Context, packageName string, editID string, locale string, storeImages *StoreImages) error {
	if storeImages == nil {
		return nil
	}
	slots := map[string]StoreImageSlot{
		"featureGraphic":       storeImages.FeatureGraphicURL,
		"phoneScreenshots":     storeImages.PhoneScreenshots,
		"sevenInchScreenshots": storeImages.TabletScreenshots,
		"tenInchScreenshots":   storeImages.TabletScreenshots,
	}
	for imageType, slot := range slots {
		sources := storeImageSources(slot)
		if len(sources) == 0 {
			continue
		}
		if err := g.googleDeleteImagesForType(ctx, packageName, editID, locale, imageType); err != nil {
			return err
		}
		for _, source := range sources {
			image, err := g.downloadStoreImage(ctx, source)
			if err != nil {
				return err
			}
			if err := g.googleUploadImage(ctx, packageName, editID, locale, imageType, image); err != nil {
				return err
			}
		}
	}
	return nil
}

func (g *LiveStoreGateway) googleDeleteImagesForType(ctx context.Context, packageName string, editID string, locale string, imageType string) error {
	path := "/androidpublisher/v3/applications/" + url.PathEscape(packageName) +
		"/edits/" + url.PathEscape(editID) +
		"/listings/" + url.PathEscape(locale) +
		"/" + url.PathEscape(imageType)
	return g.googleRequest(ctx, http.MethodDelete, path, nil, nil)
}

func (g *LiveStoreGateway) googleUploadImage(ctx context.Context, packageName string, editID string, locale string, imageType string, image downloadedStoreImage) error {
	token, err := g.credential.GoogleAccessToken(ctx, g.httpClient, time.Now())
	if err != nil {
		return err
	}
	apiBase := strings.TrimRight(g.settings.GoogleAPIBaseURL, "/")
	path := "/upload/androidpublisher/v3/applications/" + url.PathEscape(packageName) +
		"/edits/" + url.PathEscape(editID) +
		"/listings/" + url.PathEscape(locale) +
		"/" + url.PathEscape(imageType) + "?uploadType=media"
	return doRaw(ctx, g.httpClient, http.MethodPost, apiBase+path, "Bearer "+token, image.Body, image.ContentType, nil, nil, nil)
}

func (g *LiveStoreGateway) googleRequest(ctx context.Context, method string, path string, body any, out any) error {
	token, err := g.credential.GoogleAccessToken(ctx, g.httpClient, time.Now())
	if err != nil {
		return err
	}
	_, err = doJSON(ctx, g.httpClient, method, g.settings.GoogleAPIBaseURL+path, "Bearer "+token, body, out, nil)
	return err
}

func (g *LiveStoreGateway) downloadStoreImage(ctx context.Context, source StoreImageAsset) (downloadedStoreImage, error) {
	rawURL := strings.TrimSpace(source.DownloadURL)
	if rawURL == "" {
		rawURL = strings.TrimSpace(source.StorageKey)
	}
	if rawURL == "" {
		return downloadedStoreImage{}, errors.New("商店图缺少可下载地址")
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, rawURL, nil)
	if err != nil {
		return downloadedStoreImage{}, err
	}
	response, err := g.httpClient.Do(request)
	if err != nil {
		return downloadedStoreImage{}, err
	}
	defer response.Body.Close()
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return downloadedStoreImage{}, fmt.Errorf("商店图下载失败 HTTP %d", response.StatusCode)
	}
	body, err := io.ReadAll(io.LimitReader(response.Body, 64<<20))
	if err != nil {
		return downloadedStoreImage{}, err
	}
	if len(body) == 0 {
		return downloadedStoreImage{}, errors.New("商店图下载内容为空")
	}
	fileName := strings.TrimSpace(source.FileName)
	if fileName == "" {
		if parsed, parseErr := url.Parse(rawURL); parseErr == nil {
			fileName = path.Base(parsed.Path)
		}
	}
	if fileName == "" || fileName == "." || fileName == "/" {
		fileName = "store-image.png"
	}
	contentType := strings.TrimSpace(source.ContentType)
	if contentType == "" {
		contentType = response.Header.Get("Content-Type")
	}
	if semicolon := strings.Index(contentType, ";"); semicolon >= 0 {
		contentType = strings.TrimSpace(contentType[:semicolon])
	}
	if contentType == "" {
		contentType = http.DetectContentType(body)
	}
	if extension := strings.ToLower(path.Ext(fileName)); extension == "" {
		if extensions, _ := mime.ExtensionsByType(contentType); len(extensions) > 0 {
			fileName += extensions[0]
		}
	}
	return downloadedStoreImage{
		FileName:    fileName,
		ContentType: contentType,
		Body:        body,
	}, nil
}

func doJSON(ctx context.Context, client *http.Client, method string, url string, authorization string, body any, out any, after func(http.Header)) (http.Header, error) {
	var requestBody io.Reader
	if body != nil {
		raw, err := json.Marshal(body)
		if err != nil {
			return nil, err
		}
		requestBody = bytes.NewReader(raw)
	}
	request, err := http.NewRequestWithContext(ctx, method, url, requestBody)
	if err != nil {
		return nil, err
	}
	request.Header.Set("Accept", "application/json")
	if body != nil {
		request.Header.Set("Content-Type", "application/json")
	}
	if authorization != "" {
		request.Header.Set("Authorization", authorization)
	}
	response, err := client.Do(request)
	if err != nil {
		return nil, err
	}
	defer response.Body.Close()
	if after != nil {
		after(response.Header)
	}
	responseBody, _ := io.ReadAll(io.LimitReader(response.Body, 2<<20))
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return response.Header, fmt.Errorf("商店接口返回 HTTP %d: %s", response.StatusCode, strings.TrimSpace(string(responseBody)))
	}
	if out != nil && len(responseBody) > 0 {
		if err := json.Unmarshal(responseBody, out); err != nil {
			return response.Header, fmt.Errorf("商店接口响应解析失败: %w", err)
		}
	}
	return response.Header, nil
}

func doRaw(ctx context.Context, client *http.Client, method string, url string, authorization string, body []byte, contentType string, headers http.Header, out any, after func(http.Header)) error {
	request, err := http.NewRequestWithContext(ctx, method, url, bytes.NewReader(body))
	if err != nil {
		return err
	}
	request.Header.Set("Accept", "application/json")
	if contentType != "" {
		request.Header.Set("Content-Type", contentType)
	}
	for key, values := range headers {
		for _, value := range values {
			request.Header.Add(key, value)
		}
	}
	if authorization != "" {
		request.Header.Set("Authorization", authorization)
	}
	response, err := client.Do(request)
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if after != nil {
		after(response.Header)
	}
	responseBody, _ := io.ReadAll(io.LimitReader(response.Body, 2<<20))
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return fmt.Errorf("商店接口返回 HTTP %d: %s", response.StatusCode, strings.TrimSpace(string(responseBody)))
	}
	if out != nil && len(responseBody) > 0 {
		if err := json.Unmarshal(responseBody, out); err != nil {
			return fmt.Errorf("商店接口响应解析失败: %w", err)
		}
	}
	return nil
}

func appleLocalizationsFromResponse(data []struct {
	ID         string         `json:"id"`
	Attributes map[string]any `json:"attributes"`
}) []appleLocalization {
	localizations := make([]appleLocalization, 0, len(data))
	for _, item := range data {
		locale, _ := item.Attributes["locale"].(string)
		localizations = append(localizations, appleLocalization{
			ID:         item.ID,
			Locale:     locale,
			Attributes: item.Attributes,
		})
	}
	return localizations
}

func localizationLocales(localizations []appleLocalization) []string {
	locales := make([]string, 0, len(localizations))
	for _, localization := range localizations {
		if strings.TrimSpace(localization.Locale) != "" {
			locales = append(locales, localization.Locale)
		}
	}
	return locales
}

func findLocalization(localizations []appleLocalization, locale string) *appleLocalization {
	for i := range localizations {
		if strings.EqualFold(localizations[i].Locale, locale) {
			return &localizations[i]
		}
	}
	return nil
}

func appleProductPageOptimizationFromResource(item appleResource) ProductPageOptimization {
	return ProductPageOptimization{
		ID:                item.ID,
		Name:              item.Attributes.Name,
		Platform:          item.Attributes.Platform,
		State:             item.Attributes.State,
		TrafficProportion: item.Attributes.TrafficProportion,
		ReviewRequired:    item.Attributes.ReviewRequired,
		StartDate:         item.Attributes.StartDate,
		EndDate:           item.Attributes.EndDate,
		Treatments:        []ProductPageOptimizationTreatment{},
	}
}

func stringAttribute(attributes map[string]any, key string) string {
	value, _ := attributes[key].(string)
	return strings.TrimSpace(value)
}

func sortedStoreListings(listingsByLocale map[string]StoreListing) []StoreListing {
	listings := make([]StoreListing, 0, len(listingsByLocale))
	for _, listing := range listingsByLocale {
		if strings.TrimSpace(listing.Locale) != "" {
			listings = append(listings, listing)
		}
	}
	sort.SliceStable(listings, func(i int, j int) bool {
		return listings[i].Locale < listings[j].Locale
	})
	return listings
}

func appleImageRefFromAttributes(id string, attributes map[string]any) StoreImageRef {
	image := StoreImageRef{
		ID:       id,
		FileName: stringAttribute(attributes, "fileName"),
		URL:      stringAttribute(attributes, "url"),
	}
	if image.URL == "" {
		if asset, ok := attributes["imageAsset"].(map[string]any); ok {
			image.URL = stringAttribute(asset, "templateUrl")
			image.Width = intAttribute(asset, "width")
			image.Height = intAttribute(asset, "height")
		}
	}
	return image
}

func intAttribute(attributes map[string]any, key string) int {
	switch value := attributes[key].(type) {
	case int:
		return value
	case int64:
		return int(value)
	case float64:
		return int(value)
	default:
		return 0
	}
}

func appleScreenshotSlot(displayType string) string {
	normalized := strings.ToUpper(strings.TrimSpace(displayType))
	switch {
	case strings.Contains(normalized, "IPHONE"):
		return "phone_screenshots"
	case strings.Contains(normalized, "IPAD"):
		return "tablet_screenshots"
	default:
		return ""
	}
}

func appleScreenshotDisplayType(slot string) string {
	switch slot {
	case "phone_screenshots":
		return "APP_IPHONE_67"
	case "tablet_screenshots":
		return "APP_IPAD_PRO_3GEN_129"
	default:
		return ""
	}
}

func appleScreenshotOwnerType(ownerRelationship string) string {
	switch ownerRelationship {
	case "appStoreVersionLocalization":
		return "appStoreVersionLocalizations"
	case "appCustomProductPageLocalization":
		return "appCustomProductPageLocalizations"
	default:
		return ""
	}
}

func googleImageTypes() []string {
	return []string{
		"featureGraphic",
		"phoneScreenshots",
		"sevenInchScreenshots",
		"tenInchScreenshots",
		"tvScreenshots",
		"wearScreenshots",
	}
}

func googleImageSlot(imageType string) string {
	switch imageType {
	case "featureGraphic":
		return "feature_graphic_url"
	case "phoneScreenshots":
		return "phone_screenshots"
	case "sevenInchScreenshots":
		return "seven_inch_screenshots"
	case "tenInchScreenshots":
		return "ten_inch_screenshots"
	case "tvScreenshots":
		return "tv_screenshots"
	case "wearScreenshots":
		return "wear_screenshots"
	default:
		return imageType
	}
}

func clampStoreReviewPageSize(value int, maxValue int) int {
	if value <= 0 {
		return 50
	}
	if value > maxValue {
		return maxValue
	}
	return value
}

func appleNextPageToken(rawNext string) string {
	next := strings.TrimSpace(rawNext)
	if next == "" {
		return ""
	}
	parsed, err := url.Parse(next)
	if err != nil {
		return next
	}
	if cursor := parsed.Query().Get("cursor"); cursor != "" {
		return cursor
	}
	return next
}

func latestGoogleUserComment(comments []struct {
	UserComment *googleUserComment `json:"userComment"`
}) *googleUserComment {
	var selected *googleUserComment
	for index := range comments {
		if comments[index].UserComment != nil {
			selected = comments[index].UserComment
		}
	}
	return selected
}

func googleReviewTime(value googleReviewTimestamp) string {
	seconds, err := strconv.ParseInt(strings.TrimSpace(value.Seconds), 10, 64)
	if err != nil || seconds <= 0 {
		return ""
	}
	return time.Unix(seconds, int64(value.Nanos)).UTC().Format(time.RFC3339Nano)
}

func storeImageSources(slot StoreImageSlot) []StoreImageAsset {
	sources := make([]StoreImageAsset, 0, len(slot.Assets)+len(slot.URLs))
	for _, asset := range slot.Assets {
		if strings.TrimSpace(asset.DownloadURL) != "" || strings.TrimSpace(asset.StorageKey) != "" {
			sources = append(sources, asset)
		}
	}
	for _, rawURL := range slot.URLs {
		trimmed := strings.TrimSpace(rawURL)
		if trimmed == "" {
			continue
		}
		fileName := ""
		if parsed, err := url.Parse(trimmed); err == nil {
			fileName = path.Base(parsed.Path)
		}
		sources = append(sources, StoreImageAsset{
			FileName:    fileName,
			DownloadURL: trimmed,
		})
	}
	return sources
}

func failedSync(code string, summary string) SyncRunResponse {
	return SyncRunResponse{
		Status:       "failed",
		Message:      summary,
		ErrorCode:    stringPtr(code),
		ErrorSummary: stringPtr(summary),
	}
}

func operationLabel(operation string) string {
	if operation == "update_app_metadata" {
		return "商店元数据"
	}
	if operation == "update_marketing_page" {
		return "营销页面"
	}
	return "版本说明"
}

func defaultString(value string, fallback string) string {
	if strings.TrimSpace(value) == "" {
		return fallback
	}
	return value
}
