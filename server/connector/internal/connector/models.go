package connector

type StoreApp struct {
	AppID            string `json:"appId"`
	BundleIdentifier string `json:"bundleIdentifier"`
	StoreAppID       string `json:"storeAppId,omitempty"`
	PackageName      string `json:"packageName,omitempty"`
}

type PreflightRequest struct {
	DeveloperAccountID string   `json:"developerAccountId"`
	ConnectorID        string   `json:"connectorId,omitempty"`
	Operation          string   `json:"operation"`
	Platform           string   `json:"platform"`
	Version            string   `json:"version"`
	Locale             string   `json:"locale"`
	App                StoreApp `json:"app"`
}

type PreflightResponse struct {
	CanSync    bool           `json:"canSync"`
	ReasonCode *string        `json:"reasonCode"`
	Message    string         `json:"message"`
	StoreState map[string]any `json:"storeState"`
}

type SupportedLocalesResponse struct {
	Locales []string `json:"locales"`
}

type StoreListingsResponse struct {
	Listings []StoreListing `json:"listings"`
}

type StoreListing struct {
	Locale           string `json:"locale"`
	Title            string `json:"title,omitempty"`
	Subtitle         string `json:"subtitle,omitempty"`
	ShortDescription string `json:"shortDescription,omitempty"`
	FullDescription  string `json:"fullDescription,omitempty"`
	Description      string `json:"description,omitempty"`
	Keywords         string `json:"keywords,omitempty"`
	PromotionalText  string `json:"promotionalText,omitempty"`
	ReleaseNotes     string `json:"releaseNotes,omitempty"`
	VideoURL         string `json:"videoUrl,omitempty"`
}

type StoreImagesResponse struct {
	Locales []StoreImageLocale `json:"locales"`
}

type StoreImageLocale struct {
	Locale string                     `json:"locale"`
	Images map[string][]StoreImageRef `json:"images"`
}

type StoreImageRef struct {
	ID       string `json:"id,omitempty"`
	FileName string `json:"fileName,omitempty"`
	URL      string `json:"url,omitempty"`
	Width    int    `json:"width,omitempty"`
	Height   int    `json:"height,omitempty"`
	SHA1     string `json:"sha1,omitempty"`
	SHA256   string `json:"sha256,omitempty"`
}

type ProductPageOptimizationTreatment struct {
	ID          string   `json:"id,omitempty"`
	Name        string   `json:"name"`
	AppIconName string   `json:"appIconName,omitempty"`
	Locales     []string `json:"locales,omitempty"`
}

type ProductPageOptimization struct {
	ID                string                             `json:"id"`
	Name              string                             `json:"name"`
	Platform          string                             `json:"platform"`
	State             string                             `json:"state"`
	TrafficProportion int                                `json:"trafficProportion"`
	ReviewRequired    bool                               `json:"reviewRequired"`
	StartDate         string                             `json:"startDate,omitempty"`
	EndDate           string                             `json:"endDate,omitempty"`
	Treatments        []ProductPageOptimizationTreatment `json:"treatments"`
}

type ProductPageOptimizationsResponse struct {
	Experiments []ProductPageOptimization `json:"experiments"`
}

type ProductPageOptimizationCreateRequest struct {
	DeveloperAccountID string                             `json:"developerAccountId"`
	Platform           string                             `json:"platform"`
	App                StoreApp                           `json:"app"`
	Name               string                             `json:"name"`
	TrafficProportion  int                                `json:"trafficProportion"`
	Locales            []string                           `json:"locales"`
	Treatments         []ProductPageOptimizationTreatment `json:"treatments"`
}

type ProductPageOptimizationCreateResponse struct {
	Experiment ProductPageOptimization `json:"experiment"`
}

type StoreRelease struct {
	Track        string             `json:"track"`
	Name         string             `json:"name,omitempty"`
	Status       string             `json:"status,omitempty"`
	VersionCodes []string           `json:"versionCodes"`
	ReleaseNotes []StoreReleaseNote `json:"releaseNotes,omitempty"`
}

type StoreReleaseNote struct {
	Language string `json:"language"`
	Text     string `json:"text"`
}

type StoreReleasesResponse struct {
	Releases []StoreRelease `json:"releases"`
}

type StoreReviewsQuery struct {
	PageSize            int
	PageToken           string
	StartIndex          string
	TranslationLanguage string
	Sort                string
}

type StoreReview struct {
	ID               string         `json:"id"`
	Platform         string         `json:"platform"`
	Rating           int            `json:"rating,omitempty"`
	Title            string         `json:"title,omitempty"`
	Body             string         `json:"body,omitempty"`
	AuthorName       string         `json:"authorName,omitempty"`
	ReviewerNickname string         `json:"reviewerNickname,omitempty"`
	Locale           string         `json:"locale,omitempty"`
	Territory        string         `json:"territory,omitempty"`
	AppVersion       string         `json:"appVersion,omitempty"`
	AppVersionCode   string         `json:"appVersionCode,omitempty"`
	CreatedAt        string         `json:"createdAt,omitempty"`
	UpdatedAt        string         `json:"updatedAt,omitempty"`
	OriginalText     string         `json:"originalText,omitempty"`
	Raw              map[string]any `json:"raw,omitempty"`
}

type StoreReviewsResponse struct {
	Reviews       []StoreReview  `json:"reviews"`
	NextPageToken string         `json:"nextPageToken,omitempty"`
	RawPageInfo   map[string]any `json:"rawPageInfo,omitempty"`
}

type StoreReleaseTarget struct {
	Track       string `json:"track,omitempty"`
	VersionCode string `json:"versionCode,omitempty"`
	VersionName string `json:"versionName,omitempty"`
}

type StoreMetadata struct {
	ContentSet       *ContentSet  `json:"contentSet"`
	Title            string       `json:"title"`
	Keywords         string       `json:"keywords"`
	PromotionalText  string       `json:"promotionalText"`
	ShortDescription string       `json:"shortDescription"`
	Description      string       `json:"description"`
	FullDescription  string       `json:"fullDescription"`
	VideoURL         string       `json:"videoUrl"`
	StoreImages      *StoreImages `json:"storeImages"`
}

type MarketingPage struct {
	PageID          string       `json:"pageId"`
	PageName        string       `json:"pageName"`
	PageType        string       `json:"pageType"`
	ApplePageID     string       `json:"applePageId"`
	DeepLinkURL     string       `json:"deepLinkUrl"`
	Locale          string       `json:"locale"`
	Keywords        string       `json:"keywords"`
	PromotionalText string       `json:"promotionalText"`
	StoreImages     *StoreImages `json:"storeImages"`
}

type ContentSet struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

type StoreImages struct {
	FeatureGraphicURL StoreImageSlot `json:"feature_graphic_url"`
	PhoneScreenshots  StoreImageSlot `json:"phone_screenshots"`
	TabletScreenshots StoreImageSlot `json:"tablet_screenshots"`
}

type StoreImageSlot struct {
	URLs   []string          `json:"urls"`
	Assets []StoreImageAsset `json:"assets"`
}

type StoreImageAsset struct {
	FileName    string `json:"fileName"`
	ContentType string `json:"contentType"`
	SizeBytes   int64  `json:"sizeBytes"`
	StorageKey  string `json:"storageKey"`
	DownloadURL string `json:"downloadUrl"`
}

type SyncRunRequest struct {
	PreflightRequest
	RunID         string              `json:"runId"`
	ReleaseNotes  string              `json:"releaseNotes"`
	StoreRelease  *StoreReleaseTarget `json:"storeRelease,omitempty"`
	Metadata      *StoreMetadata      `json:"metadata"`
	MarketingPage *MarketingPage      `json:"marketingPage"`
	SyncScopes    []string            `json:"syncScopes"`
}

type SyncRunResponse struct {
	Status       string         `json:"status"`
	Message      string         `json:"message"`
	ErrorCode    *string        `json:"errorCode"`
	ErrorSummary *string        `json:"errorSummary"`
	StoreState   map[string]any `json:"storeState,omitempty"`
}

type ConnectorError struct {
	Detail string `json:"detail"`
}

type HealthResponse struct {
	Status             string                      `json:"status"`
	DeveloperAccountID string                      `json:"developerAccountId"`
	Mode               string                      `json:"mode"`
	Credentials        map[string]CredentialStatus `json:"credentials"`
}

type CredentialStatus struct {
	Configured bool   `json:"configured"`
	Valid      bool   `json:"valid"`
	Message    string `json:"message"`
}

func stringPtr(value string) *string {
	return &value
}
