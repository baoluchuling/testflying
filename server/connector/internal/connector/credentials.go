package connector

import (
	"bytes"
	"context"
	"crypto"
	"crypto/ecdsa"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"strings"
	"sync"
	"time"
)

type CredentialManager struct {
	settings Settings
	google   *GoogleServiceAccount

	mu                 sync.Mutex
	googleAccessToken  string
	googleTokenExpires time.Time
}

func NewCredentialManager(settings Settings) *CredentialManager {
	return &CredentialManager{settings: settings}
}

func (m *CredentialManager) Status() map[string]CredentialStatus {
	return map[string]CredentialStatus{
		"apple":  m.AppleStatus(),
		"google": m.GoogleStatus(),
	}
}

func (m *CredentialManager) HasAnyLiveCredential() bool {
	apple := m.AppleStatus()
	google := m.GoogleStatus()
	return apple.Valid || google.Valid
}

func (m *CredentialManager) AppleStatus() CredentialStatus {
	if m.settings.AppleIssuerID == "" && m.settings.AppleKeyID == "" &&
		m.settings.ApplePrivateKeyPath == "" && m.settings.ApplePrivateKeyPEM == "" {
		return CredentialStatus{Configured: false, Valid: false, Message: "未配置 Apple App Store Connect API Key"}
	}
	if _, err := m.applePrivateKey(); err != nil {
		return CredentialStatus{Configured: true, Valid: false, Message: err.Error()}
	}
	if m.settings.AppleIssuerID == "" || m.settings.AppleKeyID == "" {
		return CredentialStatus{Configured: true, Valid: false, Message: "Apple Issuer ID 和 Key ID 必填"}
	}
	return CredentialStatus{Configured: true, Valid: true, Message: "Apple 凭据格式可用"}
}

func (m *CredentialManager) GoogleStatus() CredentialStatus {
	if m.settings.GoogleServiceAccountJSONPath == "" && m.settings.GoogleServiceAccountJSON == "" &&
		m.settings.GoogleClientEmail == "" && m.settings.GooglePrivateKey == "" {
		return CredentialStatus{Configured: false, Valid: false, Message: "未配置 Google service account 凭据"}
	}
	account, err := m.googleServiceAccount()
	if err != nil {
		return CredentialStatus{Configured: true, Valid: false, Message: err.Error()}
	}
	if account.ClientEmail == "" || account.PrivateKey == "" {
		return CredentialStatus{Configured: true, Valid: false, Message: "Google service account 凭据缺少 client_email 或 private_key"}
	}
	if _, err := account.privateKey(); err != nil {
		return CredentialStatus{Configured: true, Valid: false, Message: err.Error()}
	}
	return CredentialStatus{Configured: true, Valid: true, Message: "Google service account 格式可用"}
}

func (m *CredentialManager) AppleJWT(now time.Time) (string, error) {
	privateKey, err := m.applePrivateKey()
	if err != nil {
		return "", err
	}
	if m.settings.AppleIssuerID == "" || m.settings.AppleKeyID == "" {
		return "", errors.New("Apple Issuer ID 和 Key ID 必填")
	}
	header := map[string]string{
		"alg": "ES256",
		"kid": m.settings.AppleKeyID,
		"typ": "JWT",
	}
	claims := map[string]any{
		"iss": m.settings.AppleIssuerID,
		"iat": now.Unix(),
		"exp": now.Add(20 * time.Minute).Unix(),
		"aud": "appstoreconnect-v1",
	}
	return signECDSAJWT(header, claims, privateKey)
}

func (m *CredentialManager) GoogleAccessToken(ctx context.Context, client *http.Client, now time.Time) (string, error) {
	m.mu.Lock()
	if m.googleAccessToken != "" && now.Before(m.googleTokenExpires.Add(-time.Minute)) {
		token := m.googleAccessToken
		m.mu.Unlock()
		return token, nil
	}
	m.mu.Unlock()

	account, err := m.googleServiceAccount()
	if err != nil {
		return "", err
	}
	assertion, err := account.JWTAssertion(now, m.settings.GoogleTokenURL)
	if err != nil {
		return "", err
	}
	form := url.Values{}
	form.Set("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer")
	form.Set("assertion", assertion)
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, m.settings.GoogleTokenURL, strings.NewReader(form.Encode()))
	if err != nil {
		return "", err
	}
	request.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	response, err := client.Do(request)
	if err != nil {
		return "", fmt.Errorf("Google token 请求失败: %w", err)
	}
	defer response.Body.Close()
	body, _ := io.ReadAll(io.LimitReader(response.Body, 1<<20))
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return "", fmt.Errorf("Google token 返回 HTTP %d: %s", response.StatusCode, strings.TrimSpace(string(body)))
	}
	var payload struct {
		AccessToken string `json:"access_token"`
		ExpiresIn   int64  `json:"expires_in"`
	}
	if err := json.Unmarshal(body, &payload); err != nil {
		return "", fmt.Errorf("Google token 响应解析失败: %w", err)
	}
	if payload.AccessToken == "" {
		return "", errors.New("Google token 响应缺少 access_token")
	}
	if payload.ExpiresIn <= 0 {
		payload.ExpiresIn = 3600
	}
	m.mu.Lock()
	defer m.mu.Unlock()
	m.googleAccessToken = payload.AccessToken
	m.googleTokenExpires = now.Add(time.Duration(payload.ExpiresIn) * time.Second)
	return payload.AccessToken, nil
}

func (m *CredentialManager) applePrivateKey() (*ecdsa.PrivateKey, error) {
	pemBytes, err := readSecret(m.settings.ApplePrivateKeyPEM, m.settings.ApplePrivateKeyPath)
	if err != nil {
		return nil, fmt.Errorf("读取 Apple .p8 失败: %w", err)
	}
	block, _ := pem.Decode(pemBytes)
	if block == nil {
		return nil, errors.New("Apple .p8 不是 PEM 格式")
	}
	key, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return nil, fmt.Errorf("解析 Apple .p8 失败: %w", err)
	}
	privateKey, ok := key.(*ecdsa.PrivateKey)
	if !ok || privateKey.Curve != elliptic.P256() {
		return nil, errors.New("Apple .p8 必须是 P-256 ECDSA 私钥")
	}
	return privateKey, nil
}

func (m *CredentialManager) googleServiceAccount() (*GoogleServiceAccount, error) {
	m.mu.Lock()
	if m.google != nil {
		account := m.google
		m.mu.Unlock()
		return account, nil
	}
	m.mu.Unlock()

	var account GoogleServiceAccount
	if m.settings.GoogleServiceAccountJSONPath != "" || m.settings.GoogleServiceAccountJSON != "" {
		raw, err := readSecret(m.settings.GoogleServiceAccountJSON, m.settings.GoogleServiceAccountJSONPath)
		if err != nil {
			return nil, fmt.Errorf("读取 Google service account JSON 失败: %w", err)
		}
		if err := json.Unmarshal(raw, &account); err != nil {
			return nil, fmt.Errorf("解析 Google service account JSON 失败: %w", err)
		}
	} else {
		account = GoogleServiceAccount{
			ClientEmail: strings.TrimSpace(m.settings.GoogleClientEmail),
			PrivateKey:  normalizeMultilineSecret(m.settings.GooglePrivateKey),
			TokenURI:    m.settings.GoogleTokenURL,
		}
	}
	if account.TokenURI == "" {
		account.TokenURI = m.settings.GoogleTokenURL
	}

	m.mu.Lock()
	m.google = &account
	m.mu.Unlock()
	return &account, nil
}

type GoogleServiceAccount struct {
	ClientEmail string `json:"client_email"`
	PrivateKey  string `json:"private_key"`
	TokenURI    string `json:"token_uri"`
}

func (a GoogleServiceAccount) JWTAssertion(now time.Time, tokenURI string) (string, error) {
	privateKey, err := a.privateKey()
	if err != nil {
		return "", err
	}
	if tokenURI == "" {
		tokenURI = a.TokenURI
	}
	if tokenURI == "" {
		tokenURI = "https://oauth2.googleapis.com/token"
	}
	header := map[string]string{
		"alg": "RS256",
		"typ": "JWT",
	}
	claims := map[string]any{
		"iss":   a.ClientEmail,
		"scope": "https://www.googleapis.com/auth/androidpublisher",
		"aud":   tokenURI,
		"iat":   now.Unix(),
		"exp":   now.Add(time.Hour).Unix(),
	}
	return signRSAJWT(header, claims, privateKey)
}

func (a GoogleServiceAccount) privateKey() (*rsa.PrivateKey, error) {
	block, _ := pem.Decode([]byte(a.PrivateKey))
	if block == nil {
		return nil, errors.New("Google private_key 不是 PEM 格式")
	}
	key, err := x509.ParsePKCS8PrivateKey(block.Bytes)
	if err != nil {
		return nil, fmt.Errorf("解析 Google private_key 失败: %w", err)
	}
	privateKey, ok := key.(*rsa.PrivateKey)
	if !ok {
		return nil, errors.New("Google private_key 必须是 RSA 私钥")
	}
	return privateKey, nil
}

func readSecret(inline string, path string) ([]byte, error) {
	if strings.TrimSpace(inline) != "" {
		return []byte(inline), nil
	}
	if strings.TrimSpace(path) == "" {
		return nil, errors.New("未提供内联内容或文件路径")
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	return bytes.TrimSpace(raw), nil
}

func normalizeMultilineSecret(value string) string {
	trimmed := strings.TrimSpace(value)
	if trimmed == "" {
		return ""
	}
	return strings.ReplaceAll(trimmed, `\n`, "\n")
}

func signECDSAJWT(header map[string]string, claims map[string]any, privateKey *ecdsa.PrivateKey) (string, error) {
	signingInput, err := jwtSigningInput(header, claims)
	if err != nil {
		return "", err
	}
	digest := sha256.Sum256([]byte(signingInput))
	r, s, err := ecdsa.Sign(rand.Reader, privateKey, digest[:])
	if err != nil {
		return "", err
	}
	signature := make([]byte, 64)
	r.FillBytes(signature[:32])
	s.FillBytes(signature[32:])
	return signingInput + "." + base64.RawURLEncoding.EncodeToString(signature), nil
}

func signRSAJWT(header map[string]string, claims map[string]any, privateKey *rsa.PrivateKey) (string, error) {
	signingInput, err := jwtSigningInput(header, claims)
	if err != nil {
		return "", err
	}
	digest := sha256.Sum256([]byte(signingInput))
	signature, err := rsa.SignPKCS1v15(rand.Reader, privateKey, crypto.SHA256, digest[:])
	if err != nil {
		return "", err
	}
	return signingInput + "." + base64.RawURLEncoding.EncodeToString(signature), nil
}

func jwtSigningInput(header map[string]string, claims map[string]any) (string, error) {
	headerBytes, err := json.Marshal(header)
	if err != nil {
		return "", err
	}
	claimBytes, err := json.Marshal(claims)
	if err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(headerBytes) + "." +
		base64.RawURLEncoding.EncodeToString(claimBytes), nil
}
