package aimux

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"
)

// Params carries caller-defined operation parameters.
type Params map[string]json.RawMessage

func validateJSON(name string, data []byte) error {
	if !json.Valid(data) {
		return localError("binding", 202, "InvalidWireJson", name+": invalid UTF-8 JSON")
	}
	return validateJSONText(name, data)
}

func validateJSONText(name string, data []byte) error {
	if !utf8.Valid(data) {
		return localError("binding", 202, "InvalidWireJson", name+": invalid UTF-8 JSON")
	}
	if err := checkSurrogates(name, data); err != nil {
		return localError("binding", 202, "InvalidWireJson", err.Error())
	}
	return nil
}

// Unmarshal requires a complete JSON document, including the end of input.
// encoding/json accepts invalid UTF-8 and surrogate halves, which the wire rejects.
func decodeJSON(name string, data []byte, dst any) error {
	if err := json.Unmarshal(data, dst); err != nil {
		return err
	}
	return validateJSONText(name, data)
}

func encodeParams(op string, params any) ([]byte, error) {
	if op == "" || !utf8.ValidString(op) || strings.ContainsRune(op, 0) {
		return nil, localError("protocol", 300, "InvalidOp", "op must be nonempty NUL-free UTF-8")
	}
	data, err := json.Marshal(params)
	if err != nil {
		return nil, localError("binding", 202, "InvalidWireJson", err.Error())
	}
	if err := validateJSONText("params", data); err != nil {
		return nil, err
	}
	return data, nil
}

// checkSurrogates rejects the \uXXXX escapes serde_json rejects but
// encoding/json accepts: an unpaired surrogate half (json.Valid passes it,
// then serde fails in C with a C ABI failure code). The caller has already
// checked JSON syntax through Marshal or decoding.
func checkSurrogates(param string, s []byte) error {
	for i := 0; i < len(s); {
		next := bytes.IndexByte(s[i:], '\\')
		if next == -1 {
			break
		}
		i += next
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
func hex4(s []byte, i int) (int, bool) {
	if i+6 > len(s) || s[i] != '\\' || s[i+1] != 'u' {
		return 0, false
	}
	n, err := strconv.ParseUint(string(s[i+2:i+6]), 16, 32)
	if err != nil {
		return 0, false
	}
	return int(n), true
}
