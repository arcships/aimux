package aimux

import (
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"
)

// HandleID is a uint64 in C and a decimal string in JSON.
type HandleID uint64

func (id HandleID) MarshalJSON() ([]byte, error) {
	return json.Marshal(strconv.FormatUint(uint64(id), 10))
}

func (id *HandleID) UnmarshalJSON(data []byte) error {
	var s string
	if err := json.Unmarshal(data, &s); err != nil {
		return fmt.Errorf("aimux: handle must be a decimal string: %w", err)
	}
	n, err := decimalUint64(s)
	if err != nil {
		return err
	}
	*id = HandleID(n)
	return nil
}

func decimalUint64(s string) (uint64, error) {
	if s == "" || (len(s) > 1 && s[0] == '0') {
		return 0, fmt.Errorf("aimux: invalid decimal uint64 %q", s)
	}
	for _, c := range s {
		if c < '0' || c > '9' {
			return 0, fmt.Errorf("aimux: invalid decimal uint64 %q", s)
		}
	}
	n, err := strconv.ParseUint(s, 10, 64)
	if err != nil {
		return 0, fmt.Errorf("aimux: invalid decimal uint64: %w", err)
	}
	return n, nil
}

// Params keeps absent keys distinct from explicit JSON null. Use json.RawMessage
// for open providerOptions and other canonical JSON values.
type Params map[string]json.RawMessage

func validateJSON(name string, data []byte) error {
	if !utf8.Valid(data) || !json.Valid(data) {
		return localError("binding", 202, "InvalidWireJson", name+": invalid UTF-8 JSON")
	}
	if err := checkSurrogates(name, string(data)); err != nil {
		return localError("binding", 202, "InvalidWireJson", err.Error())
	}
	return nil
}

func encodeParams(op string, params any) ([]byte, error) {
	if op == "" || !utf8.ValidString(op) || strings.ContainsRune(op, 0) {
		return nil, localError("protocol", 300, "InvalidOp", "op must be nonempty NUL-free UTF-8")
	}
	data, err := json.Marshal(params)
	if err != nil {
		return nil, localError("binding", 202, "InvalidWireJson", err.Error())
	}
	if err := validateJSON("params", data); err != nil {
		return nil, err
	}
	return data, nil
}

// checkSurrogates rejects the \uXXXX escapes serde_json rejects but
// encoding/json accepts: an unpaired surrogate half (json.Valid passes it,
// then serde fails in C with a C ABI failure code). Runs after json.Valid, so
// every backslash here is a well-formed escape inside a string.
func checkSurrogates(param, s string) error {
	for i := 0; i+1 < len(s); {
		if s[i] != '\\' {
			i++
			continue
		}
		if s[i+1] != 'u' {
			i += 2 // \" \\ \/ \b \f \n \r \t — skip the escaped byte
			continue
		}
		hi, ok := hex4(s, i)
		if !ok {
			i += 2
			continue
		}
		i += 6
		switch {
		case hi >= 0xDC00 && hi <= 0xDFFF:
			return fmt.Errorf("aimux: %s: invalid JSON: lone low surrogate \\u%04X", param, hi)
		case hi >= 0xD800 && hi <= 0xDBFF:
			lo, ok := hex4(s, i)
			if !ok || lo < 0xDC00 || lo > 0xDFFF {
				return fmt.Errorf("aimux: %s: invalid JSON: unpaired high surrogate \\u%04X", param, hi)
			}
			i += 6
		}
	}
	return nil
}

// hex4 reads the code point of a \uXXXX escape starting at s[i]; false when
// s[i:] is not such an escape.
func hex4(s string, i int) (int, bool) {
	if i+6 > len(s) || s[i] != '\\' || s[i+1] != 'u' {
		return 0, false
	}
	n, err := strconv.ParseUint(s[i+2:i+6], 16, 32)
	if err != nil {
		return 0, false
	}
	return int(n), true
}

// WireDTO retains canonical JSON alongside a typed view. In particular,
// optional pointer fields in T do not erase absent versus explicit null,
// unknown fields, or large JSON integers when the DTO is encoded again.
// Use CallInto[WireDTO[ResultType]] for a lossless typed result.
// WireDTO is immutable; decode Value to work on a separate typed value.
type WireDTO[T any] struct{ raw json.RawMessage }

func (d *WireDTO[T]) UnmarshalJSON(data []byte) error {
	if err := validateJSON("DTO", data); err != nil {
		return err
	}
	var value T
	if err := json.Unmarshal(data, &value); err != nil {
		return err
	}
	d.raw = append(json.RawMessage(nil), data...)
	return nil
}

func (d WireDTO[T]) MarshalJSON() ([]byte, error) {
	if len(d.raw) == 0 {
		return nil, fmt.Errorf("aimux: uninitialized wire DTO")
	}
	return append([]byte(nil), d.raw...), nil
}

func (d WireDTO[T]) Value() (T, error) {
	var value T
	err := json.Unmarshal(d.raw, &value)
	return value, err
}

// Field returns an owned copy and a presence bit: nil,false means absent;
// []byte("null"),true means explicitly null.
func (d WireDTO[T]) Field(name string) (json.RawMessage, bool) {
	var fields map[string]json.RawMessage
	if json.Unmarshal(d.raw, &fields) != nil {
		return nil, false
	}
	raw, present := fields[name]
	return raw, present
}
