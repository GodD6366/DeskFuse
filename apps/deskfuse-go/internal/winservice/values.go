package winservice

import (
	"encoding/json"
	"math"
	"strconv"
)

// The MyGo binding may deliver numbers as float64 or json.Number depending on
// whether the caller went through encoding/json with UseNumber. Both are
// accepted; a value that cannot be represented exactly is rejected instead of
// being silently rounded.

func stringValue(values map[string]any, names ...string) (string, bool) {
	for _, name := range names {
		if value, ok := values[name].(string); ok && value != "" {
			return value, true
		}
	}
	return "", false
}

func boolValue(values map[string]any, names ...string) (bool, bool) {
	for _, name := range names {
		if value, ok := values[name].(bool); ok {
			return value, true
		}
	}
	return false, false
}

func floatValue(values map[string]any, names ...string) (float64, bool) {
	for _, name := range names {
		switch value := values[name].(type) {
		case float64:
			return value, true
		case json.Number:
			parsed, err := value.Float64()
			if err == nil {
				return parsed, true
			}
		case int:
			return float64(value), true
		case int64:
			return float64(value), true
		case uint64:
			return float64(value), true
		}
	}
	return 0, false
}

func uintValue(values map[string]any, names ...string) (uint64, bool) {
	for _, name := range names {
		switch value := values[name].(type) {
		case float64:
			// 1<<64 converts exactly to float64; <= math.MaxUint64 would round
			// the bound up to 2^64 and admit a value that cannot become uint64.
			if value >= 0 && value < 1<<64 && value == math.Trunc(value) {
				return uint64(value), true
			}
		case json.Number:
			parsed, err := strconv.ParseUint(value.String(), 10, 64)
			if err == nil {
				return parsed, true
			}
		case string:
			parsed, err := strconv.ParseUint(value, 10, 64)
			if err == nil {
				return parsed, true
			}
		case int:
			if value >= 0 {
				return uint64(value), true
			}
		case int64:
			if value >= 0 {
				return uint64(value), true
			}
		case uint64:
			return value, true
		}
	}
	return 0, false
}

func uintArg(values map[string]any, name, message string) (uint64, error) {
	value, ok := uintValue(values, name)
	if !ok {
		return 0, invalidParams(message)
	}
	return value, nil
}

func requiredString(values map[string]any, names []string, message string) (string, error) {
	value, ok := stringValue(values, names...)
	if !ok {
		return "", invalidParams(message)
	}
	return value, nil
}

func finiteFloatArg(values map[string]any, names []string, message string) (float64, error) {
	value, ok := floatValue(values, names...)
	if !ok || math.IsNaN(value) || math.IsInf(value, 0) {
		return 0, invalidParams(message)
	}
	return value, nil
}

// numberString parses a decimal identifier that may arrive as a JSON number.
func numberString(value any) (string, bool) {
	switch typed := value.(type) {
	case string:
		if _, err := strconv.ParseUint(typed, 10, 64); err == nil {
			return typed, true
		}
	case json.Number:
		if _, err := strconv.ParseUint(typed.String(), 10, 64); err == nil {
			return typed.String(), true
		}
	case float64:
		if typed >= 0 && typed < 1<<64 && typed == math.Trunc(typed) {
			return strconv.FormatUint(uint64(typed), 10), true
		}
	}
	return "", false
}

func nonNilStrings(values []string) []string {
	if values == nil {
		return []string{}
	}
	return values
}

func nonNilRaw(values []json.RawMessage) []json.RawMessage {
	if values == nil {
		return []json.RawMessage{}
	}
	return values
}

// sliceValues accepts the generic slice encoding/json produces and the typed
// slices a direct Go caller may pass.
func sliceValues(value any) ([]any, bool) {
	switch values := value.(type) {
	case []any:
		return values, true
	case []map[string]any:
		rows := make([]any, len(values))
		for index := range values {
			rows[index] = values[index]
		}
		return rows, true
	case []string:
		rows := make([]any, len(values))
		for index := range values {
			rows[index] = values[index]
		}
		return rows, true
	default:
		return nil, false
	}
}
