package runner

import (
	"regexp"
)

var (
	privateKeyPattern        = regexp.MustCompile(`(?is)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----`)
	secretAssignmentPattern  = regexp.MustCompile(`(?i)\b(password|token|secret|api[_-]?key|private[_-]?key)\b\s*[:=]\s*([^\r\n\s]+)`)
	bearerTokenPattern       = regexp.MustCompile(`(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{12,}`)
	githubTokenPattern       = regexp.MustCompile(`\b(?:ghp_|gho_|github_pat_|glpat-|sk-)[A-Za-z0-9_=-]{8,}`)
)

func RedactText(value string) string {
	value = privateKeyPattern.ReplaceAllString(value, "[REDACTED]")
	value = secretAssignmentPattern.ReplaceAllString(value, "$1=[REDACTED]")
	value = bearerTokenPattern.ReplaceAllString(value, "Bearer [REDACTED]")
	value = githubTokenPattern.ReplaceAllString(value, "[REDACTED]")
	return value
}

func RedactValue(value interface{}) interface{} {
	switch typed := value.(type) {
	case string:
		return RedactText(typed)
	case map[string]interface{}:
		return RedactMap(typed)
	case []interface{}:
		items := make([]interface{}, 0, len(typed))
		for _, item := range typed {
			items = append(items, RedactValue(item))
		}
		return items
	case AgentReport:
		typed.Summary = RedactText(typed.Summary)
		typed.HumanAction = RedactText(typed.HumanAction)
		return typed
	default:
		return value
	}
}

func RedactMap(value map[string]interface{}) map[string]interface{} {
	redacted := make(map[string]interface{}, len(value))
	for key, item := range value {
		redacted[key] = RedactValue(item)
	}
	return redacted
}
