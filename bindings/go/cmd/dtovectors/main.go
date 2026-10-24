package main

import (
	"encoding/json"
	"fmt"
	aimux "github.com/arcships/aimux/bindings/go"
	"os"
	"path/filepath"
)

func main() {
	directory := "contract-tests/dto"
	if len(os.Args) > 1 {
		directory = os.Args[1]
	}
	data, err := os.ReadFile(filepath.Join(directory, "index.json"))
	if err != nil {
		panic(err)
	}
	var index []struct {
		Type string `json:"type"`
		File string `json:"file"`
	}
	if err := json.Unmarshal(data, &index); err != nil {
		panic(err)
	}
	encoder := json.NewEncoder(os.Stdout)
	for _, entry := range index {
		record := map[string]any{"file": entry.File, "ok": false}
		value, err := aimux.NewDTO(entry.Type)
		if err == nil {
			var raw []byte
			raw, err = os.ReadFile(filepath.Join(directory, entry.File))
			if err == nil {
				err = json.Unmarshal(raw, value)
			}
		}
		if err == nil {
			var raw []byte
			raw, err = json.Marshal(value)
			if err == nil {
				record["ok"] = true
				record["value"] = json.RawMessage(raw)
			}
		}
		if err != nil {
			record["error"] = fmt.Sprint(err)
		}
		if err := encoder.Encode(record); err != nil {
			panic(err)
		}
	}
}
