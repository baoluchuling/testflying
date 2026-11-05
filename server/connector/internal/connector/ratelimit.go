package connector

import (
	"math"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"
)

type RateLimitRule struct {
	Key         string
	MaxRequests int
	Window      time.Duration
}

type RateLimitDecision struct {
	Allowed    bool
	RetryAfter time.Duration
	Rule       RateLimitRule
}

type SlidingWindowRateLimiter struct {
	mu     sync.Mutex
	events map[string][]time.Time
	now    func() time.Time
}

func NewSlidingWindowRateLimiter() *SlidingWindowRateLimiter {
	return &SlidingWindowRateLimiter{
		events: make(map[string][]time.Time),
		now:    time.Now,
	}
}

func (l *SlidingWindowRateLimiter) Check(rule RateLimitRule) RateLimitDecision {
	maxRequests := rule.MaxRequests
	if maxRequests < 1 {
		maxRequests = 1
	}
	window := rule.Window
	if window <= 0 {
		window = time.Second
	}

	l.mu.Lock()
	defer l.mu.Unlock()

	now := l.now()
	cutoff := now.Add(-window)
	events := l.events[rule.Key]
	kept := events[:0]
	for _, event := range events {
		if event.After(cutoff) {
			kept = append(kept, event)
		}
	}
	events = kept

	if len(events) >= maxRequests {
		retryAfter := events[0].Add(window).Sub(now)
		if retryAfter < time.Second {
			retryAfter = time.Second
		}
		l.events[rule.Key] = events
		return RateLimitDecision{
			Allowed:    false,
			RetryAfter: retryAfter,
			Rule: RateLimitRule{
				Key:         rule.Key,
				MaxRequests: maxRequests,
				Window:      window,
			},
		}
	}

	events = append(events, now)
	l.events[rule.Key] = events
	return RateLimitDecision{
		Allowed: true,
		Rule: RateLimitRule{
			Key:         rule.Key,
			MaxRequests: maxRequests,
			Window:      window,
		},
	}
}

func (l *SlidingWindowRateLimiter) Reset() {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.events = make(map[string][]time.Time)
}

type StoreRateLimitPolicy struct {
	settings         Settings
	mu               sync.Mutex
	appleMaxRequests int
	appleWindow      time.Duration
}

func NewStoreRateLimitPolicy(settings Settings) *StoreRateLimitPolicy {
	return &StoreRateLimitPolicy{
		settings:         settings,
		appleMaxRequests: maxInt(settings.AppleRateLimitFallbackMax, 1),
		appleWindow:      maxDuration(settings.AppleRateLimitWindow, time.Second),
	}
}

func (p *StoreRateLimitPolicy) RuleForPlatform(platform string) RateLimitRule {
	switch normalizePlatform(platform) {
	case "ios":
		p.mu.Lock()
		defer p.mu.Unlock()
		return RateLimitRule{
			Key:         "apple",
			MaxRequests: p.appleMaxRequests,
			Window:      p.appleWindow,
		}
	case "android":
		return RateLimitRule{
			Key:         "google",
			MaxRequests: p.settings.GoogleRateLimitMaxRequests,
			Window:      p.settings.GoogleRateLimitWindow,
		}
	default:
		return RateLimitRule{
			Key:         "unknown-store",
			MaxRequests: minInt(p.settings.GoogleRateLimitMaxRequests, p.appleMaxRequests),
			Window:      maxDuration(p.settings.GoogleRateLimitWindow, p.appleWindow),
		}
	}
}

func (p *StoreRateLimitPolicy) RecordAppleRateLimitHeader(headerValue string) {
	limit, ok := ParseAppleUserHourLimit(headerValue)
	if !ok {
		return
	}
	adjusted := int(math.Floor(float64(limit) * p.settings.AppleRateLimitSafetyRatio))
	if adjusted < 1 {
		adjusted = 1
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	p.appleMaxRequests = adjusted
	p.appleWindow = time.Hour
}

var appleUserHourLimitPattern = regexp.MustCompile(`(?:^|;)\s*user-hour-lim\s*:\s*(\d+)\s*(?:;|$)`)

func ParseAppleUserHourLimit(headerValue string) (int, bool) {
	match := appleUserHourLimitPattern.FindStringSubmatch(headerValue)
	if len(match) != 2 {
		return 0, false
	}
	value, err := strconv.Atoi(match[1])
	if err != nil {
		return 0, false
	}
	return value, true
}

func normalizePlatform(platform string) string {
	switch strings.ToLower(strings.TrimSpace(platform)) {
	case "ios":
		return "ios"
	case "android":
		return "android"
	default:
		return ""
	}
}

func minInt(a int, b int) int {
	if a < b {
		return a
	}
	return b
}

func maxInt(a int, b int) int {
	if a > b {
		return a
	}
	return b
}

func maxDuration(a time.Duration, b time.Duration) time.Duration {
	if a > b {
		return a
	}
	return b
}
